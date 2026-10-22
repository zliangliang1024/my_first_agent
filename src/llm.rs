use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::Config;
use tokio::time::{Duration, timeout};

// ---------------- 对话数据结构 ----------------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn user(content: &str) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn tool(tool_call_id: &str, content: &str) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub function: FunctionCall,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

// ---------------- 流式增量 ----------------

#[derive(Debug, Clone)]
pub enum StreamDelta {
    /// 文本增量
    Content(String),
}

// ---------------- SSE 解析用的临时结构 ----------------

#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDeltaPayload,
}

#[derive(Deserialize, Default)]
struct StreamDeltaPayload {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Deserialize)]
struct StreamToolCallDelta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<StreamFunctionDelta>,
}

#[derive(Deserialize, Default)]
struct StreamFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

// ---------------- 累积器 ----------------

#[derive(Default)]
struct ToolCallAccum {
    id: Option<String>,
    name: String,
    arguments: String,
}

// ---------------- 流式调用 ----------------

pub async fn call_deepseek_stream<F>(
    cfg: &Config,
    messages: &[ChatMessage],
    tools: &[Value],
    cancel: Arc<AtomicBool>, // ✅ 新增
    mut on_delta: F,
) -> Result<ChatMessage>
where
    F: FnMut(StreamDelta),
{
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10)) // ✅ 10秒连接超时
        .build()?;
    let body = json!({
        "model": cfg.model,
        "messages": messages,
        "tools": tools,
        "stream": true,
    });

    let resp = client
        .post(&cfg.deepseek_url)
        .bearer_auth(&cfg.deepseek_api_key)
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("DeepSeek 返回 {}: {}", status, text);
    }

    // 累积器
    let mut content = String::new();
    let mut tool_calls: Vec<ToolCallAccum> = Vec::new();

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();

    // 替换 while 循环，改用 loop + match 严谨处理超时和结束
    loop {
        // 1. 等待下一个数据块，最多等 30 秒
        let next_item = timeout(Duration::from_secs(30), stream.next()).await;

        match next_item {
            // 成功拿到数据块
            Ok(Some(chunk_res)) => {
                // 2. 依然检查用户是否点了取消
                if cancel.load(Ordering::SeqCst) {
                    anyhow::bail!("__CANCELLED__");
                }

                // 3. 处理数据
                let bytes = chunk_res?;
                buffer.push_str(&String::from_utf8_lossy(&bytes));

                // 4. SSE 按行分割 (原来的逻辑)
                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].trim_end_matches('\r').to_string();
                    buffer.drain(..=pos);

                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };
                    let data = data.trim();
                    if data == "[DONE]" || data.is_empty() {
                        continue;
                    }

                    let Ok(chunk) = serde_json::from_str::<StreamChunk>(data) else {
                        continue;
                    };

                    let Some(choice) = chunk.choices.into_iter().next() else {
                        continue;
                    };

                    if let Some(text) = choice.delta.content {
                        if !text.is_empty() {
                            content.push_str(&text);
                            on_delta(StreamDelta::Content(text.to_string()));
                        }
                    }

                    if let Some(tcs) = choice.delta.tool_calls {
                        for tc in tcs {
                            let idx = tc.index;
                            while tool_calls.len() <= idx {
                                tool_calls.push(ToolCallAccum::default());
                            }
                            let accum = &mut tool_calls[idx];
                            if let Some(id) = tc.id {
                                accum.id = Some(id);
                            }
                            if let Some(f) = tc.function {
                                if let Some(n) = f.name {
                                    accum.name.push_str(&n);
                                }
                                if let Some(a) = f.arguments {
                                    accum.arguments.push_str(&a);
                                }
                            }
                        }
                    }
                }
            }
            // 流正常结束
            Ok(None) => {
                break;
            }
            // 超时了
            Err(_) => {
                anyhow::bail!("生成超时：30秒未收到数据");
            }
        }
    }

    // 组装最终 ChatMessage
    let final_tool_calls = if tool_calls.is_empty() {
        None
    } else {
        Some(
            tool_calls
                .into_iter()
                .map(|a| ToolCall {
                    id: a.id.unwrap_or_default(),
                    type_: "function".into(),
                    function: FunctionCall {
                        name: a.name,
                        arguments: a.arguments,
                    },
                })
                .collect(),
        )
    };

    let content_opt = if content.is_empty() {
        None
    } else {
        Some(content)
    };

    Ok(ChatMessage {
        role: "assistant".into(),
        content: content_opt,
        tool_calls: final_tool_calls,
        tool_call_id: None,
    })
}
