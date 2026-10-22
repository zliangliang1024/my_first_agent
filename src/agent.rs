use crate::llm::{ChatMessage, StreamDelta, call_deepseek_stream};
use std::sync::mpsc::Sender;

use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use rmcp::{ServiceExt, transport::TokioChildProcess};
use serde_json::{Value, json};
use tokio::process::Command;

use crate::config::Config;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{Duration, timeout};

/// UI → Agent 命令
pub enum AgentCommand {
    SendMessage(String),
    ReloadConfig(Config), // ✅ 新增
    Shutdown,
}

/// Agent → UI 事件
pub enum AgentEvent {
    Status(String),
    Ready { tool_count: usize },
    Thinking,
    // ✅ 新增
    AssistantStart,
    AssistantDelta(String),

    AssistantReply(String),
    ToolCallStart { name: String, args: String },
    ToolCallEnd { name: String, result: String },
    Cancelled, // ✅ 新增
    Error(String),
}

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

pub async fn agent_loop(
    mut cfg: Config,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<AgentCommand>,
    event_tx: Sender<AgentEvent>,
    cancel_flag: Arc<AtomicBool>, // ✅ 新增
) {
    let _ = event_tx.send(AgentEvent::Status("正在连接 MCP...".into()));

    // ---- 连接 MCP ----

    if cfg.dbx_exe.trim().is_empty() {
        let _ = event_tx.send(AgentEvent::Error(
            "未配置 DBX 路径，请点击右上角【⚙️ 设置】填写 dbx-mcp.exe 的完整路径".into(),
        ));
        return; // 直接退出，不要启动子进程
    }

    let transport = match TokioChildProcess::new(Command::new(&cfg.dbx_exe)) {
        Ok(t) => t,
        Err(e) => {
            let _ = event_tx.send(AgentEvent::Error(format!("启动 MCP 子进程失败: {}", e)));
            return;
        }
    };
    let client = match ().serve(transport).await {
        Ok(c) => c,
        Err(e) => {
            let _ = event_tx.send(AgentEvent::Error(format!("MCP 初始化失败: {}", e)));
            return;
        }
    };

    let tools_result = match client.list_tools(None).await {
        Ok(t) => t,
        Err(e) => {
            let _ = event_tx.send(AgentEvent::Error(format!("获取工具列表失败: {}", e)));
            return;
        }
    };
    let tools = mcp_tools_to_deepseek(&tools_result.tools);
    let _ = event_tx.send(AgentEvent::Ready {
        tool_count: tools.len(),
    });

    // ---- 主循环 ----
    let mut history: Vec<ChatMessage> = Vec::new();

    // ✅ 插入 system prompt
    if !cfg.system_prompt.is_empty() {
        history.push(ChatMessage {
            role: "system".into(),
            content: Some(cfg.system_prompt.clone()),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            AgentCommand::Shutdown => break,

            AgentCommand::ReloadConfig(new_cfg) => {
                cfg = new_cfg;
                let _ = event_tx.send(AgentEvent::Status("配置已更新并生效".into()));
            }

            AgentCommand::SendMessage(text) => {
                // ✅ 双重保险：如果 Key 为空，直接报错返回
                if cfg.deepseek_api_key.trim().is_empty() {
                    let _ = event_tx.send(AgentEvent::Error(
                        "未配置 DeepSeek API Key，请在设置中填写。".into(),
                    ));
                    continue; // 跳过本次循环
                }
                // ✅ 每次新请求，先重置取消标志
                cancel_flag.store(false, Ordering::SeqCst);

                let save_point = history.len();
                history.push(ChatMessage::user(&text));

                let result: Result<()> = async {
                    loop {
                        let _ = event_tx.send(AgentEvent::Thinking);

                        // ✅ 带重试逻辑的流式调用
                        let _ = event_tx.send(AgentEvent::AssistantStart);
                        let tx_delta = event_tx.clone();
                        let cancel_for_llm = cancel_flag.clone();

                        let mut attempt = 1;
                        let max_attempts = 3;
                        let mut assistant_opt = None;

                        while attempt <= max_attempts {
                            let res = call_deepseek_stream(
                                &cfg,
                                &history,
                                &tools,
                                cancel_for_llm.clone(),
                                {
                                    let tx = tx_delta.clone();
                                    move |delta| {
                                        if let StreamDelta::Content(s) = delta {
                                            let _ = tx.send(AgentEvent::AssistantDelta(s));
                                        }
                                    }
                                },
                            )
                            .await;

                            match res {
                                Ok(msg) => {
                                    assistant_opt = Some(msg);
                                    break;
                                }
                                Err(e) => {
                                    let err_str = e.to_string();

                                    // 如果是用户主动点了“停止”，直接退出，千万别重试
                                    if err_str.contains("__CANCELLED__") {
                                        anyhow::bail!("__CANCELLED__");
                                    }

                                    // 如果已经达到最大重试次数，彻底放弃并报错
                                    if attempt == max_attempts {
                                        anyhow::bail!(
                                            "重试 {} 次后仍然失败: {}",
                                            max_attempts,
                                            err_str
                                        );
                                    }

                                    // 通知 UI 正在重试
                                    let _ = event_tx.send(AgentEvent::Status(format!(
                                        "网络请求失败，正在重试 ({}/{})...",
                                        attempt, max_attempts
                                    )));
                                    println!(
                                        "[Retry] 第 {} 次失败: {}，准备重试...",
                                        attempt, err_str
                                    );
                                    // 指数退避：第 1 次等 1 秒，第 2 次等 2 秒
                                    let sleep_secs = 2_u64.pow(attempt - 1);
                                    tokio::time::sleep(tokio::time::Duration::from_secs(
                                        sleep_secs,
                                    ))
                                    .await;
                                    attempt += 1;
                                }
                            }
                        }

                        // 拿到最终结果（因为已经加了 expect，如果没拿到说明逻辑有 bug）
                        let assistant = assistant_opt.expect("重试逻辑错误：未获取到 assistant");

                        // 无工具调用 → 最终回复
                        if assistant.tool_calls.is_none() {
                            let text = assistant.content.clone().unwrap_or_default();
                            history.push(assistant);
                            let _ = event_tx.send(AgentEvent::AssistantReply(text));
                            break;
                        }

                        // 有工具调用 → 逐个执行
                        let tool_calls = assistant.tool_calls.clone().unwrap();
                        history.push(assistant);

                        for tc in &tool_calls {
                            // ✅ 工具调用前检查取消
                            if cancel_flag.load(Ordering::SeqCst) {
                                anyhow::bail!("__CANCELLED__");
                            }

                            let _ = event_tx.send(AgentEvent::ToolCallStart {
                                name: tc.function.name.clone(),
                                args: tc.function.arguments.clone(),
                            });

                            let args: Value = serde_json::from_str(&tc.function.arguments)
                                .unwrap_or_else(|_| json!({}));

                            let mut params = CallToolRequestParams::new(tc.function.name.clone());
                            if let Some(map) = args.as_object() {
                                params = params.with_arguments(map.clone());
                            }

                            // ✅ 包裹 120 秒超时检测
                            let result_text =
                                match timeout(Duration::from_secs(120), client.call_tool(params))
                                    .await
                                {
                                    Ok(Ok(r)) => serde_json::to_string(&r).unwrap_or_default(),
                                    Ok(Err(e)) => format!("工具执行失败: {}", e),
                                    Err(_) => "工具执行超时（超过 120 秒），已强制放弃".to_string(),
                                };

                            let _ = event_tx.send(AgentEvent::ToolCallEnd {
                                name: tc.function.name.clone(),
                                result: result_text.clone(),
                            });

                            history.push(ChatMessage::tool(&tc.id, &result_text));
                        }
                    }
                    Ok(())
                }
                .await;

                match result {
                    Ok(()) => {}
                    Err(e) if e.to_string().contains("__CANCELLED__") => {
                        // ✅ 取消：回滚 history，通知 UI
                        history.truncate(save_point);
                        let _ = event_tx.send(AgentEvent::Cancelled);
                    }
                    Err(e) => {
                        history.truncate(save_point);
                        let _ = event_tx.send(AgentEvent::Error(format!("{}", e)));
                    }
                }
            }
        }
    }
}
