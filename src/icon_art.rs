//! The HomeKVM icon, a small keyboard, drawn at any size with an optional status dot.
//! It has no dependencies, so build.rs can use it too (for the .exe's own icon).

const BODY: [u8; 3] = [62, 84, 120];
const KEY: [u8; 3] = [235, 240, 248];

/// RGBA pixels, `n` by `n`. With `dot`, a status dot sits in the bottom-right corner.
pub fn draw(n: usize, dot: Option<[u8; 3]>) -> Vec<u8> {
    // Shapes are laid out on a 32 x 32 grid and scaled to `n`.
    let s = n as f32 / 32.0;
    let mut px = vec![0u8; n * n * 4];

    paint(&mut px, n, s, BODY, |x, y| rrect(x, y, 1.0, 7.0, 31.0, 25.0, 3.5));
    for row in [10.0, 14.0] {
        for col in [4.0, 8.0, 12.0, 16.0, 20.0, 24.0] {
            paint(&mut px, n, s, KEY, |x, y| rrect(x, y, col, row, col + 3.0, row + 2.0, 0.4));
        }
    }
    paint(&mut px, n, s, KEY, |x, y| rrect(x, y, 8.0, 19.0, 24.0, 21.0, 0.4));

    if let Some(rgb) = dot {
        // Cut a transparent ring around the dot so it reads on any taskbar color.
        let (cx, cy, r) = (24.5, 24.5, 6.0);
        let dist = |x: f32, y: f32| ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        for y in 0..n {
            for x in 0..n {
                let d = dist((x as f32 + 0.5) / s, (y as f32 + 0.5) / s) - (r + 1.75);
                let ring = (0.5 - d * s).clamp(0.0, 1.0);
                let i = (y * n + x) * 4 + 3;
                px[i] = (px[i] as f32 * (1.0 - ring)) as u8;
            }
        }
        paint(&mut px, n, s, rgb, |x, y| dist(x, y) - r);
    }
    px
}

/// Paint `rgb` where the signed distance `d` (in grid units, negative inside) is inside.
fn paint(px: &mut [u8], n: usize, s: f32, rgb: [u8; 3], d: impl Fn(f32, f32) -> f32) {
    for y in 0..n {
        for x in 0..n {
            let a = (0.5 - d((x as f32 + 0.5) / s, (y as f32 + 0.5) / s) * s).clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let i = (y * n + x) * 4;
            let old = px[i + 3] as f32 / 255.0;
            let out = a + old * (1.0 - a);
            for c in 0..3 {
                px[i + c] = ((rgb[c] as f32 * a + px[i + c] as f32 * old * (1.0 - a)) / out).round() as u8;
            }
            px[i + 3] = (out * 255.0).round() as u8;
        }
    }
}

/// Signed distance to a rounded rectangle from (x0, y0) to (x1, y1).
fn rrect(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let dx = (x - (x0 + x1) / 2.0).abs() - ((x1 - x0) / 2.0 - r);
    let dy = (y - (y0 + y1) / 2.0).abs() - ((y1 - y0) / 2.0 - r);
    (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() + dx.max(dy).min(0.0) - r
}
