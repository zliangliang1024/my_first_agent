use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use rmcp::{ServiceExt, transport::TokioChildProcess};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

use super::{AgentCommand, AgentEvent};
use crate::config::Config;
use crate::llm::deepseek::call_deepseek_stream;
use crate::llm::{ChatMessage, StreamDelta};
use crate::storage; // ✅ 引入 storage

fn mcp_tools_to_deepseek(tools: &[rmcp::model::Tool]) -> Vec<Value> {
    tools.iter().map(|t| json!({
        "type": "function",
        "function": { "name": t.name, "description": t.description.clone().unwrap_or_default(), "parameters": t.input_schema }
    })).collect()
}

pub async fn agent_loop(
    mut cfg: Config,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<AgentCommand>,
    event_tx: Sender<AgentEvent>,
    cancel_flag: Arc<AtomicBool>,
) {
    let _ = event_tx.send(AgentEvent::Status("正在连接 MCP...".into()));

    if cfg.dbx_exe.trim().is_empty() {
        let _ = event_tx.send(AgentEvent::Error("未配置 DBX 路径，请在设置中填写".into()));
        return;
    }

    let transport = match TokioChildProcess::new(Command::new(&cfg.dbx_exe)) {
        Ok(t) => t,
        Err(e) => {
            let _ = event_tx.send(AgentEvent::Error(format!("启动失败: {}", e)));
            return;
        }
    };
    let client = match ().serve(transport).await {
        Ok(c) => c,
        Err(e) => {
            let _ = event_tx.send(AgentEvent::Error(format!("初始化失败: {}", e)));
            return;
        }
    };

    let tools_result = match client.list_tools(None).await {
        Ok(t) => t,
        Err(e) => {
            let _ = event_tx.send(AgentEvent::Error(format!("获取工具失败: {}", e)));
            return;
        }
    };

    let tools = mcp_tools_to_deepseek(&tools_result.tools);
    let _ = event_tx.send(AgentEvent::Ready {
        tool_count: tools.len(),
    });

    // ✅ 加载 LLM 历史
    let mut history: Vec<ChatMessage> = storage::load_json("llm_history.json").unwrap_or_default();

    // ✅ 检查并插入 System Prompt
    if !cfg.system_prompt.is_empty() {
        if history.is_empty() || history[0].role != "system" {
            history.insert(
                0,
                ChatMessage {
                    role: "system".into(),
                    content: Some(cfg.system_prompt.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
        }
    }

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            AgentCommand::Shutdown => break,
            AgentCommand::ReloadConfig(new_cfg) => {
                cfg = new_cfg;
                let _ = event_tx.send(AgentEvent::Status("配置已更新并生效".into()));
            }
            AgentCommand::SendMessage(text) => {
                if cfg.deepseek_api_key.trim().is_empty() {
                    let _ = event_tx.send(AgentEvent::Error("未配置 API Key".into()));
                    continue;
                }

                cancel_flag.store(false, Ordering::SeqCst);
                let save_point = history.len();
                history.push(ChatMessage::user(&text));
                storage::save_json("llm_history.json", &history); // ✅ 保存

                let result: Result<()> = async {
                    loop {
                        let _ = event_tx.send(AgentEvent::Thinking);
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
                                    if err_str.contains("__CANCELLED__") {
                                        anyhow::bail!("__CANCELLED__");
                                    }
                                    if attempt == max_attempts {
                                        anyhow::bail!("重试失败: {}", err_str);
                                    }
                                    let _ = event_tx.send(AgentEvent::Status(format!(
                                        "网络失败，重试 ({}/{})...",
                                        attempt, max_attempts
                                    )));
                                    tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt - 1)))
                                        .await;
                                    attempt += 1;
                                }
                            }
                        }

                        let assistant = assistant_opt.expect("重试逻辑错误");
                        if assistant.tool_calls.is_none() {
                            let text = assistant.content.clone().unwrap_or_default();
                            history.push(assistant);
                            storage::save_json("llm_history.json", &history); // ✅ 保存
                            let _ = event_tx.send(AgentEvent::AssistantReply(text));
                            break;
                        }

                        let tool_calls = assistant.tool_calls.clone().unwrap();
                        history.push(assistant);
                        storage::save_json("llm_history.json", &history); // ✅ 保存

                                                for tc in &tool_calls {
                            if cancel_flag.load(Ordering::SeqCst) { anyhow::bail!("__CANCELLED__"); }
                            
                            // 1. 发送 Pending 事件，挂起等待审批
                            let _ = event_tx.send(AgentEvent::ToolCallPending { 
                                name: tc.function.name.clone(), 
                                args: tc.function.arguments.clone() 
                            });

                            // 2. 阻塞等待 UI 发送 ToolApproval 指令
                            let mut approved = false;
                            loop {
                                tokio::select! {
                                    _ = tokio::time::sleep(Duration::from_millis(50)) => {
                                        if cancel_flag.load(Ordering::SeqCst) { anyhow::bail!("__CANCELLED__"); }
                                    }
                                    cmd = cmd_rx.recv() => {
                                        match cmd {
                                            Some(AgentCommand::ToolApproval(a)) => { approved = a; break; }
                                            Some(AgentCommand::Shutdown) => std::process::exit(0),
                                            _ => {} // 忽略其他指令
                                        }
                                    }
                                }
                            }

                            // 3. 如果被拒绝，伪造一个拒绝结果写入历史
                            if !approved {
                                let reject_msg = "用户拒绝执行该工具调用。";
                                let _ = event_tx.send(AgentEvent::ToolCallEnd { 
                                    name: tc.function.name.clone(), 
                                    result: reject_msg.into() 
                                });
                                history.push(ChatMessage::tool(&tc.id, reject_msg));
                                crate::storage::save_json("llm_history.json", &history);
                                continue; // 直接跳过这次工具执行
                            }

                            // 4. 已批准，正式执行
                            let _ = event_tx.send(AgentEvent::ToolCallStart { 
                                name: tc.function.name.clone(), 
                                args: tc.function.arguments.clone() 
                            });
                            
                            let args: Value = serde_json::from_str(&tc.function.arguments).unwrap_or_else(|_| json!({}));
                            let mut params = CallToolRequestParams::new(tc.function.name.clone());
                            if let Some(map) = args.as_object() { params = params.with_arguments(map.clone()); }

                            let result_text = match timeout(Duration::from_secs(120), client.call_tool(params)).await {
                                Ok(Ok(r)) => serde_json::to_string(&r).unwrap_or_default(),
                                Ok(Err(e)) => format!("工具执行失败: {}", e),
                                Err(_) => "工具执行超时（超过 120 秒），已强制放弃".to_string(),
                            };

                            let _ = event_tx.send(AgentEvent::ToolCallEnd { 
                                name: tc.function.name.clone(), 
                                result: result_text.clone() 
                            });
                            history.push(ChatMessage::tool(&tc.id, &result_text));
                            crate::storage::save_json("llm_history.json", &history);
                        }
                    }
                    Ok(())
                }
                .await;

                match result {
                    Ok(()) => {}
                    Err(e) if e.to_string().contains("__CANCELLED__") => {
                        history.truncate(save_point);
                        storage::save_json("llm_history.json", &history); // ✅ 回滚并保存
                        let _ = event_tx.send(AgentEvent::Cancelled);
                    }
                    Err(e) => {
                        history.truncate(save_point);
                        storage::save_json("llm_history.json", &history); // ✅ 回滚并保存
                        let _ = event_tx.send(AgentEvent::Error(format!("{}", e)));
                    }
                }
            }
            AgentCommand::ClearHistory => {
                history.clear();
                if !cfg.system_prompt.is_empty() {
                    history.push(ChatMessage {
                        role: "system".into(),
                        content: Some(cfg.system_prompt.clone()),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
                crate::storage::save_json("llm_history.json", &history);
                let _ = event_tx.send(AgentEvent::Status("历史已清空".into()));
            }
            // ✅ 忽略多余的审批指令，避免编译报错
            AgentCommand::ToolApproval(_) => {}
        }
    }
}
