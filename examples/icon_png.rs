//! The app icon as a PNG at any size, for Store listing images.
//! Usage: cargo run --example icon_png -- <size> <output.png>

#[path = "../src/icon_art.rs"]
mod icon_art;

fn main() {
    let mut args = std::env::args().skip(1);
    let size: usize = args.next().and_then(|s| s.parse().ok()).expect("usage: icon_png <size> <output.png>");
    let path = args.next().expect("usage: icon_png <size> <output.png>");
    let rgba = icon_art::draw(size, None);
    let file = std::io::BufWriter::new(std::fs::File::create(&path).expect("create image"));
    let mut encoder = png::Encoder::new(file, size as u32, size as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut w| w.write_image_data(&rgba)).expect("write image");
    println!("{size} x {size} icon written to {path}");
}
