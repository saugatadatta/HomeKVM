//! Icons for the tray (with a status dot) and the window.

use crate::icon_art;
use crate::state::Status;

/// Tray icon size; Windows scales it for the display.
pub const SIZE: u32 = 32;

/// Tray icon with the status dot.
pub fn rgba(status: Status) -> Vec<u8> {
    icon_art::draw(SIZE as usize, Some(status.rgb()))
}

/// Window and taskbar icon: the plain keyboard, no dot.
pub fn window_icon() -> eframe::egui::IconData {
    let n = 64;
    eframe::egui::IconData { rgba: icon_art::draw(n, None), width: n as u32, height: n as u32 }
}
