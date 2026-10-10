use anyhow::Result;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
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
        ("tga", ImageFormat::Tga),
        ("pam", ImageFormat::Pnm),
    ] {
        image.write_to(
            &mut BufWriter::new(File::create(dir.join(format!("gradient.{extension}")))?),
            format,
        )?;
    }
    DynamicImage::ImageRgba16(image.to_rgba16()).write_to(
        &mut BufWriter::new(File::create(dir.join("gradient.ff"))?),
        ImageFormat::Farbfeld,
    )?;
    let rgb = image.as_rgb8().unwrap();
    // Minimal PSD v1 composites. No layer encoder or new runtime dependency.
    for depth in [8_u16, 16] {
        let mut file = BufWriter::new(File::create(dir.join(format!("gradient-{depth}.psd")))?);
        file.write_all(b"8BPS\0\x01\0\0\0\0\0\0\0\x03")?;
        file.write_all(&1080_u32.to_be_bytes())?;
        file.write_all(&1920_u32.to_be_bytes())?;
        file.write_all(&depth.to_be_bytes())?;
        file.write_all(&3_u16.to_be_bytes())?;
        file.write_all(&[0; 12])?; // Empty color data, resources and layers.
        file.write_all(&0_u16.to_be_bytes())?; // Uncompressed planar samples.
        for channel in 0..3 {
            for pixel in rgb.pixels() {
                if depth == 8 {
                    file.write_all(&[pixel[channel]])?;
                } else {
                    file.write_all(&(u16::from(pixel[channel]) * 257).to_be_bytes())?;
                }
            }
        }
    }
    let gray = image.to_luma8();
    for (extension, signature, data) in [("ppm", "P6", rgb.as_raw()), ("pgm", "P5", gray.as_raw())]
    {
        let mut file = BufWriter::new(File::create(dir.join(format!("gradient.{extension}")))?);
        write!(file, "{signature}\n1920 1080\n255\n")?;
        file.write_all(data)?;
    }
    let mut bitmap = BufWriter::new(File::create(dir.join("gradient.pbm"))?);
    bitmap.write_all(b"P4\n1920 1080\n")?;
    for row in gray.as_raw().as_chunks::<1920>().0 {
        for chunk in row.as_chunks::<8>().0 {
            let mut byte = 0;
            for (bit, &value) in chunk.iter().enumerate() {
                byte |= u8::from(value < 128) << (7 - bit);
            }
            bitmap.write_all(&[byte])?;
        }
    }
    let hdr = DynamicImage::ImageRgb32F(image::Rgb32FImage::from_fn(1920, 1080, |x, y| {
        Rgb(rgb
            .get_pixel(x, y)
            .0
            .map(|c| vysyn::color::srgb_to_linear(c) * 8.0))
    }));
    hdr.write_to(
        &mut BufWriter::new(File::create(dir.join("gradient.hdr"))?),
        ImageFormat::Hdr,
    )?;
    let mut dds = vec![0; 128];
    dds[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [
        (4, 124_u32),
        (8, 0x81007),
        (12, 1080),
        (16, 1920),
        (20, 1920 * 1080 / 2),
        (76, 32),
        (80, 4),
        (108, 0x1000),
    ] {
        dds[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    dds[84..88].copy_from_slice(b"DXT1");
    for _ in 0..1920 * 1080 / 16 {
        dds.extend([0, 0xf8, 0, 0, 0, 0, 0, 0]);
    }
    fs::write(dir.join("red.dds"), dds)?;
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
