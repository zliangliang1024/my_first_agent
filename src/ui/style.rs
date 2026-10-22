use eframe::egui::{self, Color32};

pub const USER_BG: Color32 = Color32::from_rgb(37, 99, 235);
pub const ASSIST_BG: Color32 = Color32::from_rgb(238, 240, 244);
pub const TOOL_BG: Color32 = Color32::from_rgb(255, 248, 225);
pub const ERROR_BG: Color32 = Color32::from_rgb(253, 236, 234);
pub const TEXT_COLOR: Color32 = Color32::from_rgb(31, 35, 40);
pub const SUBTLE_TEXT: Color32 = Color32::from_rgb(110, 118, 129);
pub const PAGE_BG: Color32 = Color32::from_rgb(245, 247, 250);
pub const CARD_BG: Color32 = Color32::from_rgb(255, 255, 255);
pub const BORDER_COLOR: Color32 = Color32::from_rgb(220, 224, 230);

pub fn setup_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = PAGE_BG;
    visuals.window_fill = CARD_BG;
    visuals.extreme_bg_color = Color32::from_rgb(248, 250, 252); 
    visuals.override_text_color = Some(TEXT_COLOR);

    visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, Color32::from_rgb(200, 205, 210));
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, USER_BG);
    visuals.widgets.active.bg_fill = Color32::WHITE;
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

pub fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "my_font".to_owned(),
        egui::FontData::from_static(include_bytes!("../../fonts/default.ttf")).into(),
    );
    fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().insert(0, "my_font".to_owned());
    fonts.families.get_mut(&egui::FontFamily::Monospace).unwrap().push("my_font".to_owned());
    ctx.set_fonts(fonts);
}