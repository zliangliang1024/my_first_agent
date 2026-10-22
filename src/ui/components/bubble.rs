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
        egui::Frame::new().fill(bg).corner_radius(10.0).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
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
        egui::Frame::new().fill(bg).corner_radius(10.0).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_max_width(max_w);
            ui.set_min_width(40.0);
            ui.push_id(bubble_index, |ui| {
                egui::ScrollArea::horizontal().id_salt(format!("md_scroll_{}", bubble_index)).show(ui, |ui| {
                    CommonMarkViewer::new().show(ui, md_cache, text);
                });
            });
        });
    });
}