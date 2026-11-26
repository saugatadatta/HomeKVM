// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("HomeKVM currently supports Windows only; macOS and Linux come in a later phase.");

mod autostart;
mod config;
mod devices;
mod icon;
mod menu_icons;
mod state;
mod tray;
mod ui;
mod win;

use std::sync::Arc;

pub const WINDOW_WIDTH: f32 = 440.0;

fn main() {
    // One copy per user session. Starting it again opens the running copy's window.
    if !win::acquire_single_instance() {
        win::signal_running_instance();
        return;
    }

    let cfg = config::load();
    if let Err(e) = autostart::apply(cfg.start_at_login) {
        eprintln!("start at login: {e}");
    }

    let shared = Arc::new(state::Shared::default());
    let tray = tray::spawn(shared.clone());
    win::listen_for_show_requests();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("HomeKVM")
            // Fixed width, no resizing, no maximize. The height fits the content (ui.rs).
            .with_inner_size([WINDOW_WIDTH, 720.0])
            .with_resizable(false)
            .with_maximize_button(false)
            // Always start in the tray; the window opens from the tray icon.
            .with_visible(false)
            .with_icon(icon::window_icon()),
        ..Default::default()
    };

    let result = eframe::run_native(
        "HomeKVM",
        options,
        Box::new(move |cc| {
            win::register_main_window(cc);
            Ok(Box::new(ui::App::new(cc, cfg, shared, tray)))
        }),
    );
    if let Err(e) = result {
        eprintln!("HomeKVM window failed: {e}");
    }
}
