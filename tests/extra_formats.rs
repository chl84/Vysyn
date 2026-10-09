use image::{ImageFormat, RgbaImage};
use std::path::Path;
use vysyn::{
    decode::{self, Decoded, Format, Target},
    limits::{Budget, Limits},
    navigation,
};

fn decode(path: &Path) -> anyhow::Result<Decoded> {
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

fn pixels(decoded: &Decoded) -> &RgbaImage {
    &decoded.frames[0].pixels
}

#[test]
fn netpbm_p1_through_p7_decode_pixels_and_navigate_without_extensions() {
    let dir = tempfile::tempdir().unwrap();
    type Case = (&'static str, &'static [u8], &'static [[u8; 4]]);
    let cases: &[Case] = &[
        ("P1", b"P1\n# ASCII bitmap\n2 1\n0 1\n", &[[255, 255, 255, 255], [0, 0, 0, 255]]),
        ("P2", b"P2\n2 1\n15\n0 15\n", &[[0, 0, 0, 255], [255, 255, 255, 255]]),
        ("P3", b"P3\n2 1\n255\n255 0 0 0 255 0\n", &[[255, 0, 0, 255], [0, 255, 0, 255]]),
        ("P4", b"P4\n2 1\n\x40", &[[255, 255, 255, 255], [0, 0, 0, 255]]),
        ("P5", b"P5\n2 1\n65535\n\0\0\xff\xff", &[[0, 0, 0, 255], [255, 255, 255, 255]]),
        ("P6", b"P6\n2 1\n255\n\xff\0\0\0\xff\0", &[[255, 0, 0, 255], [0, 255, 0, 255]]),
        ("P7", b"P7\nWIDTH 2\nHEIGHT 1\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n\xff\0\0\x80\0\xff\0\xff", &[[188, 0, 0, 128], [0, 255, 0, 255]]),
    ];
    for &(name, bytes, expected) in cases {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(
            decode::detect_file(&path).unwrap(),
            Format::Raster(ImageFormat::Pnm)
        );
        let image = decode(&path).unwrap();
        assert_eq!(image.original, [2, 1], "{name}");
        let actual: Vec<_> = pixels(&image).pixels().map(|p| p.0).collect();
        assert_eq!(actual, expected, "{name}");
    }
    let files = navigation::scan(dir.path(), &|| false).unwrap();
    assert_eq!(files.len(), cases.len());
    assert_eq!(
        navigation::neighbor(&files, &files[0], -1),
        files.last().cloned()
    );
}

fn farbfeld(width: u32, height: u32, color: [u16; 4]) -> Vec<u8> {
    let mut bytes = b"farbfeld".to_vec();
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    for _ in 0..width * height {
        for channel in color {
            bytes.extend(channel.to_be_bytes());
        }
    }
    bytes
}

#[test]
fn farbfeld_preserves_big_endian_channels_and_linear_transparency() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("renamed.bin");
    let mut bytes = farbfeld(1, 1, [65535, 65535, 65535, 32768]);
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(
        pixels(&decode(&path).unwrap()).get_pixel(0, 0).0,
        [188, 188, 188, 128]
    );
    bytes[14..16].copy_from_slice(&2_u16.to_be_bytes());
    bytes.extend(
        [65535_u16, 0, 65535, 0]
            .into_iter()
            .flat_map(u16::to_be_bytes),
    );
    std::fs::write(&path, bytes).unwrap();
    let image = decode(&path).unwrap();
    assert_eq!(image.original, [1, 2]);
    assert_eq!(pixels(&image).get_pixel(0, 1).0, [0, 0, 0, 0]);
}

fn tga(width: u16, height: u16, kind: u8, depth: u8, descriptor: u8) -> Vec<u8> {
    let mut bytes = vec![0; 18];
    bytes[2] = kind;
    bytes[12..14].copy_from_slice(&width.to_le_bytes());
    bytes[14..16].copy_from_slice(&height.to_le_bytes());
    bytes[16] = depth;
    bytes[17] = descriptor;
    bytes
}

fn tga_footer(bytes: &mut Vec<u8>) {
    bytes.extend([0; 8]);
    bytes.extend(b"TRUEVISION-XFILE.\0");
}

#[test]
fn tga_uncompressed_and_rle_honor_all_four_origins_and_alpha() {
    let dir = tempfile::tempdir().unwrap();
    let colors = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 255, 128],
    ];
    for origin in [0_u8, 0x10, 0x20, 0x30] {
        for rle in [false, true] {
            let mut bytes = tga(2, 2, if rle { 10 } else { 2 }, 32, origin | 8);
            if rle {
                bytes.push(3);
            } // Four literal pixels in one RLE packet.
            for y in 0..2 {
                for x in 0..2 {
                    let sx = if origin & 0x10 == 0 { x } else { 1 - x };
                    let sy = if origin & 0x20 == 0 { 1 - y } else { y };
                    let [r, g, b, a] = colors[sy * 2 + sx];
                    bytes.extend([b, g, r, a]);
                }
            }
            let path = dir.path().join(format!("origin-{origin}-{rle}.TARGA"));
            std::fs::write(&path, bytes).unwrap();
            let image = decode(&path).unwrap();
            let actual: Vec<_> = pixels(&image).pixels().map(|p| p.0).collect();
            assert_eq!(
                actual,
                [
                    [255, 0, 0, 255],
                    [0, 255, 0, 255],
                    [0, 0, 255, 255],
                    [188, 188, 188, 128]
                ],
                "{origin} {rle}"
            );
        }
    }
    assert_eq!(navigation::scan(dir.path(), &|| false).unwrap().len(), 8);
}

#[test]
fn tga_footer_detects_large_renamed_files_and_v1_requires_a_valid_extension() {
    let dir = tempfile::tempdir().unwrap();
    let mut bytes = tga(64, 32, 2, 24, 0x20);
    for _ in 0..64 * 32 {
        bytes.extend([0, 0, 255]);
    }
    let path = dir.path().join("unknown");
    std::fs::write(&path, &bytes).unwrap();
    assert!(decode::detect(&bytes).is_err());
    assert!(decode::detect_file(&path).is_err());
    assert!(decode(&path).is_err());
    tga_footer(&mut bytes);
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(
        decode::detect_file(&path).unwrap(),
        Format::Raster(ImageFormat::Tga)
    );
    assert_eq!(decode(&path).unwrap().original, [64, 32]);
    let named = dir.path().join("invalid.tga");
    for bad in [
        vec![0; 18],
        b"ordinary text pretending to be TGA".to_vec(),
        bytes[..17].to_vec(),
    ] {
        std::fs::write(&named, bad).unwrap();
        assert!(decode::detect_file(&named).is_err());
        assert!(decode(&named).is_err());
    }
    // A recognizable format still takes priority over a misleading extension.
    image::RgbImage::from_pixel(2, 1, image::Rgb([255, 0, 0]))
        .save_with_format(&named, ImageFormat::Png)
        .unwrap();
    assert_eq!(
        decode::detect_file(&named).unwrap(),
        Format::Raster(ImageFormat::Png)
    );
}

#[test]
fn tga_palette_grayscale_and_repeated_rle_pixels_decode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("types.tga");
    let mut palette = tga(2, 1, 1, 8, 0x20);
    palette[1] = 1;
    palette[5..7].copy_from_slice(&2_u16.to_le_bytes());
    palette[7] = 24;
    palette.extend([0, 0, 255, 0, 255, 0, 0, 1]);
    std::fs::write(&path, palette).unwrap();
    assert_eq!(
        pixels(&decode(&path).unwrap()).as_raw(),
        &[255, 0, 0, 255, 0, 255, 0, 255]
    );
    let mut gray = tga(2, 1, 3, 8, 0x20);
    gray.extend([0, 255]);
    std::fs::write(&path, gray).unwrap();
    assert_eq!(
        pixels(&decode(&path).unwrap()).as_raw(),
        &[0, 0, 0, 255, 255, 255, 255, 255]
    );
    let mut repeated = tga(2, 1, 10, 24, 0x20);
    repeated.extend([0x81, 0, 0, 255]);
    std::fs::write(&path, repeated).unwrap();
    assert!(
        pixels(&decode(&path).unwrap())
            .pixels()
            .all(|p| p.0 == [255, 0, 0, 255])
    );
}

fn dds(fourcc: &[u8; 4], blocks: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 128];
    bytes[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [
        (4, 124_u32),
        (8, 0x81007),
        (12, 4),
        (16, 4),
        (20, blocks.len() as u32),
        (76, 32),
        (80, 4),
        (108, 0x1000),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[84..88].copy_from_slice(fourcc);
    bytes.extend(blocks);
    bytes
}

fn dx10(format: u32, alpha: u32, blocks: &[u8]) -> Vec<u8> {
    let mut bytes = dds(b"DX10", &[]);
    for value in [format, 3, 0, 1, alpha] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(blocks);
    bytes
}

#[test]
fn dds_dxt1_dxt3_dxt5_and_dx10_keep_color_and_transparency() {
    let dir = tempfile::tempdir().unwrap();
    let red_block = [0, 0xf8, 0xe0, 7, 0, 0, 0, 0];
    let mut dxt3 = vec![0x88; 8];
    dxt3.extend(red_block);
    let mut dxt5 = vec![128, 0, 0, 0, 0, 0, 0, 0];
    dxt5.extend(red_block);
    let cases = [
        (dds(b"DXT1", &red_block), [255, 0, 0, 255]),
        (dds(b"DXT3", &dxt3), [193, 0, 0, 136]),
        (dds(b"DXT5", &dxt5), [188, 0, 0, 128]),
        (dx10(71, 1, &red_block), [255, 0, 0, 255]),
        (dx10(74, 1, &dxt3), [193, 0, 0, 136]),
        (dx10(77, 1, &dxt5), [188, 0, 0, 128]),
    ];
    for (index, (bytes, expected)) in cases.into_iter().enumerate() {
        let path = dir.path().join(format!("renamed-{index}"));
        std::fs::write(&path, bytes).unwrap();
        let image = decode(&path).unwrap();
        assert_eq!(image.original, [4, 4]);
        assert!(
            pixels(&image).pixels().all(|p| p.0 == expected),
            "DDS variant {index}: {:?}",
            pixels(&image).get_pixel(0, 0)
        );
    }
    let transparent_block = [0, 0, 255, 255, 7, 0, 0, 0]; // Selector 3, then 1 (white).
    let path = dir.path().join("bc1.dds");
    for bytes in [
        dds(b"DXT1", &transparent_block),
        dx10(72, 1, &transparent_block),
    ] {
        std::fs::write(&path, bytes).unwrap();
        let image = decode(&path).unwrap();
        assert_eq!(pixels(&image).get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(pixels(&image).get_pixel(1, 0).0, [255, 255, 255, 255]);
    }
    std::fs::write(&path, dx10(71, 3, &transparent_block)).unwrap();
    assert_eq!(
        pixels(&decode(&path).unwrap()).get_pixel(0, 0).0,
        [0, 0, 0, 255]
    );
    assert_eq!(navigation::scan(dir.path(), &|| false).unwrap().len(), 7);
}

#[test]
fn dds_unsupported_surfaces_and_codecs_fail_without_panics() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.dds");
    let block = [0; 8];
    let mut cube = dds(b"DXT1", &block);
    cube[112..116].copy_from_slice(&0x200_u32.to_le_bytes());
    let mut array = dx10(71, 1, &block);
    array[140..144].copy_from_slice(&2_u32.to_le_bytes());
    let mut dimensions = dds(b"DXT1", &block);
    dimensions[16..20].copy_from_slice(&5_u32.to_le_bytes());
    for bytes in [
        cube,
        array,
        dimensions,
        dx10(98, 1, &block),
        dx10(71, 2, &block),
        dds(b"DXT2", &block),
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert!(decode(&path).is_err());
    }
}

fn hdr(signature: &str, width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
    let mut bytes =
        format!("{signature}\nFORMAT=32-bit_rle_rgbe\n\n-Y {height} +X {width}\n").into_bytes();
    for _ in 0..width * height {
        bytes.extend(pixel);
    }
    bytes
}

#[test]
fn hdr_maps_linear_values_and_highlights_to_sdr_for_both_signatures() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("renamed-hdr");
    for signature in ["#?RADIANCE", "#?RGBE"] {
        let mut bytes = hdr(signature, 6, 1, [0; 4]);
        bytes.truncate(bytes.len() - 24);
        bytes.extend([
            0, 0, 0, 0, 128, 128, 128, 126, 128, 128, 128, 129, 128, 128, 128, 130, 128, 128, 128,
            132, 128, 128, 128, 136,
        ]);
        std::fs::write(&path, bytes).unwrap();
        let image = decode(&path).unwrap();
        assert_eq!(image.original, [6, 1]);
        let expected = [0_u8, 94, 188, 213, 242, 254];
        for (pixel, expected) in pixels(&image).pixels().zip(expected) {
            assert!(
                (i16::from(pixel[0]) - i16::from(expected)).abs() <= 1,
                "{signature}: {pixel:?} expected {expected}"
            );
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[1], pixel[2]);
            assert_eq!(pixel[3], 255);
        }
    }
    let mut rle = b"#?RGBE\r\nFORMAT=32-bit_rle_rgbe\r\n\r\n-Y 1 +X 8\r\n".to_vec();
    rle.extend([2, 2, 0, 8, 136, 128, 136, 128, 136, 128, 136, 129]);
    std::fs::write(&path, rle).unwrap();
    assert!(
        pixels(&decode(&path).unwrap())
            .pixels()
            .all(|p| p.0 == [188, 188, 188, 255])
    );
}

#[test]
fn hdr_rejects_xyze_and_unbounded_headers_even_for_rgbe_alias() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.hdr");
    for signature in ["#?RADIANCE", "#?RGBE"] {
        std::fs::write(
            &path,
            format!("{signature}\nFORMAT=32-bit_rle_xyze\n\n-Y 1 +X 1\n"),
        )
        .unwrap();
        assert!(decode(&path).is_err());
    }
    let mut bytes = b"#?RADIANCE\n".to_vec();
    bytes.extend(vec![b'x'; 65536 - bytes.len() - 1]);
    bytes.push(b'\n'); // A line ending at the cap is not an empty header line.
    bytes.extend(b"FORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 1\n\0\0\0\0");
    std::fs::write(&path, bytes).unwrap();
    assert!(decode(&path).unwrap_err().to_string().contains("64 KiB"));
}

#[test]
fn new_formats_enforce_dimensions_buffers_budget_and_gpu_downscaling() {
    let dir = tempfile::tempdir().unwrap();
    let mut pnm = b"P6\n4 4\n255\n".to_vec();
    pnm.extend([255, 0, 0].repeat(16));
    let mut tga = tga(4, 4, 2, 24, 0x20);
    tga.extend([0, 0, 255].repeat(16));
    let cases = [
        ("pnm", pnm),
        ("tga", tga),
        ("ff", farbfeld(4, 4, [65535, 0, 0, 65535])),
        ("dds", dds(b"DXT1", &[0, 0xf8, 0xe0, 7, 0, 0, 0, 0])),
        ("hdr", hdr("#?RADIANCE", 4, 4, [128, 0, 0, 129])),
    ];
    for (extension, bytes) in cases {
        let path = dir.path().join(format!("limits.{extension}"));
        std::fs::write(&path, &bytes).unwrap();
        for limits in [
            Limits {
                max_pixels: 15,
                ..Limits::default()
            },
            Limits {
                decoded_bytes: 32,
                ..Limits::default()
            },
        ] {
            let budget = Budget::new(limits.ram_bytes);
            let result = decode::decode(
                &path,
                &limits,
                &budget,
                Target {
                    max_dimension: 8192,
                    gpu_bytes: 1024,
                },
                &|| false,
            );
            assert!(
                result.unwrap_err().to_string().contains("limit"),
                "{extension}"
            );
            assert_eq!(
                budget.used(),
                0,
                "{extension} leaked memory after rejection"
            );
        }
        let limits = Limits::default();
        let tiny = Budget::new(1024);
        assert!(
            decode::decode(
                &path,
                &limits,
                &tiny,
                Target {
                    max_dimension: 8192,
                    gpu_bytes: 16
                },
                &|| false
            )
            .is_err()
        );
        assert_eq!(tiny.used(), 0);
        let budget = Budget::new(limits.ram_bytes);
        let image = decode::decode(
            &path,
            &limits,
            &budget,
            Target {
                max_dimension: 8192,
                gpu_bytes: 16,
            },
            &|| false,
        )
        .unwrap();
        assert_eq!(image.original, [4, 4]);
        assert_eq!(pixels(&image).dimensions(), (2, 2));
        assert_eq!(budget.used(), 16);
        drop(image);
        assert_eq!(budget.used(), 0);
        std::fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
        assert!(
            decode(&path).is_err(),
            "{extension} accepted truncated pixel data"
        );
    }
    let hdr_path = dir.path().join("float.hdr");
    std::fs::write(&hdr_path, hdr("#?RADIANCE", 4, 4, [128, 0, 0, 129])).unwrap();
    let limits = Limits {
        decoded_bytes: 128,
        ..Limits::default()
    };
    assert!(
        decode::decode(
            &hdr_path,
            &limits,
            &Budget::new(limits.ram_bytes),
            Target {
                max_dimension: 8192,
                gpu_bytes: 1024
            },
            &|| false
        )
        .unwrap_err()
        .to_string()
        .contains("float buffer")
    );
}
