use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use rmcp::{ServiceExt, transport::TokioChildProcess};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, Write};
use tokio::process::Command;

// ---------- 配置 ----------

#[derive(Debug, Deserialize)]
struct Config {
    deepseek_api_key: String,
    #[serde(default = "default_url")]
    deepseek_url: String,
    #[serde(default = "default_model")]
    model: String,
    dbx_exe: String,
}

fn default_url() -> String {
    "https://api.deepseek.com/chat/completions".into()
}
fn default_model() -> String {
    "deepseek-chat".into()
}

impl Config {
    fn load() -> Result<Self> {
        let path = std::env::var("AGENT_CONFIG").unwrap_or_else(|_| "config.json".into());
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("读取 {} 失败：{}", path, e))?;
        Ok(serde_json::from_str(&content)?)
    }
}

// ---------- 数据结构（保持不变） ----------

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ChatMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

impl ChatMessage {
    fn user(content: &str) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    fn assistant(content: Option<String>, tool_calls: Option<Vec<ToolCall>>) -> Self {
        Self {
            role: "assistant".into(),
            content,
            tool_calls,
            tool_call_id: None,
        }
    }
    fn tool(tool_call_id: &str, content: &str) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ToolCall {
    id: String,
    #[serde(rename = "type")]
    type_: String,
    function: FunctionCall,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct FunctionCall {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct DeepSeekResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChatMessage,
}

// ---------- DeepSeek ----------

async fn call_deepseek(
    cfg: &Config,
    messages: &[ChatMessage],
    tools: &[Value],
) -> Result<ChatMessage> {
    let client = reqwest::Client::new();
    let body = json!({
        "model": cfg.model,
        "messages": messages,
        "tools": tools,
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

    let data: DeepSeekResponse = resp.json().await?;
    data.choices
        .into_iter()
        .next()
        .map(|c| c.message)
        .ok_or_else(|| anyhow::anyhow!("DeepSeek 返回了空的 choices"))
}

// ---------- MCP 工具转换 ----------

fn mcp_tools_to_deepseek(tools: &[rmcp::model::Tool]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description.clone().unwrap_or_default(),
                    "parameters": t.input_schema,
                }
            })
        })
        .collect()
}

// ---------- 主流程 ----------

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::load()?;

    println!("🔌 正在连接 DBX MCP: {}", cfg.dbx_exe);
    let mcp_client = ().serve(TokioChildProcess::new(Command::new(&cfg.dbx_exe))?).await?;

    let tools_result = mcp_client.list_tools(None).await?;
    let deepseek_tools = mcp_tools_to_deepseek(&tools_result.tools);
    println!("✅ 已加载 {} 个工具\n", deepseek_tools.len());

    let mut history: Vec<ChatMessage> = Vec::new();

    loop {
        print!("你> ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        if input == "exit" || input == "quit" {
            break;
        }

        history.push(ChatMessage::user(input));

        loop {
            let assistant = call_deepseek(&cfg, &history, &deepseek_tools).await?;

            if assistant.tool_calls.is_none() {
                let text = assistant.content.clone().unwrap_or_default();
                println!("\nAgent> {}\n", text);
                history.push(assistant);
                break;
            }

            let tool_calls = assistant.tool_calls.clone().unwrap();
            history.push(assistant);

            for tc in &tool_calls {
                println!(
                    "🔧 调用工具: {} 参数: {}",
                    tc.function.name, tc.function.arguments
                );

                let args: Value = serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));

                // ✅ 用 new() + with_arguments() 构造
                let mut params = CallToolRequestParams::new(tc.function.name.clone());
                if let Some(map) = args.as_object() {
                    params = params.with_arguments(map.clone());
                }

                let result = mcp_client.call_tool(params).await;

                let result_text = match result {
                    Ok(r) => serde_json::to_string(&r).unwrap_or_default(),
                    Err(e) => format!("工具执行失败: {}", e),
                };

                println!("   结果: {}\n", truncate(&result_text, 300));
                history.push(ChatMessage::tool(&tc.id, &result_text));
            }
        }
    }

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "..."
    }
}
