// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("HomeKVM is a Windows app.");

mod autostart;
mod clipboard;
mod config;
mod devices;
mod icon;
mod icon_art;
mod identity;
mod input;
mod menu_icons;
mod net;
mod peers;
mod profile;
mod proto;
mod state;
mod tls;
mod tray;
mod vfiles;
mod ui;
mod update;
mod win;

use std::sync::Arc;

pub const WINDOW_WIDTH: f32 = 440.0;

fn main() {
    // One copy per user session. Starting it again opens the running copy's window.
    // Right after an update, the previous version is still closing: wait for it.
    let after_update = std::env::args().any(|a| a == update::AFTER_UPDATE);
    let mut first = win::acquire_single_instance();
    for _ in 0..(if after_update { 100 } else { 0 }) {
        if first {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        first = win::acquire_single_instance();
    }
    if !first {
        win::signal_running_instance();
        return;
    }

    let cfg = config::load();
    if let Err(e) = autostart::apply(cfg.start_at_login) {
        eprintln!("start at login: {e}");
    }

    let shared = Arc::new(state::Shared::default());
    shared.set_layout(&cfg.layout);
    let tray = tray::spawn(shared.clone());
    win::listen_for_show_requests();

    // Network changes redraw the window and the tray icon.
    let on_change: Arc<dyn Fn() + Send + Sync> = {
        let tray = tray.clone();
        Arc::new(move || {
            win::request_repaint();
            tray.refresh();
        })
    };
    let net = net::start(shared.clone(), cfg.device_name.clone(), on_change.clone());
    update::start(shared.clone(), on_change);
    // Sharing follows the settings from the start, even before the window opens.
    let attached = devices::scan();
    ui::apply_settings(&cfg, &attached);
    net.send(net::Cmd::SetSharing(ui::shares_input(&cfg, &attached)));

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(profile::title())
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
            Ok(Box::new(ui::App::new(cc, cfg, shared, tray, net)))
        }),
    );
    if let Err(e) = result {
        eprintln!("HomeKVM window failed: {e}");
    }
}
