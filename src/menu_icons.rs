//! Small line icons for the tray menu, drawn in code at any size so they stay
//! sharp at every display scaling.

use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

#[derive(Clone, Copy)]
pub enum Glyph {
    Open,
    Pause,
    Resume,
    Home,
    Power,
    /// Status dot for each paired PC in the menu (used once pairing exists).
    #[allow(dead_code)]
    Dot([u8; 3]),
}

/// True when Windows apps use the dark theme.
pub fn dark_mode() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
        .map(|v| v == 0)
        .unwrap_or(false)
}

/// RGBA pixels, `n` by `n`, drawn in `ink` (dots use their own color).
pub fn rgba(glyph: Glyph, n: usize, ink: [u8; 3]) -> Vec<u8> {
    let s = n as f32 / 16.0;
    let w = 1.3 * s.max(1.0);
    let mut c = Canvas { n, px: vec![0; n * n * 4] };
    let p = |x: f32, y: f32| (x * s, y * s);

    match glyph {
        Glyph::Open => {
            c.draw(ink, |x, y| rrect(x, y, p(2.5, 3.0), p(13.5, 13.0), 1.5 * s).abs() - w / 2.0);
            c.draw(ink, |x, y| seg(x, y, p(8.0, 3.0), p(8.0, 13.0)) - w / 2.0);
        }
        Glyph::Pause => {
            c.draw(ink, |x, y| rrect(x, y, p(4.0, 3.0), p(7.0, 13.0), 1.0 * s).abs() - w / 2.0);
            c.draw(ink, |x, y| rrect(x, y, p(9.0, 3.0), p(12.0, 13.0), 1.0 * s).abs() - w / 2.0);
        }
        Glyph::Resume => {
            for (a, b) in [((5.0, 3.0), (12.5, 8.0)), ((12.5, 8.0), (5.0, 13.0)), ((5.0, 13.0), (5.0, 3.0))] {
                c.draw(ink, |x, y| seg(x, y, p(a.0, a.1), p(b.0, b.1)) - w / 2.0);
            }
        }
        Glyph::Home => {
            let lines = [
                ((2.5, 8.0), (8.0, 2.8)),
                ((8.0, 2.8), (13.5, 8.0)),
                ((4.0, 7.0), (4.0, 13.5)),
                ((4.0, 13.5), (12.0, 13.5)),
                ((12.0, 13.5), (12.0, 7.0)),
                ((8.0, 13.5), (8.0, 10.5)),
            ];
            for (a, b) in lines {
                c.draw(ink, |x, y| seg(x, y, p(a.0, a.1), p(b.0, b.1)) - w / 2.0);
            }
        }
        Glyph::Power => {
            let (cx, cy, r) = (8.0 * s, 8.8 * s, 5.0 * s);
            c.draw(ink, |x, y| {
                // Ring with a gap at the top for the switch line.
                let in_gap = y < cy && (x - cx).abs() < (cy - y) * 0.75;
                if in_gap {
                    f32::MAX
                } else {
                    (((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r).abs() - w / 2.0
                }
            });
            c.draw(ink, |x, y| seg(x, y, p(8.0, 2.3), p(8.0, 8.0)) - w / 2.0);
        }
        Glyph::Dot(rgb) => {
            let (cx, cy, r) = (8.0 * s, 8.0 * s, 3.5 * s);
            c.draw(rgb, |x, y| ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r);
        }
    }
    c.px
}

struct Canvas {
    n: usize,
    px: Vec<u8>,
}

impl Canvas {
    /// Paint `rgb` wherever the signed distance `d` is inside (negative), anti-aliased.
    fn draw(&mut self, rgb: [u8; 3], d: impl Fn(f32, f32) -> f32) {
        for y in 0..self.n {
            for x in 0..self.n {
                let a = (0.5 - d(x as f32 + 0.5, y as f32 + 0.5)).clamp(0.0, 1.0);
                if a <= 0.0 {
                    continue;
                }
                let i = (y * self.n + x) * 4;
                let old = self.px[i + 3] as f32 / 255.0;
                let out = a + old * (1.0 - a);
                for k in 0..3 {
                    let v = (rgb[k] as f32 * a + self.px[i + k] as f32 * old * (1.0 - a)) / out;
                    self.px[i + k] = v.round() as u8;
                }
                self.px[i + 3] = (out * 255.0).round() as u8;
            }
        }
    }
}

/// Distance from a point to a line segment.
fn seg(x: f32, y: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
    ((x - a.0 - t * dx).powi(2) + (y - a.1 - t * dy).powi(2)).sqrt()
}

/// Signed distance to a rounded rectangle.
pub fn rrect(x: f32, y: f32, a: (f32, f32), b: (f32, f32), r: f32) -> f32 {
    let (cx, cy) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let (hw, hh) = ((b.0 - a.0) / 2.0, (b.1 - a.1) / 2.0);
    let dx = (x - cx).abs() - (hw - r);
    let dy = (y - cy).abs() - (hh - r);
    (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() + dx.max(dy).min(0.0) - r
}
