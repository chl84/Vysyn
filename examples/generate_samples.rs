use anyhow::Result;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use std::{
    fs::{self, File},
    io::BufWriter,
    path::PathBuf,
};

fn main() -> Result<()> {
    let dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "artifacts/bench-images".into());
    fs::create_dir_all(&dir)?;
    let image = DynamicImage::ImageRgb8(RgbImage::from_fn(1920, 1080, |x, y| {
        Rgb([
            (x * 255 / 1920) as u8,
            (y * 255 / 1080) as u8,
            ((x + y) % 256) as u8,
        ])
    }));
    for (extension, format) in [
        ("jpg", ImageFormat::Jpeg),
        ("png", ImageFormat::Png),
        ("webp", ImageFormat::WebP),
        ("bmp", ImageFormat::Bmp),
        ("tiff", ImageFormat::Tiff),
    ] {
        image.write_to(
            &mut BufWriter::new(File::create(dir.join(format!("gradient.{extension}")))?),
            format,
        )?;
    }
    DynamicImage::ImageRgba8(image.thumbnail(256, 256).into_rgba8()).write_to(
        &mut BufWriter::new(File::create(dir.join("gradient.ico"))?),
        ImageFormat::Ico,
    )?;
    let mut encoder = image::codecs::gif::GifEncoder::new(BufWriter::new(File::create(
        dir.join("animated.gif"),
    )?));
    encoder.set_repeat(image::codecs::gif::Repeat::Infinite)?;
    for color in [[240, 60, 60, 255], [60, 240, 60, 255], [60, 60, 240, 255]] {
        encoder.encode_frame(image::Frame::from_parts(
            image::RgbaImage::from_pixel(320, 240, image::Rgba(color)),
            0,
            0,
            image::Delay::from_numer_denom_ms(100, 1),
        ))?;
    }
    fs::write(dir.join("gradient.svg"), br#"<svg xmlns="http://www.w3.org/2000/svg" width="1920" height="1080"><defs><linearGradient id="g"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs><rect width="1920" height="1080" fill="url(#g)"/></svg>"#)?;
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    fs::copy(fixtures.join("sample.avif"), dir.join("sample.avif"))?;
    fs::copy(fixtures.join("rotated.heic"), dir.join("rotated.heic"))?;
    println!("{}", dir.display());
    Ok(())
}
