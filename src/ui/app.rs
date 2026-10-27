use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;

use eframe::egui::{self, Margin};
use egui_commonmark::CommonMarkCache;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use crate::agent::{AgentCommand, AgentEvent};
use crate::config::Config;
use crate::storage::{self, SessionMeta};
use crate::ui::style::PAGE_BG;

#[derive(Serialize, Deserialize)]
pub enum ToolStatus {
    Pending, // ✅ 新增
    Running,
    Done(String),
}

#[derive(Serialize, Deserialize)]
pub enum ChatItem {
    User(String),
    Assistant(String),
    ToolCall {
        id: String, // ✅ 新增
        name: String,
        args: String,
        status: ToolStatus,
    },
    Error(String),
    Status(String),
}

pub struct AgentApp {
    pub cmd_tx: UnboundedSender<AgentCommand>,
    pub event_rx: Receiver<AgentEvent>,

    pub input: String,
    pub chat: Vec<ChatItem>,
    pub is_waiting: bool,
    pub status: String,
    pub mcp_ready: bool,
    pub md_cache: CommonMarkCache,
    pub streaming_text: Option<String>,
    pub cancel_flag: Arc<AtomicBool>,
    pub pending_tool: Option<(String, String)>, // ✅ 新增：待审批工具 (name, args)
    pub show_settings: bool,
    pub edit_cfg: Config,
    pub active_cfg: Config,
    pub total_tokens: u64,           // ✅ 新增
    pub last_prompt_tokens: u64,     // ✅ 新增
    pub sessions: Vec<SessionMeta>,  // ✅ 会话列表
    pub current_session_id: String,  // ✅ 当前选中的 ID
    pub renaming_id: Option<String>, // ✅ 记录正在重命名的会话 ID
    pub rename_buffer: String,       // ✅ 重命名时的输入缓冲
}

impl AgentApp {
    pub fn new(
        cmd_tx: UnboundedSender<AgentCommand>,
        event_rx: Receiver<AgentEvent>,
        cancel_flag: Arc<AtomicBool>,
        initial_cfg: Config,
    ) -> Self {
        let is_key_empty = initial_cfg.deepseek_api_key.trim().is_empty();

        // 获取 session
        let mut sessions = crate::storage::load_sessions_index();
        let current_session_id = if let Some(first) = sessions.first() {
            first.id.clone()
        } else {
            let id = format!("session_{}", chrono::Local::now().timestamp_millis());
            sessions.push(SessionMeta {
                id: id.clone(),
                title: "新对话".into(),
            });
            crate::storage::save_sessions_index(&sessions);
            id
        };

        // ✅ 加载历史
        let mut chat: Vec<ChatItem> = storage::load_json("ui_history.json").unwrap_or_default();

        // 修改加载历史时的防崩溃逻辑
        for item in &mut chat {
            // ✅ 去掉 status 后面的 : ToolStatus
            if let ChatItem::ToolCall { status, .. } = item {
                // ✅ 用 if let 或 matches!
                if matches!(status, ToolStatus::Running | ToolStatus::Pending) {
                    *status = ToolStatus::Done("(程序重启，操作已中断)".into());
                }
            }
        }

        let saved_total_tokens: u64 = storage::load_json("total_tokens.json").unwrap_or(0);

        Self {
            cmd_tx,
            event_rx,
            input: String::new(),
            chat,
            is_waiting: false,
            status: if is_key_empty {
                "未配置 API Key".into()
            } else {
                "正在连接 MCP...".into()
            },
            mcp_ready: false,
            md_cache: CommonMarkCache::default(),
            streaming_text: None,
            cancel_flag,
            pending_tool: None,
            show_settings: is_key_empty,
            edit_cfg: initial_cfg.clone(),
            active_cfg: initial_cfg,
            total_tokens: saved_total_tokens,
            last_prompt_tokens: 0,
            sessions,
            current_session_id,
            renaming_id: None,
            rename_buffer: String::new(),
        }
    }

    // ✅ 新增新建会话方法
    pub fn new_session(&mut self) {
        let now = chrono::Local::now();
        let id = format!("session_{}", now.timestamp_millis());

        // ✅ 带日期前缀
        let title = format!("对话 {}", now.format("%m-%d %H:%M"));

        self.sessions.push(crate::storage::SessionMeta {
            id: id.clone(),
            title: title.clone(),
        });
        crate::storage::save_sessions_index(&self.sessions);

        self.current_session_id = id.clone();
        self.chat.clear();
        self.input.clear();
        self.status = format!("已新建 {}", title);
        let _ = self.cmd_tx.send(AgentCommand::SwitchSession(id));
    }

    // ✅ 提交重命名的方法
    pub fn commit_rename(&mut self) {
        if let Some(id) = self.renaming_id.take() {
            let new_title = self.rename_buffer.trim().to_string();
            if !new_title.is_empty() {
                if let Some(s) = self.sessions.iter_mut().find(|s| s.id == id) {
                    s.title = new_title;
                    crate::storage::save_sessions_index(&self.sessions);
                }
            }
        }
    }

    // ✅ 新增切换会话方法
    pub fn switch_session(&mut self, id: &str) {
        if self.current_session_id == id {
            return;
        }
        self.current_session_id = id.to_string();
        self.chat = crate::storage::load_session_ui(id);
        self.status = "会话已切换".into();
        let _ = self
            .cmd_tx
            .send(AgentCommand::SwitchSession(id.to_string()));
    }

    // ✅ 删除会话
    pub fn delete_session(&mut self, id: &str) {
        // 1. 从列表里移除
        self.sessions.retain(|s| s.id != id);
        crate::storage::save_sessions_index(&self.sessions);
        crate::storage::delete_session_files(id);

        // 2. 如果删除的是当前正在看的会话，需要做特殊处理
        if self.current_session_id == id {
            if let Some(first) = self.sessions.first() {
                // 自动切到剩下的第一个会话
                let next_id = first.id.clone();
                self.switch_session(&next_id);
            } else {
                // 如果全删光了，自动新建一个
                self.new_session();
            }
        }
    }

    pub fn poll_events(&mut self) -> bool {
        let mut handled = false;
        while let Ok(ev) = self.event_rx.try_recv() {
            handled = true; // ✅ 只要进循环，说明有事件到了
            match ev {
                AgentEvent::TokenUsage {
                    total_tokens,
                    prompt_tokens,
                } => {
                    self.total_tokens += total_tokens; // ✅ 累加
                    self.last_prompt_tokens = prompt_tokens;
                    // ✅ 每次累加后，立刻存盘
                    crate::storage::save_json("total_tokens.json", &self.total_tokens);
                }
                AgentEvent::Status(s) => {
                    self.status = s.clone();
                    self.chat.push(ChatItem::Status(s));
                }
                AgentEvent::Ready { tool_count } => {
                    self.mcp_ready = true;
                    self.status = format!("MCP 就绪 · {} 个工具", tool_count);
                    self.chat.push(ChatItem::Status(format!(
                        "MCP 已连接，共 {} 个可用工具",
                        tool_count
                    )));
                    storage::save_session_ui(&self.current_session_id, &self.chat);
                }
                AgentEvent::Thinking => {
                    self.status = "思考中...".into();
                }
                AgentEvent::AssistantStart => {
                    self.streaming_text = Some(String::new());
                    self.status = "生成中...".into();
                }
                AgentEvent::AssistantDelta(text) => match &mut self.streaming_text {
                    Some(buf) => buf.push_str(&text),
                    None => self.streaming_text = Some(text),
                },
                AgentEvent::AssistantReply(text) => {
                    self.streaming_text = None;
                    self.pending_tool = None; // ✅ 确保模态框状态清空

                    // ✅ 终极清扫：只要最终回复到达，说明这轮彻底结束。
                    // 所有遗留的 Pending 或 Running 工具，全部强制标为 Done，防止 UI 鬼畜！
                    for item in self.chat.iter_mut() {
                        if let ChatItem::ToolCall { status, .. } = item {
                            if matches!(status, ToolStatus::Pending | ToolStatus::Running) {
                                *status =
                                    ToolStatus::Done("(工具执行完毕，已由 Agent 汇总结果)".into());
                            }
                        }
                    }

                    self.chat.push(ChatItem::Assistant(text));
                    self.is_waiting = false;
                    self.status = "就绪".into();
                    crate::storage::save_session_ui(&self.current_session_id, &self.chat);
                }
                // 在 poll_events 的 match 里增加：
                AgentEvent::ToolCallPending { id, name, args } => {
                    self.streaming_text = None;
                    self.pending_tool = Some((name.clone(), args.clone()));
                    self.status = format!("等待审批: {}", name);
                    // ✅ 使用 id 入库
                    self.chat.push(ChatItem::ToolCall {
                        id,
                        name,
                        args,
                        status: ToolStatus::Pending,
                    });
                    storage::save_session_ui(&self.current_session_id, &self.chat);
                }
                AgentEvent::ToolCallStart { id, name, .. } => {
                    self.pending_tool = None;
                    self.status = format!("调用 {}", name);
                    // ✅ 按 id 查找
                    for item in self.chat.iter_mut() {
                        if let ChatItem::ToolCall {
                            id: n_id, status, ..
                        } = item
                        {
                            if n_id == &id {
                                *status = ToolStatus::Running;
                                break;
                            }
                        }
                    }
                    storage::save_session_ui(&self.current_session_id, &self.chat);
                }
                AgentEvent::ToolCallEnd { id, name, result } => {
                    self.pending_tool = None;
                    self.status = "思考中...".into();
                    // ✅ 按 id 查找并修改
                    for item in self.chat.iter_mut() {
                        if let ChatItem::ToolCall {
                            id: n_id, status, ..
                        } = item
                        {
                            if n_id == &id {
                                *status = ToolStatus::Done(result.clone());
                                break;
                            }
                        }
                    }
                    storage::save_session_ui(&self.current_session_id, &self.chat);
                }
                AgentEvent::Cancelled => {
                    self.streaming_text = None;
                    self.is_waiting = false;
                    self.status = "已取消".into();
                    self.chat.push(ChatItem::Status("已取消本次生成".into()));
                    storage::save_session_ui(&self.current_session_id, &self.chat);
                }
                AgentEvent::Error(err) => {
                    self.streaming_text = None;
                    self.chat.push(ChatItem::Error(err));
                    self.is_waiting = false;
                    self.status = "出错".into();
                    storage::save_session_ui(&self.current_session_id, &self.chat);
                }
            }
        }
        handled // ✅ 返回结果
    }

    pub fn send_current_input(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() || self.is_waiting || !self.mcp_ready {
            return;
        }
        self.chat.push(ChatItem::User(text.clone()));
        self.input.clear();
        self.is_waiting = true;
        self.status = "发送中...".into();
        storage::save_session_ui(&self.current_session_id, &self.chat);
        let _ = self.cmd_tx.send(AgentCommand::SendMessage(text));
    }
    // ✅ 在 impl AgentApp 里加这个方法
    pub fn approve_tool(&mut self, approved: bool) {
        if self.pending_tool.is_some() {
            let _ = self.cmd_tx.send(AgentCommand::ToolApproval(approved));
            self.pending_tool = None;
            self.status = if approved {
                "正在执行...".into()
            } else {
                "已拒绝执行".into()
            };
        }
    }
}

// ✅ 修复错误2：使用 fn ui 替代 fn update
impl eframe::App for AgentApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let handled_events = self.poll_events(); // ✅ 接收是否处理了事件

        let has_api_key = !self.active_cfg.deepseek_api_key.trim().is_empty();
        if !has_api_key {
            self.show_settings = true;
        }

        // ✅ 关键：只要处理了事件，或者正在等待，或者正在生成，就强制请求重绘
        if handled_events || self.is_waiting || !self.mcp_ready || self.streaming_text.is_some() {
            ui.ctx().request_repaint();
        }

        if self.is_waiting || !self.mcp_ready || self.streaming_text.is_some() {
            ui.ctx().request_repaint();
        }

        let mut do_send = false;
        let mut do_exit = false;

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(PAGE_BG)
                    .inner_margin(Margin::symmetric(16, 16)),
            )
            .show(ui, |ui| {
                // ✅ 用 show_inside 代替 show
                let (send, exit) = self.draw_main_view(ui, has_api_key);
                do_send = send;
                do_exit = exit;
            });

        // ✅ 弹窗也需要传入 ctx
        self.draw_settings_modal(ui.ctx());

        if do_send {
            self.send_current_input();
        }
        if do_exit {
            let _ = self.cmd_tx.send(AgentCommand::Shutdown);
            std::process::exit(0);
        }
    }
}
