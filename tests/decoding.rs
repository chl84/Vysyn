use image::{DynamicImage, ImageEncoder, ImageFormat, Rgba, RgbaImage};
use std::{io::Cursor, path::Path, time::Duration};
use vysyn::{
    decode::{self, Target},
    limits::{Budget, Limits},
    navigation,
};

fn decode(path: &Path) -> anyhow::Result<decode::Decoded> {
    let limits = Limits::default();
    decode::decode(
        path,
        &limits,
        &Budget::new(limits.ram_bytes),
        Target {
            max_dimension: 8192,
            gpu_bytes: limits.gpu_bytes,
        },
        &|| false,
    )
}

#[test]
fn raster_formats_decode_without_extensions() {
    let dir = tempfile::tempdir().unwrap();
    let source = RgbaImage::from_fn(24, 16, |x, y| Rgba([x as u8 * 10, y as u8 * 10, 80, 255]));
    for format in [
        ImageFormat::Jpeg,
        ImageFormat::Png,
        ImageFormat::WebP,
        ImageFormat::Gif,
        ImageFormat::Bmp,
        ImageFormat::Tiff,
        ImageFormat::Ico,
    ] {
        let mut bytes = Cursor::new(Vec::new());
        let image = if format == ImageFormat::Jpeg {
            DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(source.clone()).into_rgb8())
        } else {
            DynamicImage::ImageRgba8(source.clone())
        };
        image.write_to(&mut bytes, format).unwrap();
        let path = dir.path().join(format!("{format:?}"));
        std::fs::write(&path, bytes.into_inner()).unwrap();
        let decoded = decode(&path).unwrap();
        assert_eq!(decoded.original, [24, 16], "{format:?}");
        assert_eq!(decoded.frames.len(), 1);
    }
    let files = navigation::scan(dir.path(), &|| false).unwrap();
    assert_eq!(files.len(), 7);
    assert!(
        files
            .windows(2)
            .all(|f| f[0].file_name() < f[1].file_name())
    );
}

#[test]
fn animated_gif_composites_and_retains_delays_and_repeats() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("animated.gif");
    let mut data = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut data);
        encoder
            .set_repeat(image::codecs::gif::Repeat::Finite(2))
            .unwrap();
        for (color, delay) in [(Rgba([255, 0, 0, 255]), 20), (Rgba([0, 255, 0, 255]), 70)] {
            encoder
                .encode_frame(image::Frame::from_parts(
                    RgbaImage::from_pixel(8, 8, color),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(delay, 1),
                ))
                .unwrap();
        }
    }
    std::fs::write(&path, &data).unwrap();
    let d = decode(&path).unwrap();
    assert_eq!(d.frames.len(), 2);
    assert_eq!(d.loops, Some(3));
    assert_eq!(d.frames[0].delay, Duration::from_millis(20));
    assert_eq!(d.frames[1].delay, Duration::from_millis(70));
    assert_eq!(d.frames[1].pixels.get_pixel(0, 0).0, [0, 255, 0, 255]);
    // Remove the looping extension: ordinary GIFs play once.
    let begin = data.windows(3).position(|w| w == [0x21, 0xff, 11]).unwrap();
    data.drain(begin..begin + 19);
    std::fs::write(&path, data).unwrap();
    assert_eq!(decode(&path).unwrap().loops, Some(1));
    let mut bad = std::fs::read(&path).unwrap();
    let frame = bad
        .windows(9)
        .position(|w| w == [0x2c, 0, 0, 0, 0, 8, 0, 8, 0])
        .unwrap();
    bad[frame + 5..frame + 7].copy_from_slice(&u16::MAX.to_le_bytes());
    std::fs::write(&path, bad).unwrap();
    assert!(
        decode(&path)
            .unwrap_err()
            .to_string()
            .contains("outside its canvas")
    );
}

#[test]
fn png_icc_and_transparency_are_applied_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("icc.png");
    let profile = moxcms::ColorProfile::new_display_p3().encode().unwrap();
    let mut encoder = image::codecs::png::PngEncoder::new(std::fs::File::create(&path).unwrap());
    encoder.set_icc_profile(profile).unwrap();
    encoder
        .write_image(&[170, 100, 80, 128], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    let image = decode(&path).unwrap();
    assert!(image.warnings.is_empty(), "{:?}", image.warnings);
    let mut expected = [170, 100, 80, 128];
    vysyn::color::convert_profile(&mut expected, &moxcms::ColorProfile::new_display_p3()).unwrap();
    vysyn::color::premultiply_linear(&mut expected);
    assert_eq!(image.frames[0].pixels.get_pixel(0, 0).0, expected);
}

#[test]
fn jpeg_exif_rotation_changes_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rotated.jpg");
    let source =
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(12, 8, image::Rgb([220, 0, 0])));
    let mut bytes = Cursor::new(Vec::new());
    source.write_to(&mut bytes, ImageFormat::Jpeg).unwrap();
    let mut bytes = bytes.into_inner();
    let exif = [
        b'E', b'x', b'i', b'f', 0, 0, b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0,
        0, 6, 0, 0, 0, 0, 0, 0, 0,
    ];
    let mut segment = vec![0xff, 0xe1];
    segment.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    segment.extend(exif);
    bytes.splice(2..2, segment);
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(decode(&path).unwrap().original, [8, 12]);
}

#[test]
fn corrupt_truncated_and_oversized_images_are_controlled_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.png");
    for bytes in [
        b"".as_slice(),
        b"\x89PNG\r\n\x1a\n",
        b"GIF89a\xff\xff\xff\xff\x80\0\0",
        b"<svg width='100000' height='100000' xmlns='http://www.w3.org/2000/svg'/>",
        b"\0\0\0\x01ftypheic\0\0\0\0",
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert!(decode(&path).is_err());
    }
    let valid = RgbaImage::from_pixel(100, 100, Rgba([1, 2, 3, 255]));
    valid.save(&path).unwrap();
    let limits = Limits {
        max_pixels: 9999,
        ..Limits::default()
    };
    assert!(
        decode::decode(
            &path,
            &limits,
            &Budget::new(limits.ram_bytes),
            Target {
                max_dimension: 8192,
                gpu_bytes: limits.gpu_bytes
            },
            &|| false
        )
        .is_err()
    );
    assert!(
        decode::decode(
            &path,
            &Limits::default(),
            &Budget::new(100),
            Target {
                max_dimension: 8192,
                gpu_bytes: 100
            },
            &|| false
        )
        .is_err()
    );
}

#[test]
fn svg_external_resources_are_ignored_and_expensive_graphs_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("safe.svg");
    std::fs::write(&path, br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><image href="file:///etc/passwd" width="8" height="8"/><image href="https://example.com/secret.png" width="8" height="8"/><rect width="8" height="8" fill="red"/></svg>"#).unwrap();
    let image = decode(&path).unwrap();
    assert_eq!(image.frames[0].pixels.get_pixel(0, 0).0, [255, 0, 0, 255]);
    std::fs::write(&path, br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><filter id="f"><feGaussianBlur stdDeviation="2"/></filter><rect width="8" height="8" filter="url(#f)"/></svg>"##).unwrap();
    assert!(decode(&path).is_err());
    let excessive_text = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><text>{}</text></svg>",
        "x".repeat(16_385)
    );
    std::fs::write(&path, excessive_text).unwrap();
    assert!(
        decode(&path)
            .unwrap_err()
            .to_string()
            .contains("shaping limit")
    );
}

#[test]
fn cancelled_work_and_empty_directories_are_handled() {
    let dir = tempfile::tempdir().unwrap();
    assert!(navigation::scan(dir.path(), &|| false).unwrap().is_empty());
    let path = dir.path().join("x.png");
    RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 255]))
        .save(&path)
        .unwrap();
    let limits = Limits::default();
    assert!(
        decode::decode(
            &path,
            &limits,
            &Budget::new(limits.ram_bytes),
            Target {
                max_dimension: 8192,
                gpu_bytes: limits.gpu_bytes
            },
            &|| true
        )
        .is_err()
    );
    assert!(navigation::scan(dir.path(), &|| true).is_err());
}

#[test]
fn native_heic_rotation_and_avif_decode_with_security_limits() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let heic = decode(&dir.join("rotated.heic")).unwrap();
    assert_eq!(heic.original, [48, 64]);
    assert!(heic.warnings.is_empty(), "{:?}", heic.warnings);
    let avif = decode(&dir.join("sample.avif")).unwrap();
    assert_eq!(avif.original, [64, 48]);
    assert!(avif.warnings.is_empty(), "{:?}", avif.warnings);
    // Verify that the native context rejects dimensions before pixel decode.
    let l = Limits {
        max_pixels: 100,
        ..Limits::default()
    };
    assert!(
        decode::decode(
            &dir.join("sample.avif"),
            &l,
            &Budget::new(l.ram_bytes),
            Target {
                max_dimension: 8192,
                gpu_bytes: l.gpu_bytes
            },
            &|| false
        )
        .is_err()
    );
}
