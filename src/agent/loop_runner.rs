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

    let mut tools = mcp_tools_to_deepseek(&tools_result.tools);

        // ✅ 注入虚拟工具 read_skill，供模型查阅技能详情
    tools.push(json!({
        "type": "function",
        "function": {
            "name": "read_skill",
            "description": "读取指定技能的完整 Markdown 文档。参数为技能名称。",
            "parameters": {
                "type": "object",
                "properties": {
                    "skill_name": { "type": "string", "description": "要读取的技能名称，例如 server-health" }
                },
                "required": ["skill_name"]
            }
        }
    }));

    // ✅ 新增：在启动时，利用 MCP 注解和本地策略，构建一张“工具风险字典”
    let mut tool_risks = std::collections::HashMap::new();
    for t in &tools_result.tools {
        let risk = crate::agent::policy::evaluate_tool_risk(t);
        tool_risks.insert(t.name.to_string(), risk);
    }

    let _ = event_tx.send(AgentEvent::Ready {
        tool_count: tools.len(),
    });

    // ✅ 加载 LLM 历史
    let mut history: Vec<ChatMessage> = storage::load_json("llm_history.json").unwrap_or_default();

    // ✅ 动态加载项目规则和技能列表
    let agent_context = crate::agent::skill_loader::load_agent_context();

    let mut final_system_prompt = cfg.system_prompt.clone();
    if !agent_context.is_empty() {
        final_system_prompt = format!("{}\n\n{}", final_system_prompt, agent_context);
    }

    if !final_system_prompt.is_empty() {
        if history.is_empty() || history[0].role != "system" {
            history.insert(0, ChatMessage { role: "system".into(), content: Some(final_system_prompt), tool_calls: None, tool_call_id: None });
        } else {
            // 如果已经有 system，更新它的内容
            history[0].content = Some(final_system_prompt);
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
                        let mut usage_opt = None; // ✅ 新增

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
                                Ok((msg,usage)) => {
                                    assistant_opt = Some(msg);
                                    usage_opt = usage; // ✅ 记录
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

                        // ✅ 发出 Token 统计事件
                        if let Some(u) = usage_opt {
                            let _ = event_tx.send(AgentEvent::TokenUsage {
                                total_tokens: u.total_tokens,
                                prompt_tokens: u.prompt_tokens
                            });
                        }

                        let assistant = assistant_opt.expect("重试逻辑错误");
                                                // ✅ 上下文压缩逻辑：保留 System Prompt 和最近 20 条消息
                        if history.len() > 25 {
                            let _ = event_tx.send(AgentEvent::Status("上下文过长，正在压缩早期记忆...".into()));
                            
                            // 提取 System Prompt (如果有)
                            let system_msg = if !history.is_empty() && history[0].role == "system" {
                                Some(history[0].clone())
                            } else {
                                None
                            };
                            
                            // 只保留最近的 16 条消息
                            let recent_msgs: Vec<ChatMessage> = history.iter().rev().take(16).cloned().collect::<Vec<_>>().into_iter().rev().collect();
                            
                            history.clear();
                            if let Some(sys) = system_msg {
                                history.push(sys);
                            }
                            
                            // 插入一条摘要标记，告诉大模型之前有记忆被删除了
                            history.push(ChatMessage {
                                role: "system".into(),
                                content: Some("[注：由于对话较长，早期的历史消息已被压缩丢弃]".into()),
                                tool_calls: None, tool_call_id: None
                            });
                            
                            history.extend(recent_msgs);
                            crate::storage::save_json("llm_history.json", &history);
                        }
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

                            // ✅ 拦截本地虚拟工具 read_skill
                            if tc.function.name == "read_skill" {
                                let _ = event_tx.send(AgentEvent::ToolCallStart {
                                    id: tc.id.clone(), name: tc.function.name.clone(), args: tc.function.arguments.clone()
                                });
                                
                                let args: Value = serde_json::from_str(&tc.function.arguments).unwrap_or_else(|_| json!({}));
                                let skill_name = args.get("skill_name").and_then(|v| v.as_str()).unwrap_or("");
                                
                                let content = crate::agent::skill_loader::read_skill_file(skill_name);
                                
                                let _ = event_tx.send(AgentEvent::ToolCallEnd {
                                    id: tc.id.clone(), name: tc.function.name.clone(), result: content.clone()
                                });
                                
                                history.push(ChatMessage::tool(&tc.id, &content));
                                crate::storage::save_json("llm_history.json", &history);
                                
                                continue; // 拦截成功，直接进入下一个工具，不走 MCP
                            }

                            // ✅ 从字典里查出该工具的风险等级
                            let risk = *tool_risks.get(&tc.function.name).unwrap_or(&crate::agent::policy::RiskLevel::High);
                            
                            // 1. 明确被拉黑的工具
                            if let crate::agent::policy::RiskLevel::Deny = risk {
                                let reject_msg = "系统拦截：该工具已被策略模块拉黑，禁止执行。";
                                let _ = event_tx.send(AgentEvent::ToolCallEnd {
                                    id: tc.id.clone(), // ✅
                                    name: tc.function.name.clone(),
                                    result: reject_msg.into()
                                });
                                history.push(ChatMessage::tool(&tc.id, reject_msg));
                                continue;
                            }

                            // 2. 只有 High 需要审批
                            let mut approved = risk == crate::agent::policy::RiskLevel::Low;

                            if risk == crate::agent::policy::RiskLevel::High {
                                // 挂起审批逻辑
                                let _ = event_tx.send(AgentEvent::ToolCallPending {
                                    id: tc.id.clone(), // ✅
                                    name: tc.function.name.clone(),
                                    args: tc.function.arguments.clone()
                                });

                                loop {
                                    tokio::select! {
                                        _ = tokio::time::sleep(Duration::from_millis(50)) => {
                                            if cancel_flag.load(Ordering::SeqCst) { anyhow::bail!("__CANCELLED__"); }
                                        }
                                        cmd = cmd_rx.recv() => {
                                            match cmd {
                                                Some(AgentCommand::ToolApproval(a)) => { approved = a; break; }
                                                Some(AgentCommand::Shutdown) => std::process::exit(0),
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                            }

                            // 3. 被拒绝直接跳过
                            if !approved {
                                let reject_msg = "用户拒绝执行该工具调用。";
                                let _ = event_tx.send(AgentEvent::ToolCallEnd {
                                    id: tc.id.clone(), // ✅
                                    name: tc.function.name.clone(),
                                    result: reject_msg.into()
                                });
                                history.push(ChatMessage::tool(&tc.id, reject_msg));
                                continue;
                            }
                            
                            // 1. 发送 Pending 事件，挂起等待审批
                            let _ = event_tx.send(AgentEvent::ToolCallPending {
                                id: tc.id.clone(), // ✅
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
                                    id: tc.id.clone(), // ✅
                                    name: tc.function.name.clone(),
                                    result: reject_msg.into()
                                });
                                history.push(ChatMessage::tool(&tc.id, reject_msg));
                                crate::storage::save_json("llm_history.json", &history);
                                continue; // 直接跳过这次工具执行
                            }

                            // 4. 已批准，正式执行
                            let _ = event_tx.send(AgentEvent::ToolCallStart {
                                id: tc.id.clone(), // ✅
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
                                id: tc.id.clone(), // ✅
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
