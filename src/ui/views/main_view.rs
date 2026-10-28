use crate::agent::AgentCommand;
use crate::ui::app::{AgentApp, ChatItem, ToolStatus};
use crate::ui::components::bubble::{bubble, bubble_md};
use crate::ui::style::*;
use chrono::{Datelike, Local, TimeZone};
use eframe::egui::{self, Color32, Margin, RichText};
use std::sync::atomic::Ordering;

impl AgentApp {
    pub fn draw_main_view(&mut self, ui: &mut egui::Ui, has_api_key: bool) -> (bool, bool) {
        let mut do_send = false;
        let mut do_exit = false;

        // ✅ 绘制左侧侧边栏
        egui::Panel::left("session_panel")
            .resizable(false)
            .default_size(220.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(240, 243, 248))
                    .inner_margin(Margin::symmetric(10, 10)),
            )
            .show(ui, |ui| {
                ui.heading(
                    RichText::new("会话列表")
                        .size(16.0)
                        .color(TEXT_COLOR)
                        .strong(),
                );
                ui.add_space(8.0);

                if ui
                    .add(
                        egui::Button::new(RichText::new("➕ 新建对话").size(14.0))
                            .min_size(egui::vec2(ui.available_width(), 32.0)),
                    )
                    .clicked()
                {
                    self.new_session();
                }
                ui.add_space(10.0);

                // 分组逻辑
                let now = chrono::Local::now();
                let today = now.date_naive();
                let yesterday = today.pred_opt().unwrap_or(today);
                let mut today_sessions = Vec::new();
                let mut yesterday_sessions = Vec::new();
                let mut older_sessions = Vec::new();

                for s in &self.sessions {
                    let ts_str = s.id.strip_prefix("session_").unwrap_or(&s.id);
                    if let Ok(ts) = ts_str.parse::<i64>() {
                        if let Some(dt) = chrono::Local.timestamp_millis_opt(ts).single() {
                            let date = dt.date_naive();
                            if date == today {
                                today_sessions.push(s.clone());
                            } else if date == yesterday {
                                yesterday_sessions.push(s.clone());
                            } else {
                                older_sessions.push(s.clone());
                            }
                        } else {
                            older_sessions.push(s.clone());
                        }
                    } else {
                        // 兼容旧数据或自定义 ID
                        older_sessions.push(s.clone());
                    }
                }

                // ✅ 新增：因为老会话先 push 进去，这里反转一下，让最新的在最上面
                today_sessions.reverse();
                yesterday_sessions.reverse();
                older_sessions.reverse();

                let mut render_group = |ui: &mut egui::Ui,
                                        title: &str,
                                        list: &[crate::storage::SessionMeta],
                                        app: &mut AgentApp| {
                    if list.is_empty() {
                        return;
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new(title).color(SUBTLE_TEXT).size(12.0).strong());
                    ui.add_space(4.0);

                    for s in list {
                        if app.renaming_id.as_deref() == Some(&s.id) {
                            // ✅ 修复重命名输入框的布局 (固定宽度)
                            ui.horizontal(|ui| {
                                let response = ui.add_sized(
                                    [140.0, 24.0], // 固定 140 宽，防止撑破侧边栏
                                    egui::TextEdit::singleline(&mut app.rename_buffer),
                                );

                                if response.lost_focus()
                                    || ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    app.commit_rename();
                                }
                                if ui.button("✔").clicked() {
                                    app.commit_rename();
                                }
                            });
                        } else {
                            let is_selected = s.id == app.current_session_id;
                            let text_color = if is_selected { USER_BG } else { TEXT_COLOR };

                            // ✅ 使用 available_width 让按钮自适应
                            let btn = egui::Button::new(
                                RichText::new(&s.title).color(text_color).size(14.0),
                            )
                            .fill(if is_selected {
                                Color32::WHITE
                            } else {
                                Color32::TRANSPARENT
                            })
                            .min_size(egui::vec2(ui.available_width(), 30.0))
                            .wrap();

                            let response = ui.add(btn);
                            if response.clicked() {
                                app.switch_session(&s.id);
                            }

                            // 右键菜单
                            response.context_menu(|ui| {
                                if ui.button("✏️ 重命名").clicked() {
                                    app.renaming_id = Some(s.id.clone());
                                    app.rename_buffer = s.title.clone();
                                    ui.close();
                                }
                                if ui.button("🗑️ 删除").clicked() {
                                    app.delete_session(&s.id);
                                    ui.close();
                                }
                            });
                        }
                    }
                };

                // 渲染列表
                egui::ScrollArea::vertical().show(ui, |ui| {
                    render_group(ui, "今天", &today_sessions, self);
                    render_group(ui, "昨天", &yesterday_sessions, self);
                    render_group(ui, "更早", &older_sessions, self);
                });
            });

        // ✅ 第一行：标题 + 右侧操作按钮
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            ui.heading(
                RichText::new("Rust egui Agent")
                    .color(TEXT_COLOR)
                    .strong()
                    .size(18.0),
            );

            // 右侧按钮组（从右向左排版）
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // 退出按钮
                if ui
                    .add(
                        egui::Button::new(RichText::new("退出").color(TEXT_COLOR).size(13.0))
                            .fill(Color32::from_rgb(233, 236, 240))
                            .corner_radius(6.0),
                    )
                    .clicked()
                {
                    do_exit = true;
                }
                // 设置按钮
                if ui
                    .add(
                        egui::Button::new(RichText::new("⚙️ 设置").color(TEXT_COLOR).size(13.0))
                            .fill(Color32::from_rgb(233, 236, 240))
                            .corner_radius(6.0),
                    )
                    .clicked()
                {
                    // ✅ 打开弹窗时，把 Vec 转成带换行符的 String
                    self.edit_mcp_exes_text = self.edit_cfg.mcp_exes.join("\n");
                    self.show_settings = true;
                }
                // ✅ 新增：日志按钮
                if ui
                    .add(
                        egui::Button::new(RichText::new("📜 日志").color(TEXT_COLOR).size(13.0))
                            .fill(Color32::from_rgb(233, 236, 240))
                            .corner_radius(6.0),
                    )
                    .clicked()
                {
                    self.show_audit_log = true;
                }
                // 清空按钮
                if ui
                    .add(
                        egui::Button::new(RichText::new("🗑️ 清空").color(TEXT_COLOR).size(13.0))
                            .fill(Color32::from_rgb(233, 236, 240))
                            .corner_radius(6.0),
                    )
                    .clicked()
                {
                    self.chat.clear();
                    self.total_tokens = 0; // ✅ 清零
                    crate::storage::save_json("ui_history.json", &self.chat);
                    crate::storage::save_json("total_tokens.json", &self.total_tokens); // ✅ 保存 0
                    let _ = self.cmd_tx.send(AgentCommand::ClearHistory);
                }
            });
        });

        // ✅ 第二行：连接状态 + 右侧 Token 统计
        ui.horizontal(|ui| {
            // 左侧：状态文字
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

            // 右侧：Token 徽章（从右向左排版，靠右显示）
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgb(240, 243, 248)) // 浅灰蓝色背景
                    .corner_radius(10.0)
                    .inner_margin(Margin::symmetric(8, 2))
                    .show(ui, |ui| {
                        // 格式：⚡ 本轮 1240 / 累计 8544
                        ui.label(
                            RichText::new(format!(
                                "⚡ 本轮 {} / 累计 {}",
                                self.last_prompt_tokens, self.total_tokens
                            ))
                            .color(Color32::from_rgb(80, 90, 105))
                            .size(12.0)
                            .strong(),
                        );
                    });
            });
        });

        ui.add_space(12.0);

        // 中部对话区
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
                                    bubble_md(ui, ASSIST_BG, false, &mut md_cache, text, idx);
                                    ui.add_space(8.0);
                                }
                                ChatItem::ToolCall {
                                    id,
                                    name,
                                    args,
                                    status,
                                } => {
                                    let (icon, label, color) = match status {
                                        ToolStatus::Pending => {
                                            ("⏸️", "等待审批", Color32::from_rgb(180, 130, 20))
                                        } // ✅ 新增
                                        ToolStatus::Running => {
                                            ("⏳", "正在调用", Color32::from_rgb(180, 130, 20))
                                        }
                                        ToolStatus::Done(_) => {
                                            ("✓", "已完成", Color32::from_rgb(35, 130, 75))
                                        }
                                    };
                                    bubble(ui, TOOL_BG, false, |ui| {
                                        ui.push_id(idx, |ui| {
                                            let header = format!("{}  {}  {}", icon, label, name);
                                            egui::CollapsingHeader::new(
                                                RichText::new(header)
                                                    .color(color)
                                                    .strong()
                                                    .size(13.5),
                                            )
                                            .default_open(false)
                                            .show(
                                                ui,
                                                |ui| {
                                                    ui.add_space(4.0);
                                                    ui.label(
                                                        RichText::new("参数")
                                                            .color(SUBTLE_TEXT)
                                                            .size(12.0),
                                                    );
                                                    ui.code(args);
                                                    ui.add_space(6.0);
                                                    match status {
                                                        ToolStatus::Pending => {
                                                            ui.label(
                                                                RichText::new("等待用户审批中...")
                                                                    .color(Color32::from_rgb(
                                                                        180, 130, 20,
                                                                    )),
                                                            );
                                                        }
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
                                                            let preview = if r.chars().count() > 800
                                                            {
                                                                let mut s: String =
                                                                    r.chars().take(800).collect();
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
                                                },
                                            );
                                        });
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

                        if let Some(text) = &self.streaming_text {
                            let stream_idx = self.chat.len();
                            if text.is_empty() {
                                bubble(ui, ASSIST_BG, false, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(RichText::new("正在生成...").color(SUBTLE_TEXT));
                                    });
                                });
                            } else {
                                bubble_md(ui, ASSIST_BG, false, &mut md_cache, text, stream_idx);
                            }
                            ui.add_space(8.0);
                        }
                        self.md_cache = md_cache;
                    });
            });

        ui.add_space(14.0);

        // 底部输入区
        // ✅ 增加 pending_tool 判断：如果有工具在等待审批，就锁死输入
        let is_pending_approval = self.pending_tool.is_some();
        let enabled = self.mcp_ready && !self.is_waiting && has_api_key && !is_pending_approval;

        let hint = if !has_api_key {
            "⚠️ 请先点击右上角【⚙️ 设置】填写 API Key"
        } else if is_pending_approval {
            "⚠️ 请先处理上方的安全审批请求"
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
                    let text_width = (ui.available_width() - btn_size.x - spacing - 8.0).max(160.0);

                    let response = ui.add_enabled(
                        enabled,
                        egui::TextEdit::multiline(&mut self.input)
                            .desired_width(text_width)
                            .desired_rows(3)
                            .hint_text(hint),
                    );

                    if enabled && response.has_focus() {
                        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                        {
                            do_send = true;
                        }
                    }

                    let btn_fill = if is_pending_approval {
                        Color32::from_rgb(200, 205, 212)
                    } else if self.is_waiting {
                        Color32::from_rgb(220, 80, 80)
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

                    let btn_enabled = self.is_waiting || enabled;
                    if ui.add_enabled(btn_enabled, btn).clicked() {
                        if self.is_waiting {
                            self.cancel_flag.store(true, Ordering::SeqCst);
                            self.status = "正在停止...".into();
                        } else {
                            do_send = true;
                        }
                    }
                });
            });
        // ✅ 审批弹窗（修复借用冲突版本）
        if let Some((name, args)) = &self.pending_tool {
            // 1. 先把数据克隆出来，打破对 self 的借用
            let name_cloned = name.clone();
            let args_cloned = args.clone();

            // 2. 用一个临时变量记录用户的操作
            let mut action: Option<bool> = None;

            let modal = egui::Modal::new(egui::Id::unique("tool_approval_modal"))
                .frame(
                    egui::Frame::new()
                        .fill(Color32::WHITE)
                        .corner_radius(12.0)
                        .inner_margin(Margin::symmetric(24, 20))
                        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 50, 50)))
                        .shadow(egui::epaint::Shadow {
                            offset: [0, 8],
                            blur: 16,
                            spread: 0,
                            color: Color32::from_black_alpha(40),
                        }),
                )
                .show(ui.ctx(), |ui| {
                    ui.set_width(500.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("⚠️ 安全审批")
                                .strong()
                                .size(18.0)
                                .color(Color32::from_rgb(220, 50, 50)),
                        );
                    });
                    ui.add_space(10.0);
                    ui.label(RichText::new("Agent 请求调用以下工具：").color(TEXT_COLOR));
                    ui.code(&name_cloned); // ✅ 使用克隆出来的变量
                    ui.add_space(10.0);
                    ui.label(RichText::new("参数：").color(TEXT_COLOR));
                    ui.code(&args_cloned);
                    ui.add_space(24.0);

                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("❌ 拒绝").color(Color32::WHITE).strong(),
                                )
                                .fill(Color32::from_rgb(220, 80, 80))
                                .min_size(egui::vec2(100.0, 36.0)),
                            )
                            .clicked()
                        {
                            action = Some(false); // ✅ 只记录动作
                        }
                        ui.add_space(10.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("✅ 允许").color(Color32::WHITE).strong(),
                                )
                                .fill(USER_BG)
                                .min_size(egui::vec2(100.0, 36.0)),
                            )
                            .clicked()
                        {
                            action = Some(true); // ✅ 只记录动作
                        }
                    });
                });

            // 3. 闭包结束，借用释放。现在可以安全地修改 self 了
            if let Some(is_approved) = action {
                self.approve_tool(is_approved);
            } else if modal.should_close() {
                self.approve_tool(false); // 点外面等同拒绝
            }
        }

        // ✅ 日志查看弹窗
        if self.show_audit_log {
            let mut open = self.show_audit_log;
            egui::Window::new("📜 操作审计日志")
                .open(&mut open)
                .default_width(700.0)
                .default_height(500.0)
                .resizable(true)
                .show(ui.ctx(), |ui| {
                    let log_path = crate::storage::get_storage_path("audit.log");
                    if let Ok(content) = std::fs::read_to_string(log_path) {
                        egui::ScrollArea::vertical()
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                // ✅ 使用 Label 并设置为可选择，同时指定等宽字体
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(content).monospace(), // 等宽字体
                                    )
                                    .selectable(true) // ✅ 关键：允许鼠标选中复制
                                    .wrap(), // 自动换行
                                );
                            });
                    } else {
                        ui.label(
                            RichText::new("暂无日志记录，去跑几个工具试试吧！").color(SUBTLE_TEXT),
                        );
                    }
                });
            self.show_audit_log = open;
        }

        (do_send, do_exit)
    }
}
