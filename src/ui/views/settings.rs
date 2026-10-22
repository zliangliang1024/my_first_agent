use eframe::egui::{self, Color32, Margin, RichText};
use crate::ui::app::AgentApp;
use crate::agent::AgentCommand;
use crate::config::Config;
use crate::ui::style::*;

impl AgentApp {
    pub fn draw_settings_modal(&mut self, ctx: &egui::Context) {
        if !self.show_settings { return; }

        let mut close_dialog = false; 

        let window_frame = egui::Frame::new()
            .fill(CARD_BG).corner_radius(12.0).inner_margin(Margin::symmetric(28, 24))
            .stroke(egui::Stroke::new(1.0, BORDER_COLOR))
            .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 16, spread: 0, color: Color32::from_black_alpha(40) });

        let modal = egui::Modal::new(egui::Id::new("settings_modal")).frame(window_frame).show(ctx, |ui| {
            ui.set_width(600.0);
            
            ui.horizontal(|ui| {
                ui.label(RichText::new("⚙️ 系统设置").strong().size(18.0).color(TEXT_COLOR));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("✕").frame(false)).clicked() { close_dialog = true; }
                });
            });
            ui.add_space(10.0);
            ui.separator(); 
            ui.add_space(16.0);

            ui.label(RichText::new("DeepSeek API Key").color(SUBTLE_TEXT).size(13.0));
            ui.add_space(4.0);
            ui.add(egui::TextEdit::singleline(&mut self.edit_cfg.deepseek_api_key).password(true).desired_width(f32::INFINITY).margin(egui::vec2(8.0, 8.0)));
            ui.add_space(16.0);

            ui.label(RichText::new("API URL").color(SUBTLE_TEXT).size(13.0));
            ui.add_space(4.0);
            ui.add(egui::TextEdit::singleline(&mut self.edit_cfg.deepseek_url).desired_width(f32::INFINITY).margin(egui::vec2(8.0, 8.0)));
            ui.add_space(16.0);

            ui.label(RichText::new("模型名").color(SUBTLE_TEXT).size(13.0));
            ui.add_space(4.0);
            ui.add(egui::TextEdit::singleline(&mut self.edit_cfg.model).desired_width(f32::INFINITY).margin(egui::vec2(8.0, 8.0)));
            ui.add_space(16.0);

            ui.label(RichText::new("DBX MCP 路径").color(SUBTLE_TEXT).size(13.0));
            ui.add_space(4.0);
            ui.add(egui::TextEdit::singleline(&mut self.edit_cfg.dbx_exe).desired_width(f32::INFINITY).margin(egui::vec2(8.0, 8.0)));
            ui.add_space(16.0);

            ui.label(RichText::new("System Prompt / 角色设定").color(SUBTLE_TEXT).size(13.0));
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("system_prompt_scroll").max_height(120.0).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut self.edit_cfg.system_prompt).desired_width(f32::INFINITY).margin(egui::vec2(8.0, 8.0)));
            });

            ui.add_space(24.0);
            
            ui.vertical_centered(|ui| {
                ui.horizontal(|ui| {
                    let reset_btn = egui::Button::new(RichText::new("恢复默认").color(TEXT_COLOR).size(15.0)).fill(Color32::from_rgb(233, 236, 240)).corner_radius(8.0).min_size(egui::vec2(120.0, 42.0));
                    if ui.add(reset_btn).clicked() { self.edit_cfg = Config::default(); }

                    ui.add_space(20.0);

                    let btn = egui::Button::new(RichText::new("保存并应用").color(Color32::WHITE).size(16.0).strong()).fill(USER_BG).corner_radius(8.0).min_size(egui::vec2(200.0, 42.0)); 
                    if ui.add(btn).clicked() {
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

        if close_dialog || modal.should_close() { self.show_settings = false; }
    }
}