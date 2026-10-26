use eframe::egui::{self, Color32, Margin};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

pub fn bubble(
    ui: &mut egui::Ui,
    bg: Color32,
    align_right: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
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
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.set_min_width(40.0);
                add_contents(ui);
            });
    });
}

pub fn bubble_md(
    ui: &mut egui::Ui,
    bg: Color32,
    align_right: bool,
    md_cache: &mut CommonMarkCache,
    text: &str,
    bubble_index: usize,
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

                // ✅ 使用 scope 单独调整 Markdown 内部的样式
                ui.scope(|ui| {
                    let style = ui.style_mut();
                    // 1. 增大复制按钮的内边距，让它不那么憋屈
                    style.spacing.button_padding = egui::vec2(8.0, 4.0);
                    // 2. 强制把 Small 字号调大（因为图标用的是 Small 字体）
                    style
                        .text_styles
                        .insert(egui::TextStyle::Small, egui::FontId::proportional(15.0));

                    ui.push_id(bubble_index, |ui| {
                        egui::ScrollArea::horizontal()
                            .id_salt(format!("md_scroll_{}", bubble_index))
                            .show(ui, |ui| {
                                CommonMarkViewer::new().show(ui, md_cache, text);
                            });
                    });
                });
            });
    });
}
