mod agent;
mod config;
mod llm;

use std::sync::mpsc::Receiver;

use eframe::egui;
use egui::{Color32, Margin, RichText};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use tokio::sync::mpsc::UnboundedSender;

use agent::{AgentCommand, AgentEvent};
use config::Config;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// ---------------- UI 展示项 ----------------

enum ToolStatus {
    Running,
    Done(String),
}

enum ChatItem {
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

// ---------------- 浅色配色 ----------------

const USER_BG: Color32 = Color32::from_rgb(37, 99, 235);
const ASSIST_BG: Color32 = Color32::from_rgb(238, 240, 244);
const TOOL_BG: Color32 = Color32::from_rgb(255, 248, 225);
const ERROR_BG: Color32 = Color32::from_rgb(253, 236, 234);

const TEXT_COLOR: Color32 = Color32::from_rgb(31, 35, 40);
const SUBTLE_TEXT: Color32 = Color32::from_rgb(110, 118, 129);

const PAGE_BG: Color32 = Color32::from_rgb(245, 247, 250);
const CARD_BG: Color32 = Color32::from_rgb(255, 255, 255);
const INPUT_BG: Color32 = Color32::from_rgb(255, 255, 255);
const BORDER_COLOR: Color32 = Color32::from_rgb(220, 224, 230);

// ---------------- App ----------------

struct AgentApp {
    cmd_tx: UnboundedSender<AgentCommand>,
    event_rx: Receiver<AgentEvent>,

    input: String,
    chat: Vec<ChatItem>,
    is_waiting: bool,
    status: String,
    mcp_ready: bool,

    md_cache: CommonMarkCache, // ✅ 新增

    // ✅ 新增：正在流式接收的文本
    streaming_text: Option<String>,

    cancel_flag: Arc<AtomicBool>, // ✅ 新增

    show_settings: bool, // 控制弹窗显示
    edit_cfg: Config,    // 正在编辑的临时配置
    active_cfg: Config,  // ✅ 新增：真正生效的
}

impl AgentApp {
    fn new(
        cmd_tx: UnboundedSender<AgentCommand>,
        event_rx: Receiver<AgentEvent>,
        cancel_flag: Arc<AtomicBool>,
        initial_cfg: Config, // ✅ 新增参数
    ) -> Self {
        // ✅ 检查 Key 是否为空，如果为空，启动直接弹窗
        let is_key_empty = initial_cfg.deepseek_api_key.trim().is_empty();
        Self {
            cmd_tx,
            event_rx,
            input: String::new(),
            chat: Vec::new(),
            is_waiting: false,
            status: "正在连接 MCP...".into(),
            mcp_ready: false,
            md_cache: CommonMarkCache::default(), // ✅ 新增
            streaming_text: None,
            cancel_flag,                 // ✅
            show_settings: is_key_empty, // ✅ 自动弹窗
            edit_cfg: initial_cfg.clone(),
            active_cfg: initial_cfg, // ✅ 新增：真正生效的
        }
    }

    fn poll_events(&mut self) {
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
                    self.streaming_text = None; // ✅ 清除流式气泡
                    self.chat.push(ChatItem::Assistant(text));
                    self.is_waiting = false;
                    self.status = "就绪".into();
                }
                AgentEvent::ToolCallStart { name, args } => {
                    self.streaming_text = None; // ✅ 进入工具调用阶段，清除流式气泡
                    self.status = format!("调用 {}", name);
                    self.chat.push(ChatItem::ToolCall {
                        name,
                        args,
                        status: ToolStatus::Running,
                    });
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
                }
                AgentEvent::Error(err) => {
                    self.chat.push(ChatItem::Error(err));
                    self.is_waiting = false;
                    self.status = "出错".into();
                }
                AgentEvent::Cancelled => {
                    self.streaming_text = None;
                    self.is_waiting = false;
                    self.status = "已取消".into();
                    self.chat.push(ChatItem::Status("已取消本次生成".into()));
                }
            }
        }
    }

    fn send_current_input(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() || self.is_waiting || !self.mcp_ready {
            return;
        }
        self.chat.push(ChatItem::User(text.clone()));
        self.input.clear();
        self.is_waiting = true;
        self.status = "发送中...".into();
        let _ = self.cmd_tx.send(AgentCommand::SendMessage(text));
    }
}

// ---------------- 气泡辅助 ----------------

fn bubble(
    ui: &mut egui::Ui,
    bg: Color32,
    align_right: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    // ✅ 用实际可用宽度，不设硬下限
    let max_w = (ui.available_width() * 0.88).max(80.0);

    let layout = if align_right {
        egui::Layout::right_to_left(egui::Align::TOP)
    } else {
        egui::Layout::left_to_right(egui::Align::TOP)
    };

    ui.with_layout(layout, |ui| {
        egui::Frame::new()
            .fill(bg)
            .corner_radius(10.0)
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_max_width(max_w);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.set_min_width(40.0); // ← 从 60 降到 40
                add_contents(ui);
            });
    });
}

/// ✅ 新增：Markdown 渲染气泡
fn bubble_md(
    ui: &mut egui::Ui,
    bg: Color32,
    align_right: bool,
    md_cache: &mut CommonMarkCache,
    text: &str,
    bubble_index: usize, // ✅ 参数回来了
) {
    let max_w = (ui.available_width() * 0.88).max(80.0);

    let layout = if align_right {
        egui::Layout::right_to_left(egui::Align::TOP)
    } else {
        egui::Layout::left_to_right(egui::Align::TOP)
    };

    ui.with_layout(layout, |ui| {
        egui::Frame::new()
            .fill(bg)
            .corner_radius(10.0)
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_max_width(max_w);
                ui.set_min_width(40.0);
                // ✅ 关键：独立 ID 作用域，避免多表格 Grid ID 冲突
                ui.push_id(bubble_index, |ui| {
                    egui::ScrollArea::horizontal()
                        .id_salt(format!("md_scroll_{}", bubble_index))
                        .show(ui, |ui| {
                            CommonMarkViewer::new().show(ui, md_cache, text);
                        });
                });
            });
    });
}

// ---------------- App 主循环 ----------------

impl eframe::App for AgentApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_events();

        // ✅ 提前定义好，让整个 ui 方法都能用
        let has_api_key = !self.active_cfg.deepseek_api_key.trim().is_empty();

        // ✅ 等待期间持续请求重绘，保证后台事件能及时刷新
        if self.is_waiting || !self.mcp_ready || self.streaming_text.is_some() {
            ui.ctx().request_repaint();
        }

        let mut do_send = false;
        let mut do_exit = false;

        // ========== 最外层留白 ==========
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(PAGE_BG)
                    .inner_margin(Margin::symmetric(16, 16)),
            )
            .show(ui, |ui| {
                // ---------- 顶部状态栏 ----------
                ui.horizontal(|ui| {
                    ui.add_space(2.0);
                    ui.heading(
                        RichText::new("Rust egui Agent")
                            .color(TEXT_COLOR)
                            .strong()
                            .size(18.0),
                    );
                    ui.add_space(8.0);
                    // ✅ 如果没 Key，状态栏显示红色
                    let status_color = if !has_api_key {
                        Color32::RED
                    } else {
                        SUBTLE_TEXT
                    };

                    ui.label(
                        RichText::new(format!("· {}", self.status))
                            .color(status_color)
                            .size(13.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // 1. 最右侧：退出按钮
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("退出").color(TEXT_COLOR).size(13.0),
                                )
                                .fill(Color32::from_rgb(233, 236, 240))
                                .corner_radius(6.0),
                            )
                            .clicked()
                        {
                            do_exit = true;
                        }

                        // 2. 退出按钮的左侧：设置按钮
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("⚙️ 设置").color(TEXT_COLOR).size(13.0),
                                )
                                .fill(Color32::from_rgb(233, 236, 240))
                                .corner_radius(6.0),
                            )
                            .clicked()
                        {
                            self.show_settings = true; // ✅ 点击打开弹窗
                        }
                    });
                });

                ui.add_space(12.0);

                // ---------- 中部对话区 ----------
                let input_area_height = 160.0;
                let scroll_height = (ui.available_height() - input_area_height).max(140.0);

                egui::Frame::new()
                    .fill(CARD_BG)
                    .stroke(egui::Stroke::new(1.0, BORDER_COLOR))
                    .corner_radius(10.0)
                    .inner_margin(Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(scroll_height)
                            .auto_shrink([false, false])
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                // ✅ 把 md_cache 从 self 中临时取出，避免借用冲突
                                let mut md_cache = std::mem::take(&mut self.md_cache);

                                for (idx, item) in self.chat.iter().enumerate() {
                                    match item {
                                        ChatItem::User(text) => {
                                            bubble(ui, USER_BG, true, |ui| {
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(text)
                                                            .color(Color32::WHITE)
                                                            .size(14.0),
                                                    )
                                                    .wrap(),
                                                );
                                            });
                                            ui.add_space(8.0);
                                        }
                                        ChatItem::Assistant(text) => {
                                            // ✅ 换成 Markdown 渲染
                                            bubble_md(
                                                ui,
                                                ASSIST_BG,
                                                false,
                                                &mut md_cache,
                                                text,
                                                idx,
                                            );
                                            ui.add_space(8.0);
                                        }
                                        ChatItem::ToolCall { name, args, status } => {
                                            let (icon, label, color) = match status {
                                                ToolStatus::Running => (
                                                    "⏳",
                                                    "正在调用",
                                                    Color32::from_rgb(180, 130, 20),
                                                ),
                                                ToolStatus::Done(_) => {
                                                    ("✓", "已完成", Color32::from_rgb(35, 130, 75))
                                                }
                                            };
                                            bubble(ui, TOOL_BG, false, |ui| {
                                                // ✅ 加上这行 push_id，用 idx 隔离整个工具面板
                                                ui.push_id(idx, |ui| {
                                                    let header =
                                                        format!("{}  {}  {}", icon, label, name);
                                                    egui::CollapsingHeader::new(
                                                        RichText::new(header)
                                                            .color(color)
                                                            .strong()
                                                            .size(13.5),
                                                    )
                                                    .default_open(false)
                                                    // ✅ 这里的 id_salt 甚至可以删掉，因为外层已经有唯一隔离了
                                                    .show(ui, |ui| {
                                                        ui.add_space(4.0);
                                                        ui.label(
                                                            RichText::new("参数")
                                                                .color(SUBTLE_TEXT)
                                                                .size(12.0),
                                                        );
                                                        ui.code(args);
                                                        ui.add_space(6.0);
                                                        match status {
                                                            ToolStatus::Running => {
                                                                ui.horizontal(|ui| {
                                                                    ui.spinner();
                                                                    ui.label(
                                                                        RichText::new("执行中...")
                                                                            .color(SUBTLE_TEXT),
                                                                    );
                                                                });
                                                            }
                                                            ToolStatus::Done(r) => {
                                                                let preview =
                                                                    if r.chars().count() > 800 {
                                                                        let mut s: String = r
                                                                            .chars()
                                                                            .take(800)
                                                                            .collect();
                                                                        s.push_str("...");
                                                                        s
                                                                    } else {
                                                                        r.clone()
                                                                    };
                                                                ui.label(
                                                                    RichText::new("结果")
                                                                        .color(SUBTLE_TEXT)
                                                                        .size(12.0),
                                                                );
                                                                ui.code(preview);
                                                            }
                                                        }
                                                    });
                                                }); // ✅ push_id 的结束
                                            });
                                            ui.add_space(8.0);
                                        }
                                        ChatItem::Error(err) => {
                                            bubble(ui, ERROR_BG, false, |ui| {
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(format!("错误：{}", err))
                                                            .color(Color32::from_rgb(180, 40, 40))
                                                            .size(14.0),
                                                    )
                                                    .wrap(),
                                                );
                                            });
                                            ui.add_space(8.0);
                                        }
                                        ChatItem::Status(s) => {
                                            ui.label(
                                                RichText::new(format!("· {}", s))
                                                    .color(SUBTLE_TEXT)
                                                    .italics()
                                                    .size(12.0),
                                            );
                                            ui.add_space(4.0);
                                        }
                                    }
                                }

                                // ✅ 流式正在生成的临时气泡
                                if let Some(text) = &self.streaming_text {
                                    let stream_idx = self.chat.len(); // ✅
                                    if text.is_empty() {
                                        bubble(ui, ASSIST_BG, false, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.spinner();
                                                ui.label(
                                                    RichText::new("正在生成...").color(SUBTLE_TEXT),
                                                );
                                            });
                                        });
                                    } else {
                                        bubble_md(
                                            ui,
                                            ASSIST_BG,
                                            false,
                                            &mut md_cache,
                                            text,
                                            stream_idx,
                                        );
                                    }
                                    ui.add_space(8.0);
                                }

                                // ✅ 记得把 md_cache 还回去（原本已有）
                                self.md_cache = md_cache;
                            });
                    });

                ui.add_space(14.0);

                // ---------- 底部输入区 ----------
                let enabled = self.mcp_ready && !self.is_waiting && has_api_key;

                let hint = if !has_api_key {
                    "⚠️ 请先点击右上角【⚙️ 设置】填写 API Key"
                } else if self.is_waiting {
                    "等待回复..."
                } else if !self.mcp_ready {
                    "MCP 连接中..."
                } else {
                    "输入消息，Enter 发送，Shift+Enter 换行"
                };

                egui::Frame::new()
                    .fill(CARD_BG)
                    .stroke(egui::Stroke::new(1.0, BORDER_COLOR))
                    .corner_radius(10.0)
                    .inner_margin(Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let btn_size = egui::vec2(84.0, 66.0);
                            let spacing = ui.spacing().item_spacing.x;
                            let text_width =
                                (ui.available_width() - btn_size.x - spacing - 8.0).max(160.0);

                            let response = ui.add_enabled(
                                enabled,
                                egui::TextEdit::multiline(&mut self.input)
                                    .desired_width(text_width)
                                    .desired_rows(3)
                                    .frame(egui::Frame::new().fill(INPUT_BG))
                                    .hint_text(RichText::new(hint).color(SUBTLE_TEXT))
                                    .text_color(TEXT_COLOR),
                            );

                            if enabled && response.has_focus() {
                                let pressed_enter = ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                                });
                                if pressed_enter {
                                    do_send = true;
                                }
                            }

                            let btn_fill = if self.is_waiting {
                                Color32::from_rgb(220, 80, 80) // 等待时红色 = 停止
                            } else if enabled {
                                USER_BG
                            } else {
                                Color32::from_rgb(200, 205, 212)
                            };

                            let btn_label = if self.is_waiting { "停止" } else { "发送" };

                            let btn = egui::Button::new(
                                RichText::new(btn_label)
                                    .color(Color32::WHITE)
                                    .size(15.0)
                                    .strong(),
                            )
                            .fill(btn_fill)
                            .corner_radius(8.0)
                            .min_size(btn_size);

                            // 等待中：点击停止；未等待：点击发送（需要 enabled）
                            let btn_enabled = self.is_waiting || enabled;
                            if ui.add_enabled(btn_enabled, btn).clicked() {
                                if self.is_waiting {
                                    // ✅ 触发取消
                                    self.cancel_flag.store(true, Ordering::SeqCst);
                                    self.status = "正在停止...".into();
                                } else {
                                    do_send = true;
                                }
                            }
                        });
                    });
            });

        // ---------- 设置弹窗 ----------
        if self.show_settings {
            let mut close_dialog = false;

            let window_frame = egui::Frame::new()
                .fill(CARD_BG)
                .corner_radius(12.0)
                .inner_margin(Margin::symmetric(28, 24))
                .stroke(egui::Stroke::new(1.0, BORDER_COLOR))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 16,
                    spread: 0,
                    color: Color32::from_black_alpha(40),
                });

            // ✅ 使用 egui::Modal 代替 Window
            let modal = egui::Modal::new(egui::Id::unique("settings_modal"))
                .frame(window_frame)
                .show(ui.ctx(), |ui| {
                    ui.set_width(600.0); // ✅ 强制弹窗宽度

                    // 自定义头部
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("⚙️ 系统设置")
                                .strong()
                                .size(18.0)
                                .color(TEXT_COLOR),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new("✕").frame(false)).clicked() {
                                close_dialog = true;
                            }
                        });
                    });
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(16.0);

                    // 定义输入框样式
                    // let input_frame = egui::Frame::new()
                    //     .fill(Color32::WHITE)
                    //     .stroke(egui::Stroke::new(1.0, BORDER_COLOR))
                    //     .corner_radius(6.0);

                    // 1. API Key
                    ui.label(
                        RichText::new("DeepSeek API Key")
                            .color(SUBTLE_TEXT)
                            .size(13.0),
                    );
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edit_cfg.deepseek_api_key)
                            .password(true)
                            .desired_width(f32::INFINITY)
                            .margin(egui::vec2(8.0, 8.0)),
                            // .frame(input_frame),
                    );
                    ui.add_space(16.0);

                    // 2. API URL
                    ui.label(RichText::new("API URL").color(SUBTLE_TEXT).size(13.0));
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edit_cfg.deepseek_url)
                            .desired_width(f32::INFINITY)
                            .margin(egui::vec2(8.0, 8.0)),
                            // .frame(input_frame),
                    );
                    ui.add_space(16.0);

                    // 3. 模型名
                    ui.label(RichText::new("模型名").color(SUBTLE_TEXT).size(13.0));
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edit_cfg.model)
                            .desired_width(f32::INFINITY)
                            .margin(egui::vec2(8.0, 8.0)),
                            // .frame(input_frame),
                    );
                    ui.add_space(16.0);

                    // 4. DBX 路径
                    ui.label(RichText::new("DBX MCP 路径").color(SUBTLE_TEXT).size(13.0));
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edit_cfg.dbx_exe)
                            .desired_width(f32::INFINITY)
                            .margin(egui::vec2(8.0, 8.0)),
                            // .frame(input_frame),
                    );
                    ui.add_space(16.0);

                    // 5. System Prompt
                    ui.label(
                        RichText::new("System Prompt / 角色设定")
                            .color(SUBTLE_TEXT)
                            .size(13.0),
                    );
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .id_salt("system_prompt_scroll")
                        .max_height(120.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.edit_cfg.system_prompt)
                                    .desired_width(f32::INFINITY)
                                    .margin(egui::vec2(8.0, 8.0)),
                                    // .frame(input_frame),
                            );
                        });

                    ui.add_space(24.0);

                    // 保存按钮
                    ui.vertical_centered(|ui| {
                        let btn = egui::Button::new(
                            RichText::new("保存并应用")
                                .color(Color32::WHITE)
                                .size(16.0)
                                .strong(),
                        )
                        .fill(USER_BG)
                        .corner_radius(8.0)
                        .min_size(egui::vec2(200.0, 42.0));

                        if ui.add(btn).clicked() {
                            match self.edit_cfg.save() {
                                Ok(_) => {
                                    self.active_cfg = self.edit_cfg.clone();
                                    let _ = self
                                        .cmd_tx
                                        .send(AgentCommand::ReloadConfig(self.edit_cfg.clone()));
                                    close_dialog = true;
                                }
                                Err(e) => {
                                    eprintln!("保存配置失败: {}", e);
                                }
                            }
                        }
                    });
                });

            // ✅ 处理关闭逻辑
            // modal.should_close() 处理点击遮罩层或按 Esc 键
            if close_dialog || modal.should_close() {
                self.show_settings = false;
            }
        }

        if do_send {
            self.send_current_input();
        }
        if do_exit {
            let _ = self.cmd_tx.send(AgentCommand::Shutdown);
            std::process::exit(0);
        }
    }
}

// ---------------- 风格 / 字体 ----------------

fn setup_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = PAGE_BG;
    visuals.window_fill = CARD_BG;

    // ✅ 让输入框的底色变成非常浅的灰色（这样和白底弹窗区分开）
    visuals.extreme_bg_color = Color32::from_rgb(248, 249, 250);
    visuals.override_text_color = Some(TEXT_COLOR);

    // ✅ 给所有控件强制加上浅灰边框
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, Color32::from_rgb(210, 215, 220));
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, USER_BG);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(2.0, USER_BG);

    // 干掉原生 Window 标题栏的深色背景
    visuals.widgets.noninteractive.bg_fill = CARD_BG;

    visuals.window_corner_radius = 12.0.into();
    visuals.menu_corner_radius = 8.0.into();

    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(14.0, 6.0);
        style.spacing.scroll.bar_width = 6.0;
    });
}

fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "my_font".to_owned(),
        egui::FontData::from_static(include_bytes!("../fonts/default.ttf")).into(),
    );
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .insert(0, "my_font".to_owned());
    fonts
        .families
        .get_mut(&egui::FontFamily::Monospace)
        .unwrap()
        .push("my_font".to_owned());
    ctx.set_fonts(fonts);
}

// ---------------- main ----------------

fn main() -> eframe::Result<()> {
    let cfg = Config::load().expect("加载 config.json 失败");
    let ui_cfg = cfg.clone(); // ✅ 克隆一份给 UI

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<AgentCommand>();
    let (event_tx, event_rx) = std::sync::mpsc::channel::<AgentEvent>();

    // ✅ 创建共享取消标志
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_for_agent = cancel_flag.clone();
    let cancel_flag_for_ui = cancel_flag.clone();

    let rt = tokio::runtime::Runtime::new().expect("创建 tokio runtime 失败");
    std::thread::spawn(move || {
        rt.block_on(agent::agent_loop(
            cfg,
            cmd_rx,
            event_tx,
            cancel_flag_for_agent,
        ));
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 720.0])
            .with_min_inner_size([560.0, 480.0])
            .with_title("my first Agent"),
        ..Default::default()
    };

    eframe::run_native(
        "my first Agent",
        options,
        Box::new(|cc| {
            setup_style(&cc.egui_ctx);
            setup_custom_fonts(&cc.egui_ctx);
            Ok(Box::new(AgentApp::new(
                cmd_tx,
                event_rx,
                cancel_flag_for_ui,
                ui_cfg,
            )))
        }),
    )
}
