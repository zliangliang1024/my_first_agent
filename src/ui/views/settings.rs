use eframe::egui::{self, Color32, Margin, RichText};
use crate::agent::AgentCommand;
use crate::config::Config;
use crate::ui::app::AgentApp;
use crate::ui::style::*;

impl AgentApp {
    pub fn draw_settings_modal(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }

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

        let modal = egui::Modal::new(egui::Id::new("settings_modal"))
            .frame(window_frame)
            .show(ctx, |ui| {
                ui.set_width(600.0);

                // ========== 头部 ==========
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚙️ 系统设置").strong().size(18.0).color(TEXT_COLOR));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("✕").frame(false)).clicked() {
                            close_dialog = true;
                        }
                    });
                });
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(16.0);

                // ✅ 计算滚动区域的最大高度，确保不会把底部按钮挤出屏幕
                let max_scroll_height = ctx.content_rect().height() * 0.65;

                // ========== 滚动主体区域 ==========
                egui::ScrollArea::vertical()
                    .max_height(max_scroll_height)
                    .show(ui, |ui| {
                        ui.label(RichText::new("DeepSeek API Key").color(SUBTLE_TEXT).size(13.0));
                        ui.add_space(4.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edit_cfg.deepseek_api_key)
                                .password(true)
                                .desired_width(f32::INFINITY)
                                .margin(egui::vec2(8.0, 8.0)),
                        );
                        ui.add_space(16.0);

                        ui.label(RichText::new("API URL").color(SUBTLE_TEXT).size(13.0));
                        ui.add_space(4.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edit_cfg.deepseek_url)
                                .desired_width(f32::INFINITY)
                                .margin(egui::vec2(8.0, 8.0)),
                        );
                        ui.add_space(16.0);

                        ui.label(RichText::new("模型名").color(SUBTLE_TEXT).size(13.0));
                        ui.add_space(4.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edit_cfg.model)
                                .desired_width(f32::INFINITY)
                                .margin(egui::vec2(8.0, 8.0)),
                        );
                        ui.add_space(16.0);

                        ui.label(RichText::new("MCP 执行路径 (每行一个)").color(SUBTLE_TEXT).size(13.0));
                        ui.add_space(4.0);
                        ui.add(
                            egui::TextEdit::multiline(&mut self.edit_mcp_exes_text)
                                .desired_width(f32::INFINITY)
                                .desired_rows(3)
                                .margin(egui::vec2(8.0, 8.0)),
                        );
                        ui.add_space(16.0);

                        ui.label(RichText::new("System Prompt / 角色设定").color(SUBTLE_TEXT).size(13.0));
                        ui.add_space(4.0);
                        ui.add(
                            egui::TextEdit::multiline(&mut self.edit_cfg.system_prompt)
                                .desired_width(f32::INFINITY)
                                .desired_rows(5)
                                .margin(egui::vec2(8.0, 8.0)),
                        );
                    });

                ui.add_space(10.0);
                ui.separator(); // ✅ 底部按钮和内容之间的分割线
                ui.add_space(10.0);

                // ========== 底部按钮区 (左右分布) ==========
                ui.horizontal(|ui| {
                    // 左侧：恢复默认
                    let reset_btn = egui::Button::new(RichText::new("恢复默认").color(TEXT_COLOR).size(14.0))
                        .fill(Color32::from_rgb(233, 236, 240))
                        .corner_radius(8.0)
                        .min_size(egui::vec2(100.0, 38.0));

                    if ui.add(reset_btn).clicked() {
                        self.edit_cfg = Config::default();
                        // ✅ 重置时也要同步更新多行文本状态
                        self.edit_mcp_exes_text = self.edit_cfg.mcp_exes.join("\n");
                    }

                    // 右侧：保存并应用
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let save_btn = egui::Button::new(RichText::new("保存并应用").color(Color32::WHITE).size(15.0).strong())
                            .fill(USER_BG)
                            .corner_radius(8.0)
                            .min_size(egui::vec2(120.0, 38.0));

                        if ui.add(save_btn).clicked() {
                            // ✅ 把多行文本解析成 Vec 并保存
                            self.edit_cfg.mcp_exes = self.edit_mcp_exes_text
                                .lines()
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();

                            match self.edit_cfg.save() {
                                Ok(_) => {
                                    self.active_cfg = self.edit_cfg.clone();
                                    let _ = self.cmd_tx.send(AgentCommand::ReloadConfig(self.edit_cfg.clone()));
                                    close_dialog = true;
                                }
                                Err(e) => eprintln!("保存配置失败: {}", e),
                            }
                        }
                    });
                });
            });

        if close_dialog || modal.should_close() {
            self.show_settings = false;
        }
    }
}