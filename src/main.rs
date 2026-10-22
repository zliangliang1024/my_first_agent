mod agent;
mod config;
mod llm;
mod storage;
mod ui; // ✅ 加上这行

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use config::Config;
use eframe::egui;
use ui::app::AgentApp;

fn main() -> eframe::Result<()> {
    let cfg = Config::load().expect("加载配置失败");
    let ui_cfg = cfg.clone();
    

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
    let (event_tx, event_rx) = std::sync::mpsc::channel();

    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_for_agent = cancel_flag.clone();
    let cancel_flag_for_ui = cancel_flag.clone();

    let rt = tokio::runtime::Runtime::new().expect("创建 tokio runtime 失败");
    std::thread::spawn(move || {
        rt.block_on(agent::loop_runner::agent_loop(
            cfg,
            cmd_rx,
            event_tx,
            cancel_flag_for_agent,
        ));
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 720.0])
            .with_min_inner_size([420.0, 360.0])
            .with_title("my first Agent"),
        ..Default::default()
    };

    eframe::run_native(
        "my first Agent",
        options,
        Box::new(move |cc| {
            ui::style::setup_style(&cc.egui_ctx);
            ui::style::setup_custom_fonts(&cc.egui_ctx);
            Ok(Box::new(AgentApp::new(
                cmd_tx,
                event_rx,
                cancel_flag_for_ui,
                ui_cfg,
            )))
        }),
    )
}
