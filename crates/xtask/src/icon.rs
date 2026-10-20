//! The app icon: a violet rounded square with the dictation pill's level bars.

use std::path::Path;

use image::{Rgba, RgbaImage, imageops::FilterType};

/// How much of the pixel at (x, y) a rounded rectangle covers (0..1, with a
/// one-pixel soft edge).
fn rounded(x: f32, y: f32, (x0, y0, x1, y1): (f32, f32, f32, f32), r: f32) -> f32 {
    let (cx, cy) = (x.clamp(x0 + r, x1 - r), y.clamp(y0 + r, y1 - r));
    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r;
    (0.5 - d).clamp(0.0, 1.0)
}

fn over(dst: &mut Rgba<u8>, src: [f32; 3], a: f32) {
    let da = dst[3] as f32 / 255.0;
    let out_a = a + da * (1.0 - a);
    if out_a <= 0.0 {
        return;
    }
    for i in 0..3 {
        dst[i] = ((src[i] * a + dst[i] as f32 * da * (1.0 - a)) / out_a).round() as u8;
    }
    dst[3] = (out_a * 255.0).round() as u8;
}

pub fn draw(n: u32) -> RgbaImage {
    let s = n as f32 / 1024.0;
    let mut img = RgbaImage::new(n, n);
    let (top, bottom) = ([91.0, 76.0, 230.0], [40.0, 26.0, 120.0]);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let t = fy / n as f32;
        let bg = [0, 1, 2].map(|i| top[i] + (bottom[i] - top[i]) * t);
        over(px, bg, rounded(fx, fy, (40.0 * s, 40.0 * s, (1024.0 - 40.0) * s, (1024.0 - 40.0) * s), 220.0 * s));
        over(px, [255.0; 3], rounded(fx, fy, (170.0 * s, 362.0 * s, 854.0 * s, 662.0 * s), 150.0 * s));
        for (i, h) in [70.0, 150.0, 220.0, 150.0, 70.0].iter().enumerate() {
            let cx = 512.0 + (i as f32 - 2.0) * 92.0;
            let bar = ((cx - 24.0) * s, (512.0 - h / 2.0) * s, (cx + 24.0) * s, (512.0 + h / 2.0) * s);
            over(px, [74.0, 58.0, 200.0], rounded(fx, fy, bar, 24.0 * s));
        }
    }
    img
}

/// icon.png (256 px) and icon.ico in `dir`; on macOS also localflow.icns.
pub fn write(dir: &Path) -> Result<(), String> {
    let big = draw(1024);
    let at = |size: u32| image::imageops::resize(&big, size, size, FilterType::Lanczos3);
    at(256).save(dir.join("icon.png")).map_err(|e| e.to_string())?;
    let frames: Vec<image::codecs::ico::IcoFrame> = [16, 24, 32, 48, 64, 128, 256]
        .iter()
        .map(|&size| image::codecs::ico::IcoFrame::as_png(at(size).as_raw(), size, size, image::ExtendedColorType::Rgba8))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let ico = std::fs::File::create(dir.join("icon.ico")).map_err(|e| e.to_string())?;
    image::codecs::ico::IcoEncoder::new(ico).encode_images(&frames).map_err(|e| e.to_string())?;
    if cfg!(target_os = "macos") {
        let set = dir.join("localflow.iconset");
        let _ = std::fs::remove_dir_all(&set);
        std::fs::create_dir_all(&set).map_err(|e| e.to_string())?;
        for size in [16, 32, 128, 256, 512] {
            at(size).save(set.join(format!("icon_{size}x{size}.png"))).map_err(|e| e.to_string())?;
            at(size * 2).save(set.join(format!("icon_{size}x{size}@2x.png"))).map_err(|e| e.to_string())?;
        }
        crate::run("iconutil", &["-c", "icns", &set.display().to_string(), "-o", &dir.join("localflow.icns").display().to_string()])?;
    }
    Ok(())
}
