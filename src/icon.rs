//! The HomeKVM icon (a small keyboard), drawn in code with a status dot.

use crate::state::Status;

pub const SIZE: u32 = 32;

const BODY: [u8; 3] = [62, 84, 120];
const KEY: [u8; 3] = [235, 240, 248];

/// RGBA pixels for the icon with a status dot in the bottom-right corner.
pub fn rgba(status: Status) -> Vec<u8> {
    let n = SIZE as usize;
    let mut px = vec![0u8; n * n * 4];

    // Keyboard body: rounded rectangle.
    for y in 0..n {
        for x in 0..n {
            let a = rounded_rect_coverage(x as f32 + 0.5, y as f32 + 0.5, 1.0, 7.0, 31.0, 25.0, 3.5);
            if a > 0.0 {
                set(&mut px, x, y, BODY, a);
            }
        }
    }
    // Keys: two rows and a space bar.
    for row in [10, 14] {
        for col in (4..=24).step_by(4) {
            fill(&mut px, col, row, 3, 2, KEY);
        }
    }
    fill(&mut px, 8, 19, 16, 2, KEY);

    // Status dot, with a transparent ring cut out around it so it reads on any taskbar.
    let (cx, cy, r) = (24.5f32, 24.5f32, 6.0f32);
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let ring = coverage(r + 1.75 - d);
            let dot = coverage(r - d);
            let i = (y * n + x) * 4;
            if ring > 0.0 {
                px[i + 3] = (px[i + 3] as f32 * (1.0 - ring)) as u8;
            }
            if dot > 0.0 {
                set(&mut px, x, y, status.rgb(), dot);
            }
        }
    }
    px
}

pub fn window_icon() -> eframe::egui::IconData {
    eframe::egui::IconData { rgba: rgba(Status::Connected), width: SIZE, height: SIZE }
}

fn coverage(edge: f32) -> f32 {
    (edge + 0.5).clamp(0.0, 1.0)
}

fn rounded_rect_coverage(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (hw, hh) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let dx = (px - cx).abs() - (hw - r);
    let dy = (py - cy).abs() - (hh - r);
    let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
    let inside = dx.max(dy).min(0.0);
    coverage(-(outside + inside - r))
}

fn set(px: &mut [u8], x: usize, y: usize, rgb: [u8; 3], a: f32) {
    let i = (y * SIZE as usize + x) * 4;
    let old_a = px[i + 3] as f32 / 255.0;
    let out_a = a + old_a * (1.0 - a);
    for c in 0..3 {
        let blended = (rgb[c] as f32 * a + px[i + c] as f32 * old_a * (1.0 - a)) / out_a.max(1e-6);
        px[i + c] = blended.round() as u8;
    }
    px[i + 3] = (out_a * 255.0).round() as u8;
}

fn fill(px: &mut [u8], x: usize, y: usize, w: usize, h: usize, rgb: [u8; 3]) {
    for yy in y..y + h {
        for xx in x..x + w {
            set(px, xx, yy, rgb, 1.0);
        }
    }
}
