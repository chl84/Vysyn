use flate2::{Compression, write::ZlibEncoder};
use std::{
    cell::Cell,
    fs::File,
    io::{Seek, SeekFrom, Write},
    path::Path,
};
use vysyn::{
    color,
    decode::{self, Decoded, Format, Target},
    limits::{Budget, Limits},
    navigation,
};

fn section(data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend(data);
    out
}

fn resource(id: u16, data: &[u8]) -> Vec<u8> {
    let mut out = b"8BIM".to_vec();
    out.extend(id.to_be_bytes());
    out.extend([0, 0]); // Empty Pascal name, padded to two bytes.
    out.extend(section(data));
    if !data.len().is_multiple_of(2) {
        out.push(0);
    }
    out
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

struct Psd {
    width: u32,
    height: u32,
    depth: u16,
    mode: u16,
    planes: Vec<Vec<u16>>,
    resources: Vec<u8>,
    layers: Vec<u8>,
}

impl Psd {
    fn rgb(depth: u16) -> Self {
        // Nonuniform rows exercise planar ordering, prediction wrap and reset.
        Self {
            width: 3,
            height: 2,
            depth,
            mode: 3,
            planes: vec![
                vec![0, 65535, 32768, 20000, 60000, 65535],
                vec![65535, 0, 12000, 65535, 8000, 50000],
                vec![30000, 50000, 65535, 0, 65535, 2000],
            ],
            resources: Vec::new(),
            layers: Vec::new(),
        }
    }
    fn alpha_ids(&mut self, ids: &[u32]) {
        self.resources.extend(resource(
            1053,
            &ids.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>(),
        ));
    }
    fn negative_layers(&mut self) {
        self.layers = section(&(-1_i16).to_be_bytes());
        self.layers.extend(section(&[]));
    }
    fn marker(&mut self, key: &[u8; 4], data: &[u8]) {
        if self.layers.is_empty() {
            self.layers = vec![0; 8];
        }
        self.layers.extend(b"8BIM");
        self.layers.extend(key);
        self.layers.extend(section(data));
        self.layers
            .resize(self.layers.len() + (4 - data.len() % 4) % 4, 0);
    }
    fn bytes(&self, compression: u16) -> Vec<u8> {
        let mut out = b"8BPS".to_vec();
        out.extend(1_u16.to_be_bytes());
        out.extend([0; 6]);
        out.extend((self.planes.len() as u16).to_be_bytes());
        out.extend(self.height.to_be_bytes());
        out.extend(self.width.to_be_bytes());
        out.extend(self.depth.to_be_bytes());
        out.extend(self.mode.to_be_bytes());
        out.extend(section(&[]));
        out.extend(section(&self.resources));
        out.extend(section(&self.layers));
        out.extend(compression.to_be_bytes());
        let mut rows = Vec::new();
        for plane in &self.planes {
            for row in plane.chunks(self.width as usize) {
                let mut previous = 0_u16;
                let bytes: Vec<_> = row
                    .iter()
                    .flat_map(|&value| {
                        let value = if self.depth == 8 {
                            ((u32::from(value) + 128) / 257) as u16
                        } else {
                            value
                        };
                        let encoded = if compression == 3 {
                            value.wrapping_sub(previous)
                        } else {
                            value
                        };
                        previous = value;
                        if self.depth == 8 {
                            vec![encoded as u8]
                        } else {
                            encoded.to_be_bytes().to_vec()
                        }
                    })
                    .collect();
                rows.push(bytes);
            }
        }
        match compression {
            1 => {
                let packets: Vec<Vec<u8>> = rows
                    .iter()
                    .map(|row| {
                        let mut packed = vec![128]; // Legal PackBits no-op.
                        for chunk in row.chunks(128) {
                            packed.push((chunk.len() - 1) as u8);
                            packed.extend(chunk);
                        }
                        packed.push(128);
                        packed
                    })
                    .collect();
                for row in &packets {
                    out.extend((row.len() as u16).to_be_bytes());
                }
                for row in packets {
                    out.extend(row);
                }
            }
            2 | 3 => out.extend(zlib(&rows.concat())),
            _ => out.extend(rows.concat()),
        }
        out
    }
}

fn load(
    path: &Path,
    limits: &Limits,
    budget: &Budget,
    target: u32,
    cancelled: &dyn Fn() -> bool,
) -> anyhow::Result<Decoded> {
    decode::decode(
        path,
        limits,
        budget,
        Target {
            max_dimension: target,
            gpu_bytes: limits.gpu_bytes,
        },
        cancelled,
    )
}

fn from_bytes(data: &[u8]) -> anyhow::Result<Decoded> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("renamed");
    std::fs::write(&path, data)?;
    let limits = Limits::default();
    load(
        &path,
        &limits,
        &Budget::new(limits.ram_bytes),
        8192,
        &|| false,
    )
}

#[test]
fn planar_rgb_and_gray_8_and_16_bit_all_four_compressions() {
    let dir = tempfile::tempdir().unwrap();
    for depth in [8, 16] {
        for gray in [false, true] {
            let mut psd = Psd::rgb(depth);
            if gray {
                psd.mode = 1;
                psd.planes.truncate(1);
            }
            let expected: Vec<_> = (0..6)
                .flat_map(|i| {
                    let channel = |n: usize| ((u32::from(psd.planes[n][i]) + 128) / 257) as u8;
                    [
                        channel(0),
                        channel(if gray { 0 } else { 1 }),
                        channel(if gray { 0 } else { 2 }),
                        255,
                    ]
                })
                .collect();
            for compression in 0..=3 {
                let bytes = psd.bytes(compression);
                assert_eq!(decode::detect(&bytes).unwrap(), Format::Psd);
                let path = dir.path().join(format!("{depth}-{gray}-{compression}"));
                std::fs::write(&path, bytes).unwrap();
                let limits = Limits::default();
                let decoded = load(
                    &path,
                    &limits,
                    &Budget::new(limits.ram_bytes),
                    8192,
                    &|| false,
                )
                .unwrap();
                assert_eq!(decoded.original, [3, 2]);
                assert!(decoded.warnings.is_empty());
                assert_eq!(
                    decoded.frames[0].pixels.as_raw(),
                    &expected,
                    "{depth} {gray} {compression}"
                );
            }
        }
    }
    assert_eq!(navigation::scan(dir.path(), &|| false).unwrap().len(), 16);
}

#[test]
fn transparency_markers_remove_white_matte_and_saved_masks_stay_opaque() {
    for depth in [8, 16] {
        let mut psd = Psd {
            width: 2,
            height: 1,
            depth,
            mode: 3,
            planes: vec![
                vec![65535, 65535],
                vec![if depth == 8 { 32639 } else { 32767 }, 65535],
                vec![if depth == 8 { 32639 } else { 32767 }, 65535],
                vec![32768, 0],
            ],
            resources: Vec::new(),
            layers: Vec::new(),
        };
        let opaque = from_bytes(&psd.bytes(0)).unwrap();
        assert_eq!(
            opaque.frames[0].pixels.get_pixel(0, 0).0,
            [255, 127, 127, 255]
        );
        psd.alpha_ids(&[10]); // A saved mask is not transparency.
        assert_eq!(
            from_bytes(&psd.bytes(0)).unwrap().frames[0]
                .pixels
                .get_pixel(0, 0)
                .0[3],
            255
        );
        psd.resources.clear();
        for marker in 0..4 {
            psd.layers.clear();
            psd.resources.clear();
            match marker {
                0 => psd.alpha_ids(&[0]),
                1 => psd.negative_layers(),
                2 => psd.marker(if depth == 8 { b"Mtrn" } else { b"Mt16" }, &[]),
                _ => psd.marker(
                    if depth == 8 { b"Layr" } else { b"Lr16" },
                    &(-1_i16).to_be_bytes(),
                ),
            }
            let decoded = from_bytes(&psd.bytes(3)).unwrap();
            assert_eq!(
                decoded.frames[0].pixels.get_pixel(0, 0).0,
                [188, 0, 0, 128],
                "{depth} {marker}"
            );
            assert_eq!(decoded.frames[0].pixels.get_pixel(1, 0).0, [0, 0, 0, 0]);
        }
        // ID zero can be after a saved mask, rather than the fourth channel.
        psd.layers.clear();
        psd.resources.clear();
        psd.planes.insert(3, vec![0, 0]);
        psd.alpha_ids(&[3, 0]);
        assert_eq!(
            from_bytes(&psd.bytes(1)).unwrap().frames[0]
                .pixels
                .get_pixel(0, 0)
                .0,
            [188, 0, 0, 128]
        );
        // A negative layer count explicitly selects the first extra channel.
        psd.negative_layers();
        assert_eq!(
            from_bytes(&psd.bytes(0)).unwrap().frames[0]
                .pixels
                .get_pixel(0, 0)
                .0,
            [0, 0, 0, 0]
        );
    }
    let mut gray = Psd::rgb(16);
    gray.mode = 1;
    gray.planes = vec![vec![32767; 6], vec![32768; 6]];
    gray.alpha_ids(&[0]);
    assert_eq!(
        from_bytes(&gray.bytes(3)).unwrap().frames[0]
            .pixels
            .get_pixel(0, 0)
            .0,
        [0, 0, 0, 128]
    );
}

#[test]
fn packbits_runs_are_decoded_and_invalid_packets_rejected() {
    let mut psd = Psd::rgb(8);
    psd.width = 3;
    psd.height = 1;
    psd.mode = 1;
    psd.planes = vec![vec![65535; 3]];
    let mut bytes = psd.bytes(0);
    bytes.truncate(40);
    bytes[38..40].copy_from_slice(&1_u16.to_be_bytes());
    bytes.extend([0, 3, 128, 254, 80]); // no-op + repeat three bytes.
    assert_eq!(
        from_bytes(&bytes).unwrap().frames[0].pixels.as_raw(),
        &[80, 80, 80, 255].repeat(3)
    );
    for packet in [
        &[253, 80][..],
        &[254][..],
        &[2, 80][..],
        &[255, 80][..],
        &[128][..],
    ] {
        let mut bad = bytes[..40].to_vec();
        bad.extend((packet.len() as u16).to_be_bytes());
        bad.extend(packet);
        assert!(from_bytes(&bad).is_err(), "{packet:?}");
    }
}

#[test]
fn icc_rgb_8_and_16_bit_conversion_precedes_quantization_and_alpha_is_preserved() {
    use moxcms::{ColorProfile, Layout};
    let profile = ColorProfile::new_display_p3();
    for depth in [8, 16] {
        let mut psd = Psd::rgb(depth);
        psd.resources = resource(1039, &profile.encode().unwrap());
        let image = from_bytes(&psd.bytes(2)).unwrap();
        assert!(image.warnings.is_empty());
        let mut expected = Vec::new();
        if depth == 8 {
            for i in 0..6 {
                expected.extend([
                    ((u32::from(psd.planes[0][i]) + 128) / 257) as u8,
                    ((u32::from(psd.planes[1][i]) + 128) / 257) as u8,
                    ((u32::from(psd.planes[2][i]) + 128) / 257) as u8,
                    255,
                ]);
            }
            color::convert_profile(&mut expected, &profile).unwrap();
        } else {
            let source: Vec<u16> = (0..6)
                .flat_map(|i| [psd.planes[0][i], psd.planes[1][i], psd.planes[2][i], 65535])
                .collect();
            let mut out = vec![0; source.len()];
            let transform = profile
                .create_transform_16bit(
                    Layout::Rgba,
                    &ColorProfile::new_srgb(),
                    Layout::Rgba,
                    Default::default(),
                )
                .unwrap();
            transform.transform(&source, &mut out).unwrap();
            expected = out
                .into_iter()
                .map(|v| ((u32::from(v) + 128) / 257) as u8)
                .collect();
        }
        assert_eq!(image.frames[0].pixels.as_raw(), &expected);
        psd.planes.push(vec![32768; 6]);
        psd.alpha_ids(&[0]);
        let image = from_bytes(&psd.bytes(1)).unwrap();
        assert!(image.warnings.is_empty());
        assert!(image.frames[0].pixels.pixels().all(|p| p[3] == 128));
        psd.resources = resource(1039, b"bad profile");
        let image = from_bytes(&psd.bytes(0)).unwrap();
        assert_eq!(image.warnings.len(), 1);
        assert!(image.warnings[0].contains("assuming sRGB"));
    }
}

#[test]
fn grayscale_icc_converts_in_source_precision_and_keeps_opacity() {
    use moxcms::{ColorProfile, Layout};
    let profile = ColorProfile::new_gray_with_gamma(1.0);
    for depth in [8, 16] {
        let mut psd = Psd::rgb(depth);
        psd.mode = 1;
        psd.planes = vec![vec![49151; 6], vec![32768; 6]];
        psd.resources = resource(1039, &profile.encode().unwrap());
        psd.alpha_ids(&[0]);
        let decoded = from_bytes(&psd.bytes(3)).unwrap();
        assert!(decoded.warnings.is_empty(), "{:?}", decoded.warnings);
        let mut expected = if depth == 8 {
            let mut pixels = [128, 128, 128, 128]; // (191 - 127) * 255 / 128.
            color::convert_profile(&mut pixels, &profile).unwrap();
            pixels
        } else {
            let transform = profile
                .create_transform_16bit(
                    Layout::Gray,
                    &ColorProfile::new_srgb(),
                    Layout::Rgba,
                    Default::default(),
                )
                .unwrap();
            let mut pixels = [0_u16; 4];
            transform.transform(&[32768], &mut pixels).unwrap();
            pixels[3] = 32768;
            pixels.map(|v| ((u32::from(v) + 128) / 257) as u8)
        };
        color::premultiply_linear(&mut expected);
        assert_eq!(decoded.frames[0].pixels.get_pixel(0, 0).0, expected);
    }
}

#[test]
fn missing_composite_unsupported_modes_and_bad_sections_are_controlled_errors() {
    let mut psd = Psd::rgb(8);
    psd.resources = resource(1057, &[0, 0, 0, 1, 0]);
    assert!(
        from_bytes(&psd.bytes(0))
            .unwrap_err()
            .to_string()
            .contains("Maximize PSD")
    );
    psd.resources.clear();
    let valid = psd.bytes(0);
    for (at, word, message) in [(4, 2, "PSB"), (22, 32, "8/16"), (24, 4, "RGB/grayscale")] {
        let mut bad = valid.clone();
        bad[at..at + 2].copy_from_slice(&(word as u16).to_be_bytes());
        assert!(from_bytes(&bad).unwrap_err().to_string().contains(message));
    }
    for at in [26, 30, 34] {
        let mut bad = valid.clone();
        bad[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(from_bytes(&bad).is_err());
    }
    psd.resources = resource(1053, &[0; 3]);
    assert!(from_bytes(&psd.bytes(0)).is_err());
    psd.resources = resource(1039, &vec![0; color::MAX_ICC_BYTES + 1]);
    assert!(from_bytes(&psd.bytes(0)).is_err());
    for compression in 0..4 {
        let bytes = Psd::rgb(16).bytes(compression);
        for end in 0..bytes.len() {
            assert!(
                from_bytes(&bytes[..end]).is_err(),
                "accepted truncated {compression} at {end}"
            );
        }
    }
}

#[test]
fn zip_checks_checksum_exact_expansion_and_trailing_data() {
    let valid = Psd::rgb(8).bytes(2);
    let mut checksum = valid.clone();
    *checksum.last_mut().unwrap() ^= 1;
    assert!(from_bytes(&checksum).is_err());
    for extra in [vec![0], zlib(&[0])] {
        let mut bad = valid.clone();
        bad.extend(extra);
        assert!(from_bytes(&bad).is_err());
    }
    for expanded in [vec![0; 17], vec![0; 19], vec![0; 1_000_000]] {
        let mut bad = valid[..40].to_vec();
        bad.extend(zlib(&expanded));
        assert!(from_bytes(&bad).is_err());
    }
}

#[test]
fn limits_cancellation_downscale_and_reservation_lifetime() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("image.psd");
    std::fs::write(&path, Psd::rgb(16).bytes(3)).unwrap();
    let limits = Limits::default();
    let budget = Budget::new(limits.ram_bytes);
    let image = load(&path, &limits, &budget, 2, &|| false).unwrap();
    assert_eq!(image.original, [3, 2]);
    assert_eq!(image.frames[0].pixels.dimensions(), (2, 1));
    assert_eq!(budget.used(), 8);
    drop(image);
    assert_eq!(budget.used(), 0);
    for small in [
        Limits {
            max_pixels: 5,
            ..limits.clone()
        },
        Limits {
            decoded_bytes: 47,
            ..limits.clone()
        },
        Limits {
            file_bytes: 40,
            ..limits.clone()
        },
    ] {
        assert!(load(&path, &small, &budget, 8192, &|| false).is_err());
        assert_eq!(budget.used(), 0);
    }
    assert!(
        load(&path, &limits, &Budget::new(8 * 1024 * 1024), 8192, &|| {
            false
        })
        .is_err()
    );
    let calls = Cell::new(0);
    assert!(
        load(&path, &limits, &budget, 8192, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 4
        })
        .is_err()
    );
    assert_eq!(budget.used(), 0);
}

#[test]
fn huge_unused_layer_data_is_sought_over_with_a_small_input_allowance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.psd");
    let mut psd = Psd::rgb(8);
    psd.width = 1;
    psd.height = 1;
    psd.planes = vec![vec![65535], vec![0], vec![0]];
    let bytes = psd.bytes(0);
    let mut file = File::create(&path).unwrap();
    file.write_all(&bytes[..34]).unwrap();
    let layer_len = 200 * 1024 * 1024_u32;
    file.write_all(&layer_len.to_be_bytes()).unwrap();
    file.write_all(&(layer_len - 8).to_be_bytes()).unwrap();
    file.write_all(&[0, 0]).unwrap();
    file.seek(SeekFrom::Start(38 + u64::from(layer_len) - 4))
        .unwrap();
    file.write_all(&[0; 4]).unwrap();
    file.write_all(&bytes[38..]).unwrap();
    drop(file);
    let limits = Limits {
        file_bytes: 128,
        ..Limits::default()
    };
    let budget = Budget::new(16 * 1024 * 1024);
    let image = load(&path, &limits, &budget, 8192, &|| false).unwrap();
    assert_eq!(image.frames[0].pixels.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(budget.used(), 4);
}

#[test]
fn bounded_deterministic_mutations_never_panic_or_leak_reservations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mutated.psd");
    let limits = Limits {
        max_pixels: 1024,
        decoded_bytes: 4096,
        file_bytes: 4096,
        ..Limits::default()
    };
    let budget = Budget::new(16 * 1024 * 1024);
    let mut seed = 0x823791_u64;
    let mut corpus: Vec<_> = (0..4)
        .map(|compression| Psd::rgb(16).bytes(compression))
        .collect();
    let mut metadata = Psd::rgb(8);
    metadata.planes.push(vec![32768; 6]);
    metadata.alpha_ids(&[0]);
    metadata.negative_layers();
    metadata.marker(b"Mtrn", &[]);
    corpus.push(metadata.bytes(1));
    metadata.resources.extend(resource(
        1039,
        &moxcms::ColorProfile::new_display_p3().encode().unwrap(),
    ));
    corpus.push(metadata.bytes(3));
    for original in corpus {
        for _ in 0..512 {
            let mut bytes = original.clone();
            for _ in 0..4 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let at = (seed >> 32) as usize % bytes.len();
                bytes[at] = (seed >> 24) as u8;
            }
            std::fs::write(&path, bytes).unwrap();
            drop(load(&path, &limits, &budget, 8192, &|| false));
            assert_eq!(budget.used(), 0);
        }
    }
}

#[test]
fn real_layered_psds_match_independent_composite_and_color_references() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/psd");
    let limits = Limits::default();
    for name in [
        "1layer",
        "16bit5x5",
        "4x4_8bit_grayscale",
        "4x4_16bit_grayscale",
        "4x4_8bit_rgba",
        "semi-transparent-layers",
        "transparentbg-gimp",
    ] {
        let decoded = load(
            &fixtures.join(format!("{name}.psd")),
            &limits,
            &Budget::new(limits.ram_bytes),
            8192,
            &|| false,
        )
        .unwrap();
        assert!(
            decoded.warnings.is_empty(),
            "{name}: {:?}",
            decoded.warnings
        );
        let mut reference = image::open(fixtures.join(format!("{name}.png")))
            .unwrap()
            .into_rgba8();
        color::premultiply_linear(reference.as_mut());
        let actual = &decoded.frames[0].pixels;
        assert_eq!(actual.dimensions(), reference.dimensions(), "{name}");
        for (at, (&a, &b)) in actual.as_raw().iter().zip(reference.as_raw()).enumerate() {
            assert!(a.abs_diff(b) <= 1, "{name}: byte {at}: {a} != {b}");
        }
    }
}
