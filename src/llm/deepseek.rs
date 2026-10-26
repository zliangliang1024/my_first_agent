use anyhow::Result;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use core::option::Option::None;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{Duration, timeout};

use super::{ChatMessage, FunctionCall, StreamDelta, ToolCall};
use crate::config::Config;

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
    usage: Option<UsageData>, // ✅ 新增：接收 Token
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDeltaPayload,
}

#[derive(Deserialize, Default)]
struct StreamDeltaPayload {
    content: Option<String>,
    tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Deserialize)]
struct StreamToolCallDelta {
    index: usize,
    id: Option<String>,
    function: Option<StreamFunctionDelta>,
}

#[derive(Deserialize, Default)]
struct StreamFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Default)]
struct ToolCallAccum {
    id: Option<String>,
    name: String,
    arguments: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct UsageData {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

// ✅ 返回值加上 Option<UsageData>
pub async fn call_deepseek_stream<F>(
    cfg: &Config,
    messages: &[ChatMessage],
    tools: &[Value],
    cancel: Arc<AtomicBool>,
    mut on_delta: F,
) -> Result<(ChatMessage, Option<UsageData>)>
where
    F: FnMut(StreamDelta),
{
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()?;

    let body = json!({
        "model": cfg.model,
        "messages": messages,
        "tools": tools,
        "stream": true,
        "stream_options": { "include_usage": true } // ✅ 开启 usage 统计
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

    let mut content = String::new();
    let mut tool_calls: Vec<ToolCallAccum> = Vec::new();
    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut final_usage: Option<UsageData> = None; // ✅ 记录 usage

    loop {
        let next_item = timeout(Duration::from_secs(30), stream.next()).await;

        match next_item {
            Ok(Some(chunk_res)) => {
                if cancel.load(Ordering::SeqCst) {
                    anyhow::bail!("__CANCELLED__");
                }
                let bytes = chunk_res?;
                buffer.push_str(&String::from_utf8_lossy(&bytes));

                // SSE 按行分割
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

                    // ✅ 提取 usage (通常在最末尾的一帧)
                    if let Some(u) = chunk.usage {
                        final_usage = Some(u);
                    }

                    let Some(choice) = chunk.choices.into_iter().next() else {
                        continue;
                    };

                    if let Some(text) = choice.delta.content {
                        if !text.is_empty() {
                            content.push_str(&text);
                            on_delta(StreamDelta::Content(text));
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
            Ok(None) => break,                                   // 流正常结束
            Err(_) => anyhow::bail!("生成超时：30秒未收到数据"), // 这里不会再报错了
        }
    }

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

    // ✅ 返回带 usage 的元组
    Ok((
        ChatMessage {
            role: "assistant".into(),
            content: content_opt,
            tool_calls: final_tool_calls,
            tool_call_id: None,
        },
        final_usage,
    ))
}
