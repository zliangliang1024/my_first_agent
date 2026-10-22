use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;

use eframe::egui::{self, Margin};
use egui_commonmark::CommonMarkCache;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use crate::agent::{AgentCommand, AgentEvent};
use crate::config::Config;
use crate::storage;
use crate::ui::style::PAGE_BG;

#[derive(Serialize, Deserialize)]
pub enum ToolStatus {
    Running,
    Done(String),
}

#[derive(Serialize, Deserialize)]
pub enum ChatItem {
    User(String),
    Assistant(String),
    ToolCall {
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

    pub show_settings: bool,
    pub edit_cfg: Config,
    pub active_cfg: Config,
}

impl AgentApp {
    pub fn new(
        cmd_tx: UnboundedSender<AgentCommand>,
        event_rx: Receiver<AgentEvent>,
        cancel_flag: Arc<AtomicBool>,
        initial_cfg: Config,
    ) -> Self {
        let is_key_empty = initial_cfg.deepseek_api_key.trim().is_empty();

        // ✅ 加载历史
        let mut chat: Vec<ChatItem> = storage::load_json("ui_history.json").unwrap_or_default();

        // ✅ 修复错误1：使用 if let *status 判断
        for item in &mut chat {
            if let ChatItem::ToolCall { status, .. } = item {
                if let ToolStatus::Running = *status {
                    *status = ToolStatus::Done("(程序重启，操作已中断)".into());
                }
            }
        }

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
            show_settings: is_key_empty,
            edit_cfg: initial_cfg.clone(),
            active_cfg: initial_cfg,
        }
    }

    pub fn poll_events(&mut self) {
        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
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
                    storage::save_json("ui_history.json", &self.chat);
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
                    self.chat.push(ChatItem::Assistant(text));
                    self.is_waiting = false;
                    self.status = "就绪".into();
                    storage::save_json("ui_history.json", &self.chat);
                }
                AgentEvent::ToolCallStart { name, args } => {
                    self.streaming_text = None;
                    self.status = format!("调用 {}", name);
                    self.chat.push(ChatItem::ToolCall {
                        name,
                        args,
                        status: ToolStatus::Running,
                    });
                    storage::save_json("ui_history.json", &self.chat);
                }
                AgentEvent::ToolCallEnd { name, result } => {
                    let idx = self.chat.iter().rposition(|item: &ChatItem| {
                        if let ChatItem::ToolCall {
                            name: n, status, ..
                        } = item
                        {
                            n == &name && matches!(status, &ToolStatus::Running)
                        } else {
                            false
                        }
                    });
                    if let Some(i) = idx {
                        let item: &mut ChatItem = &mut self.chat[i];
                        if let ChatItem::ToolCall { status, .. } = item {
                            *status = ToolStatus::Done(result);
                        }
                    }
                    self.status = "思考中...".into();
                    storage::save_json("ui_history.json", &self.chat);
                }
                AgentEvent::Cancelled => {
                    self.streaming_text = None;
                    self.is_waiting = false;
                    self.status = "已取消".into();
                    self.chat.push(ChatItem::Status("已取消本次生成".into()));
                    storage::save_json("ui_history.json", &self.chat);
                }
                AgentEvent::Error(err) => {
                    self.streaming_text = None;
                    self.chat.push(ChatItem::Error(err));
                    self.is_waiting = false;
                    self.status = "出错".into();
                    storage::save_json("ui_history.json", &self.chat);
                }
            }
        }
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
        storage::save_json("ui_history.json", &self.chat);
        let _ = self.cmd_tx.send(AgentCommand::SendMessage(text));
    }
}

// ✅ 修复错误2：使用 fn ui 替代 fn update
impl eframe::App for AgentApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_events();

        let has_api_key = !self.active_cfg.deepseek_api_key.trim().is_empty();
        // ✅ 强制拦截：没有 Key 就必定弹窗，关不掉！
        if !has_api_key {
            self.show_settings = true;
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
