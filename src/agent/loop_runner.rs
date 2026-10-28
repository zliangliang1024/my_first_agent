use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use rmcp::{ServiceExt, transport::TokioChildProcess};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use tokio::time::{timeout, Duration};

use crate::config::Config;
use crate::llm::deepseek::call_deepseek_stream;
use crate::llm::{ChatMessage, StreamDelta};
use crate::agent::policy::RiskLevel;
use super::{AgentCommand, AgentEvent};

fn sanitize_history(history: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut clean = Vec::new();
    let mut valid_tool_ids = std::collections::HashSet::new();

    for msg in history {
        if msg.role == "assistant" {
            if let Some(tcs) = &msg.tool_calls {
                for tc in tcs { valid_tool_ids.insert(tc.id.clone()); }
            }
            clean.push(msg.clone());
        } else if msg.role == "tool" {
            if let Some(id) = &msg.tool_call_id {
                if valid_tool_ids.contains(id) {
                    clean.push(msg.clone());
                    valid_tool_ids.remove(id);
                }
            }
        } else {
            clean.push(msg.clone());
        }
    }
    clean
}

pub async fn agent_loop(
    mut cfg: Config,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<AgentCommand>,
    event_tx: Sender<AgentEvent>,
    cancel_flag: Arc<AtomicBool>,
) {
    let _ = event_tx.send(AgentEvent::Status("正在连接 MCP...".into()));

    if cfg.mcp_exes.is_empty() {
        let _ = event_tx.send(AgentEvent::Error("未配置任何 MCP 路径".into()));
        return;
    }

    // 存储 所有连接的 MCP 客户端
    let mut mcp_clients: HashMap<String, rmcp::service::RunningService<rmcp::RoleClient, ()>> = HashMap::new();
    let mut all_tools: Vec<Value> = Vec::new();
    let mut tool_risks = HashMap::new();

    // 1. 加载多 MCP
    for exe_path in &cfg.mcp_exes {
        let prefix = std::path::Path::new(exe_path).file_stem().and_then(|s| s.to_str()).unwrap_or("mcp").replace("-mcp", "");
        let _ = event_tx.send(AgentEvent::Status(format!("正在连接 {}...", prefix)));

        let mut cmd = tokio::process::Command::new(exe_path);
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        let transport = match TokioChildProcess::new(cmd) {
            Ok(t) => t,
            Err(e) => { let _ = event_tx.send(AgentEvent::Error(format!("启动 {} 失败: {}", prefix, e))); continue; }
        };
        let client = match ().serve(transport).await {
            Ok(c) => c,
            Err(e) => { let _ = event_tx.send(AgentEvent::Error(format!("初始化 {} 失败: {}", prefix, e))); continue; }
        };
        let tools_result = match client.list_tools(None).await {
            Ok(t) => t,
            Err(_) => continue,
        };

        for t in &tools_result.tools {
            let new_name = format!("{}__{}", prefix, t.name);
            all_tools.push(json!({
                "type": "function",
                "function": { "name": new_name, "description": t.description.clone().unwrap_or_default(), "parameters": t.input_schema }
            }));
            tool_risks.insert(new_name, crate::agent::policy::evaluate_tool_risk(t));
        }
        mcp_clients.insert(prefix, client);
    }

    // 2. 注入本地工具
    all_tools.extend(crate::agent::local_tools::get_local_tools());
    tool_risks.insert("get_current_time".into(), RiskLevel::Low);
    tool_risks.insert("execute_local_shell".into(), RiskLevel::High);

    // 3. 注入技能工具 (Phase 4)
    let skills = crate::agent::skill_loader::scan_skills();
    all_tools.extend(crate::agent::skill_loader::get_skill_tools(&skills));
    for s in &skills {
        tool_risks.insert(format!("skill_{}", s.name.replace("-", "_")), RiskLevel::Low);
    }

    let _ = event_tx.send(AgentEvent::Ready { tool_count: all_tools.len() });

    let mut current_session_id = String::new();
    let sessions = crate::storage::load_sessions_index();
    if let Some(first) = sessions.first() { current_session_id = first.id.clone(); } 
    else {
        current_session_id = format!("session_{}", chrono::Local::now().timestamp_millis());
        let meta = crate::storage::SessionMeta { id: current_session_id.clone(), title: "新对话".into() };
        crate::storage::save_sessions_index(&[meta]);
    }

    let mut history = crate::storage::load_session_llm(&current_session_id);
    if !cfg.system_prompt.is_empty() {
        history.insert(0, ChatMessage { role: "system".into(), content: Some(cfg.system_prompt.clone()), tool_calls: None, tool_call_id: None });
    }

    // 主循环
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            AgentCommand::Shutdown => break,
            AgentCommand::ReloadConfig(new_cfg) => {
                cfg = new_cfg;
                let _ = event_tx.send(AgentEvent::Status("配置已更新，需重启生效".into()));
            }
            AgentCommand::ClearHistory => {
                history.clear();
                if !cfg.system_prompt.is_empty() {
                    history.push(ChatMessage { role: "system".into(), content: Some(cfg.system_prompt.clone()), tool_calls: None, tool_call_id: None });
                }
                crate::storage::save_session_llm(&current_session_id, &history);
                let _ = event_tx.send(AgentEvent::Status("历史已清空".into()));
            }
            AgentCommand::SwitchSession(id) => {
                current_session_id = id.clone();
                history = crate::storage::load_session_llm(&current_session_id);
                if !cfg.system_prompt.is_empty() {
                    history.insert(0, ChatMessage { role: "system".into(), content: Some(cfg.system_prompt.clone()), tool_calls: None, tool_call_id: None });
                }
                let _ = event_tx.send(AgentEvent::Status("会话已切换".into()));
            }
            AgentCommand::ToolApproval(_) => {} // 在内部 select! 处理
            AgentCommand::SendMessage(text) => {
                if cfg.deepseek_api_key.trim().is_empty() {
                    let _ = event_tx.send(AgentEvent::Error("未配置 API Key".into()));
                    continue;
                }
                cancel_flag.store(false, Ordering::SeqCst);
                let save_point = history.len();
                history.push(ChatMessage::user(&text));
                crate::storage::save_session_llm(&current_session_id, &history);

                let result: Result<()> = async {
                    loop {
                        let _ = event_tx.send(AgentEvent::Thinking);
                        let _ = event_tx.send(AgentEvent::AssistantStart);
                        let tx_delta = event_tx.clone();
                        let cancel_for_llm = cancel_flag.clone();

                        let mut attempt = 1;
                        let mut assistant_opt = None;
                        let mut usage_opt = None;

                        while attempt <= 3 {
                            let safe_history = sanitize_history(&history);
                            let res = call_deepseek_stream(&cfg, &safe_history, &all_tools, cancel_for_llm.clone(), {
                                let tx = tx_delta.clone();
                                move |delta| { if let StreamDelta::Content(s) = delta { let _ = tx.send(AgentEvent::AssistantDelta(s)); } }
                            }).await;

                            match res {
                                Ok((msg, usage)) => { assistant_opt = Some(msg); usage_opt = usage; break; }
                                Err(e) => {
                                    let err_str = e.to_string();
                                    if err_str.contains("__CANCELLED__") { anyhow::bail!("__CANCELLED__"); }
                                    if err_str.contains("400") { anyhow::bail!("请求格式错误(400): {}", err_str); }
                                    if attempt == 3 { anyhow::bail!("重试失败: {}", err_str); }
                                    let _ = event_tx.send(AgentEvent::Status(format!("网络失败，重试 ({}/3)...", attempt)));
                                    tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt - 1))).await;
                                    attempt += 1;
                                }
                            }
                        }

                        let assistant = assistant_opt.expect("重试错误");
                        // 发送 token 统计
                        if let Some(u) = usage_opt {
                            let _ = event_tx.send(AgentEvent::TokenUsage { total_tokens: u.total_tokens, prompt_tokens: u.prompt_tokens });
                        }

                        if assistant.tool_calls.is_none() {
                            let text = assistant.content.clone().unwrap_or_default();
                            history.push(assistant);
                            crate::storage::save_session_llm(&current_session_id, &history);
                            let _ = event_tx.send(AgentEvent::AssistantReply(text));
                            break;
                        }

                        let tool_calls = assistant.tool_calls.clone().unwrap();
                        history.push(assistant);
                        crate::storage::save_session_llm(&current_session_id, &history);

                        for tc in &tool_calls {
                            if cancel_flag.load(Ordering::SeqCst) { anyhow::bail!("__CANCELLED__"); }
                            
                            let risk = *tool_risks.get(&tc.function.name).unwrap_or(&RiskLevel::High);
                            if let RiskLevel::Deny = risk {
                                let reject_msg = "系统拦截：策略禁止该工具。";
                                let _ = event_tx.send(AgentEvent::ToolCallEnd { id: tc.id.clone(), name: tc.function.name.clone(), result: reject_msg.into() });
                                history.push(ChatMessage::tool(&tc.id, reject_msg));
                                crate::storage::save_session_llm(&current_session_id, &history);
                                continue;
                            }

                            let mut approved = risk == RiskLevel::Low;
                            if risk == RiskLevel::High {
                                let _ = event_tx.send(AgentEvent::ToolCallPending { id: tc.id.clone(), name: tc.function.name.clone(), args: tc.function.arguments.clone() });
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

                            if !approved {
                                let reject_msg = "用户拒绝执行该工具。";
                                let _ = event_tx.send(AgentEvent::ToolCallEnd { id: tc.id.clone(), name: tc.function.name.clone(), result: reject_msg.into() });
                                history.push(ChatMessage::tool(&tc.id, reject_msg));
                                crate::storage::save_session_llm(&current_session_id, &history);
                                continue;
                            }

                            let _ = event_tx.send(AgentEvent::ToolCallStart { id: tc.id.clone(), name: tc.function.name.clone(), args: tc.function.arguments.clone() });
                            let args: Value = serde_json::from_str(&tc.function.arguments).unwrap_or_else(|_| json!({}));
                            let mut result_text = String::new();

                            // A. 本地工具
                            if ["get_current_time", "execute_local_shell"].contains(&tc.function.name.as_str()) {
                                let (content, _) = crate::agent::local_tools::execute_local_tool(&tc.function.name, &args);
                                result_text = content;
                            } 
                            // B. 技能
                            else if tc.function.name.starts_with("skill_") {
                                let skill_dir_name = tc.function.name.trim_start_matches("skill_").replace("_", "-");
                                result_text = crate::agent::skill_loader::read_skill_file(&skill_dir_name);
                            } 
                            // C. MCP
                            else {
                                let mut parts = tc.function.name.splitn(2, "__");
                                let prefix = parts.next().unwrap_or("");
                                let real_name = parts.next().unwrap_or(&tc.function.name);
                                if let Some(client) = mcp_clients.get(prefix) {
                                    let mut params = CallToolRequestParams::new(real_name.to_string());
                                    if let Some(map) = args.as_object() { params = params.with_arguments(map.clone()); }
                                    result_text = match timeout(Duration::from_secs(120), client.call_tool(params)).await {
                                        Ok(Ok(r)) => serde_json::to_string(&r).unwrap_or_default(),
                                        Ok(Err(e)) => format!("工具执行失败: {}", e),
                                        Err(_) => "工具执行超时".to_string(),
                                    };
                                } else {
                                    result_text = "未知的 MCP 服务器前缀".to_string();
                                }
                            }

                            // 审计日志
                            crate::storage::append_audit_log(&tc.function.name, &tc.function.arguments, &result_text);

                            let _ = event_tx.send(AgentEvent::ToolCallEnd { id: tc.id.clone(), name: tc.function.name.clone(), result: result_text.clone() });
                            history.push(ChatMessage::tool(&tc.id, &result_text));
                            crate::storage::save_session_llm(&current_session_id, &history);
                        }
                    }
                    Ok(())
                }.await;

                match result {
                    Ok(()) => {}
                    Err(e) if e.to_string().contains("__CANCELLED__") => {
                        history.truncate(save_point);
                        crate::storage::save_session_llm(&current_session_id, &history);
                        let _ = event_tx.send(AgentEvent::Cancelled);
                    }
                    Err(e) => {
                        history.truncate(save_point);
                        crate::storage::save_session_llm(&current_session_id, &history);
                        let _ = event_tx.send(AgentEvent::Error(format!("{}", e)));
                    }
                }
            }
        }
    }
}