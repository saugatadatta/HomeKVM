//! Microsoft Store package images, drawn from the app icon.
//! Usage: cargo run --example store_assets -- <output folder>

#[path = "../src/icon_art.rs"]
mod icon_art;

use std::path::Path;

fn main() {
    let out = std::env::args().nth(1).expect("usage: store_assets <output folder>");
    let out = Path::new(&out);
    std::fs::create_dir_all(out).expect("create output folder");

    for (name, size) in [("Square44x44Logo.png", 44), ("Square150x150Logo.png", 150), ("StoreLogo.png", 50)] {
        save(&out.join(name), size, size, &icon_art::draw(size, None));
    }
    // The wide tile: the icon centered on a transparent 310 x 150 canvas.
    let (w, h, icon) = (310usize, 150usize, 120usize);
    let art = icon_art::draw(icon, None);
    let mut wide = vec![0u8; w * h * 4];
    let (x0, y0) = ((w - icon) / 2, (h - icon) / 2);
    for y in 0..icon {
        let src = &art[y * icon * 4..(y + 1) * icon * 4];
        let dst = ((y0 + y) * w + x0) * 4;
        wide[dst..dst + icon * 4].copy_from_slice(src);
    }
    save(&out.join("Wide310x150Logo.png"), w, h, &wide);
    println!("Store images written to {}", out.display());
}

fn save(path: &Path, width: usize, height: usize, rgba: &[u8]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).expect("create image"));
    let mut encoder = png::Encoder::new(file, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut w| w.write_image_data(rgba)).expect("write image");
}
