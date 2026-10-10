use crate::{
    animation::frame_delay,
    color,
    limits::{Budget, Limits, Reservation},
};
use anyhow::{Context, Result, bail, ensure};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage};
use std::{
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Raster(ImageFormat),
    Heif,
    Svg,
    Psd,
}

pub fn detect(bytes: &[u8]) -> Result<Format> {
    if bytes.starts_with(b"8BPS") {
        crate::psd::validate_header(bytes)?;
        return Ok(Format::Psd);
    }
    if bytes.len() >= 16 && &bytes[4..8] == b"ftyp" {
        let size = u32::from_be_bytes(bytes[..4].try_into()?) as usize;
        ensure!(size >= 16, "invalid image container header");
        let brands = &bytes[8..size.min(bytes.len()).min(256)];
        if brands
            .as_chunks::<4>()
            .0
            .iter()
            .any(|b| matches!(b, b"avif" | b"avis"))
        {
            return Ok(Format::Raster(ImageFormat::Avif));
        }
        if brands
            .as_chunks::<4>()
            .0
            .iter()
            .any(|b| matches!(b, b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1"))
        {
            return Ok(Format::Heif);
        }
    }
    if let Ok(f) = image::guess_format(bytes)
        && matches!(
            f,
            ImageFormat::Jpeg
                | ImageFormat::Png
                | ImageFormat::WebP
                | ImageFormat::Gif
                | ImageFormat::Bmp
                | ImageFormat::Tiff
                | ImageFormat::Ico
                | ImageFormat::Avif
                | ImageFormat::Pnm
                | ImageFormat::Farbfeld
                | ImageFormat::Dds
                | ImageFormat::Hdr
        )
    {
        return Ok(Format::Raster(f));
    }
    if bytes.starts_with(b"#?RGBE\n") || bytes.starts_with(b"#?RGBE\r\n") {
        return Ok(Format::Raster(ImageFormat::Hdr));
    }
    if bytes.len() >= 44 && bytes.ends_with(TGA_SIGNATURE) && tga_header(bytes) {
        return Ok(Format::Raster(ImageFormat::Tga));
    }
    let header = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    if header.contains("<svg")
        && header
            .trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with('<')
    {
        return Ok(Format::Svg);
    }
    bail!("unsupported or unrecognized image contents")
}

const TGA_SIGNATURE: &[u8] = b"TRUEVISION-XFILE.\0";

/// TGA 1.0 has no identifying signature. Only use its extension after checking
/// the fixed header; TGA 2.0's footer also permits renamed/extensionless files.
fn tga_header(bytes: &[u8]) -> bool {
    let Some(h) = bytes.get(..18) else {
        return false;
    };
    if h[1] > 1 || h[17] & 0xc0 != 0 || h[17] & 0x0f > 8 {
        return false;
    }
    let width = u16::from_le_bytes([h[12], h[13]]);
    let height = u16::from_le_bytes([h[14], h[15]]);
    if width == 0 || height == 0 {
        return false;
    }
    let map_valid =
        h[1] == 0 || (u16::from_le_bytes([h[5], h[6]]) > 0 && matches!(h[7], 15 | 16 | 24 | 32));
    map_valid
        && match h[2] {
            1 | 9 => h[1] == 1 && matches!(h[16], 8 | 16) && h[16] <= h[7],
            2 | 10 => matches!(h[16], 15 | 16 | 24 | 32),
            3 | 11 => matches!(h[16], 8 | 16),
            _ => false,
        }
}

fn detect_with_path(bytes: &[u8], path: &Path) -> Result<Format> {
    detect(bytes).or_else(|error| {
        let tga_extension = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            ["tga", "targa", "icb", "vda", "vst", "tpic"]
                .iter()
                .any(|extension| e.eq_ignore_ascii_case(extension))
        });
        if tga_extension && tga_header(bytes) {
            Ok(Format::Raster(ImageFormat::Tga))
        } else {
            Err(error)
        }
    })
}

pub fn detect_file(path: &Path) -> Result<Format> {
    let mut header = [0; 4096];
    let mut file = File::open(path)?;
    let mut n = 0;
    while n < header.len() {
        let read = file.read(&mut header[n..])?;
        if read == 0 {
            break;
        }
        n += read;
    }
    let result = detect_with_path(&header[..n], path);
    if result.is_ok() || !tga_header(&header[..n]) || file.metadata()?.len() < 44 {
        return result;
    }
    // TGA's footer is at the end, rather than in the bounded prefix. Read only
    // 26 additional bytes, including the two offsets before the signature.
    file.seek(SeekFrom::End(-26))?;
    let mut footer = [0; 26];
    file.read_exact(&mut footer)?;
    if footer.ends_with(TGA_SIGNATURE) {
        Ok(Format::Raster(ImageFormat::Tga))
    } else {
        result
    }
}

#[derive(Debug)]
pub struct Frame {
    pub pixels: RgbaImage,
    pub delay: Duration,
}

#[derive(Debug)]
pub struct Decoded {
    pub original: [u32; 2],
    pub frames: Vec<Frame>,
    /// Total playthroughs; None means infinite.
    pub loops: Option<u32>,
    pub bytes: u64,
    pub elapsed: Duration,
    pub warnings: Vec<String>,
    _memory: Vec<Reservation>,
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub max_dimension: u32,
    pub gpu_bytes: u64,
}

impl Target {
    pub fn fit(&self, width: u32, height: u32) -> [u32; 2] {
        let pixels = f64::from(width) * f64::from(height);
        let scale = (f64::from(self.max_dimension) / f64::from(width.max(height).max(1)))
            .min((self.gpu_bytes as f64 / (pixels * 4.0).max(4.0)).sqrt())
            .min(1.0);
        let mut out = [
            (f64::from(width) * scale).floor().max(1.0) as u32,
            (f64::from(height) * scale).floor().max(1.0) as u32,
        ];
        // The minimum one-pixel axis can defeat the area-based scale for
        // extremely thin images. Enforce the byte bound after rounding.
        let max_pixels = (self.gpu_bytes / 4).max(1);
        if u64::from(out[0]) * u64::from(out[1]) > max_pixels {
            let axis = usize::from(out[1] > out[0]);
            out[axis] = (max_pixels / u64::from(out[1 - axis])).max(1) as u32;
        }
        out
    }
}

pub fn decode(
    path: &Path,
    limits: &Limits,
    budget: &Budget,
    target: Target,
    cancelled: &dyn Fn() -> bool,
) -> Result<Decoded> {
    let start = Instant::now();
    let mut file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let size = file.metadata()?.len();
    let mut signature = [0; 4];
    if size >= 4 {
        file.read_exact(&mut signature)?;
        file.seek(SeekFrom::Start(0))?;
    }
    if &signature == b"8BPS" {
        let (rgba, memory, warnings) = crate::psd::decode(file, size, limits, budget, cancelled)?;
        ensure!(!cancelled(), "request superseded");
        let mut decoded = single(rgba, target, memory, warnings);
        decoded.elapsed = start.elapsed();
        return Ok(decoded);
    }
    ensure!(
        size > 0 && size <= limits.file_bytes,
        "file is empty or exceeds the input size limit"
    );
    let _input_memory = budget.reserve(size + 1)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact((size + 1) as usize)?;
    file.by_ref().take(size + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= size,
        "file changed while reading; try again"
    );
    ensure!(!cancelled(), "request superseded");
    let format = detect_with_path(&bytes, path)?;
    let mut decoded = match format {
        Format::Raster(ImageFormat::Gif) => decode_gif(&bytes, limits, budget, target, cancelled)?,
        Format::Raster(ImageFormat::Avif) => decode_heif(&bytes, limits, budget, target)?,
        Format::Raster(ImageFormat::Hdr) => decode_hdr(&bytes, limits, budget, target)?,
        Format::Raster(f) => decode_raster(&bytes, f, limits, budget, target)?,
        Format::Heif => decode_heif(&bytes, limits, budget, target)?,
        Format::Svg => decode_svg(&bytes, limits, budget, target)?,
        Format::Psd => bail!("file changed while reading; try again"),
    };
    ensure!(!cancelled(), "request superseded");
    decoded.elapsed = start.elapsed();
    Ok(decoded)
}

fn decoder_limits(limits: &Limits) -> image::Limits {
    let mut l = image::Limits::default();
    l.max_image_width = Some(limits.max_pixels.min(u64::from(u32::MAX)) as u32);
    l.max_image_height = l.max_image_width;
    l.max_alloc = Some(limits.decoded_bytes);
    l
}

fn work_memory(limits: &Limits, budget: &Budget, w: u32, h: u32) -> Result<Reservation> {
    let size = limits.rgba_bytes(w, h)?;
    // Room for high-bit-depth decoder output, conversion, orientation, and CMS.
    budget.reserve(size.checked_mul(6).context("work buffer overflow")? + 8 * 1024 * 1024)
}

fn single(
    mut rgba: RgbaImage,
    target: Target,
    mut memory: Reservation,
    warnings: Vec<String>,
) -> Decoded {
    let original = [rgba.width(), rgba.height()];
    color::premultiply_linear(rgba.as_mut());
    let rgba = color::downscale(rgba, target.fit(original[0], original[1]));
    let bytes = rgba.as_raw().len() as u64;
    memory.shrink_to(bytes);
    Decoded {
        original,
        frames: vec![Frame {
            pixels: rgba,
            delay: Duration::ZERO,
        }],
        loops: Some(1),
        bytes,
        elapsed: Duration::ZERO,
        warnings,
        _memory: vec![memory],
    }
}

fn decode_raster(
    bytes: &[u8],
    format: ImageFormat,
    limits: &Limits,
    budget: &Budget,
    target: Target,
) -> Result<Decoded> {
    let dds = (format == ImageFormat::Dds)
        .then(|| dds_layout(bytes))
        .transpose()?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(decoder_limits(limits));
    let mut decoder = reader.into_decoder()?;
    let (w, h) = decoder.dimensions();
    let memory = work_memory(limits, budget, w, h)?;
    ensure!(
        decoder.total_bytes() <= limits.decoded_bytes,
        "native pixel buffer exceeds decoded limit"
    );
    let icc = decoder.icc_profile().unwrap_or(None);
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let mut warnings = Vec::new();
    if icc.is_none()
        && image.color_space() != image::metadata::Cicp::SRGB
        && let Err(e) = image.apply_color_space(image::metadata::Cicp::SRGB, Default::default())
    {
        warnings.push(format!("unsupported color space; assuming sRGB: {e}"));
    }
    let mut rgba = image.into_rgba8();
    if let Some(layout) = dds {
        if layout.bc1 && !layout.opaque {
            restore_bc1_alpha(&mut rgba, &bytes[layout.offset..])?;
        }
        if layout.opaque {
            for p in rgba.as_mut().as_chunks_mut::<4>().0 {
                p[3] = 255;
            }
        }
    }
    if let Some(icc) = icc
        && let Err(e) = color::convert_icc(rgba.as_mut(), &icc)
    {
        warnings.push(format!(
            "invalid/unsupported ICC profile; assuming sRGB: {e}"
        ));
    }
    Ok(single(rgba, target, memory, warnings))
}

struct DdsLayout {
    offset: usize,
    bc1: bool,
    opaque: bool,
}

fn dds_layout(bytes: &[u8]) -> Result<DdsLayout> {
    ensure!(bytes.len() >= 128, "truncated DDS header");
    let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    ensure!(
        word(24) <= 1 && word(112) & (0xfe00 | 0x20_0000) == 0,
        "DDS cubemaps and volume textures are unsupported"
    );
    let mut layout = DdsLayout {
        offset: 128,
        bc1: &bytes[84..88] == b"DXT1",
        opaque: false,
    };
    if &bytes[84..88] == b"DX10" {
        ensure!(bytes.len() >= 148, "truncated DDS DX10 header");
        ensure!(
            word(132) == 3 && word(136) == 0 && word(140) == 1,
            "DDS requires a single two-dimensional texture"
        );
        ensure!(
            matches!(word(144), 0 | 1 | 3),
            "DDS premultiplied/custom alpha is unsupported"
        );
        layout.offset = 148;
        layout.bc1 = matches!(word(128), 70..=72);
        layout.opaque = word(144) == 3;
    }
    Ok(layout)
}

/// image's BC1 decoder returns RGB, dropping BC1's transparent selector. Keep
/// its RGB decoding and restore only that alpha bit from the original blocks.
fn restore_bc1_alpha(rgba: &mut RgbaImage, bytes: &[u8]) -> Result<()> {
    let blocks_wide = rgba.width() as usize / 4;
    let count = blocks_wide * (rgba.height() as usize / 4);
    let blocks = bytes.get(..count * 8).context("truncated DDS BC1 blocks")?;
    for (index, block) in blocks.as_chunks::<8>().0.iter().enumerate() {
        if u16::from_le_bytes([block[0], block[1]]) > u16::from_le_bytes([block[2], block[3]]) {
            continue;
        }
        let selectors = u32::from_le_bytes(block[4..8].try_into()?);
        for pixel in 0..16 {
            if (selectors >> (pixel * 2)) & 3 == 3 {
                let x = (index % blocks_wide) as u32 * 4 + pixel % 4;
                let y = (index / blocks_wide) as u32 * 4 + pixel / 4;
                rgba.get_pixel_mut(x, y)[3] = 0;
            }
        }
    }
    Ok(())
}

fn decode_hdr(bytes: &[u8], limits: &Limits, budget: &Budget, target: Target) -> Result<Decoded> {
    use image::codecs::hdr::HdrDecoder;
    // image's strict parser accepts only RADIANCE/LF. Normalize the bounded
    // ASCII header, leaving compressed pixel bytes untouched and validation strict.
    const HEADER_LIMIT: usize = 64 * 1024;
    let _header_memory = budget.reserve(HEADER_LIMIT as u64)?;
    let mut header = Vec::with_capacity(HEADER_LIMIT);
    let mut offset = 0;
    let mut dimensions_next = false;
    let mut end = None;
    for (index, line) in bytes[..bytes.len().min(HEADER_LIMIT)]
        .split_inclusive(|&b| b == b'\n')
        .enumerate()
    {
        let line = line
            .strip_suffix(b"\n")
            .context("HDR header exceeds 64 KiB or is truncated")?;
        offset += line.len() + 1;
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let normalized = if index == 0 { b"#?RADIANCE" } else { line };
        ensure!(
            header.len() + normalized.len() < HEADER_LIMIT,
            "HDR header exceeds 64 KiB"
        );
        header.extend(normalized);
        header.push(b'\n');
        if dimensions_next {
            end = Some(offset);
            break;
        }
        dimensions_next = index > 0 && line.is_empty();
    }
    let offset = end.context("HDR header exceeds 64 KiB or has no terminator")?;
    let reader = Cursor::new(header).chain(Cursor::new(&bytes[offset..]));
    let mut decoder = HdrDecoder::new(reader)?;
    decoder.set_limits(decoder_limits(limits))?;
    let (width, height) = decoder.dimensions();
    let memory = work_memory(limits, budget, width, height)?;
    ensure!(
        decoder.total_bytes() <= limits.decoded_bytes,
        "HDR float buffer exceeds decoded limit"
    );
    let image = DynamicImage::from_decoder(decoder)?.into_rgb32f();
    Ok(single(
        color::tone_map_hdr(&image),
        target,
        memory,
        Vec::new(),
    ))
}

fn decode_gif(
    bytes: &[u8],
    limits: &Limits,
    budget: &Budget,
    target: Target,
    cancelled: &dyn Fn() -> bool,
) -> Result<Decoded> {
    let loops = gif_loops(bytes, limits.max_frames)?;
    let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes))?;
    decoder.set_limits(decoder_limits(limits))?;
    let (w, h) = decoder.dimensions();
    let original = [w, h];
    let frame_bytes = limits.rgba_bytes(w, h)?;
    let _work = work_memory(limits, budget, w, h)?;
    let mut frames = Vec::new();
    let mut memory = Vec::new();
    let mut used: u64 = 0;
    let output_size = target.fit(w, h);
    // Reserve each frame BEFORE asking the decoder to allocate it.
    let mut iterator = decoder.into_frames();
    loop {
        ensure!(!cancelled(), "request superseded");
        let reservation = budget.reserve(frame_bytes)?;
        let Some(frame) = iterator.next() else {
            break;
        };
        ensure!(
            frames.len() < limits.max_frames,
            "GIF exceeds the {} frame limit",
            limits.max_frames
        );
        let frame = frame?;
        let (n, d) = frame.delay().numer_denom_ms();
        let mut rgba = frame.into_buffer();
        color::premultiply_linear(rgba.as_mut());
        let rgba = color::downscale(rgba, output_size);
        used = used
            .checked_add(rgba.as_raw().len() as u64)
            .context("GIF size overflow")?;
        ensure!(
            used <= limits.decoded_bytes,
            "GIF exceeds total decoded image limit"
        );
        let mut reservation = reservation;
        reservation.shrink_to(rgba.as_raw().len() as u64);
        memory.push(reservation);
        frames.push(Frame {
            pixels: rgba,
            delay: frame_delay(n, d),
        });
    }
    ensure!(!frames.is_empty(), "GIF contains no frames");
    Ok(Decoded {
        original,
        frames,
        loops,
        bytes: used,
        elapsed: Duration::ZERO,
        warnings: vec![],
        _memory: memory,
    })
}

/// Read application extensions using GIF block boundaries. The image crate
/// reports an absent repeat extension as infinite; absent means play once.
fn gif_loops(bytes: &[u8], max_frames: usize) -> Result<Option<u32>> {
    let mut i = 13;
    ensure!(bytes.len() >= i, "truncated GIF header");
    let width = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
    let height = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let mut loops = Some(1);
    let mut frames = 0;
    if bytes[10] & 0x80 != 0 {
        i += 3 * (1_usize << ((bytes[10] & 7) + 1));
    }
    while i < bytes.len() {
        let kind = bytes[i];
        i += 1;
        match kind {
            0x21 => {
                let label = *bytes.get(i).context("truncated GIF extension")?;
                i += 1;
                if label == 0xff
                    && bytes.get(i) == Some(&11)
                    && let Some(name) = bytes.get(i + 1..i + 12)
                    && (name == b"NETSCAPE2.0" || name == b"ANIMEXTS1.0")
                    && bytes.get(i + 12..i + 14) == Some(&[3, 1])
                {
                    let low = *bytes.get(i + 14).context("truncated GIF repeat count")?;
                    let high = *bytes.get(i + 15).context("truncated GIF repeat count")?;
                    let n = u16::from_le_bytes([low, high]);
                    loops = if n == 0 { None } else { Some(u32::from(n) + 1) };
                }
            }
            0x2c => {
                let desc = bytes.get(i..i + 9).context("truncated GIF frame")?;
                let x = u32::from(u16::from_le_bytes([desc[0], desc[1]]));
                let y = u32::from(u16::from_le_bytes([desc[2], desc[3]]));
                let w = u32::from(u16::from_le_bytes([desc[4], desc[5]]));
                let h = u32::from(u16::from_le_bytes([desc[6], desc[7]]));
                // image's GIF decoder permits frames outside the canvas by default.
                // Reject them before allocating a frame larger than our work budget.
                ensure!(
                    w > 0 && h > 0 && x + w <= width && y + h <= height,
                    "GIF frame lies outside its canvas"
                );
                frames += 1;
                ensure!(frames <= max_frames, "GIF exceeds the frame limit");
                i += 9;
                if desc[8] & 0x80 != 0 {
                    i += 3 * (1_usize << ((desc[8] & 7) + 1));
                }
                i += 1; // LZW code size
            }
            0x3b => return Ok(loops),
            _ => bail!("invalid GIF block"),
        }
        // Both image and extension payloads are length-prefixed subblocks.
        loop {
            let size = *bytes.get(i).context("truncated GIF subblock")?;
            i += 1;
            if size == 0 {
                break;
            }
            i += usize::from(size);
            ensure!(i <= bytes.len(), "truncated GIF subblock");
        }
    }
    bail!("GIF has no trailer")
}

/// The only boundary to libheif; native objects remain on the decoding worker.
fn decode_heif(bytes: &[u8], limits: &Limits, budget: &Budget, target: Target) -> Result<Decoded> {
    use libheif_rs::{ColorSpace, DecodingOptions, HeifContext, LibHeif, RgbChroma};
    static LIBRARY: std::sync::OnceLock<Result<LibHeif, String>> = std::sync::OnceLock::new();
    let lib = LIBRARY
        .get_or_init(|| LibHeif::new_checked().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| anyhow::anyhow!("cannot initialize libheif: {e}"))?;
    let _metadata = budget.reserve(8 * 1024 * 1024)?;
    let mut ctx = HeifContext::new()?;
    let mut security = ctx.security_limits();
    security.set_max_image_size_pixels(limits.max_pixels);
    security.set_max_memory_block_size(limits.decoded_bytes);
    security.set_max_total_memory(limits.ram_bytes.min(limits.decoded_bytes.saturating_mul(4)));
    security.set_max_color_profile_size(color::MAX_ICC_BYTES as u32);
    security.set_max_number_of_tiles(4096);
    security.set_max_items(4096);
    ctx.set_security_limits(&security)?;
    ctx.read_bytes(bytes)?;
    ctx.set_max_decoding_threads(1);
    let handle = ctx.primary_image_handle()?;
    let mut working_dimensions = [handle.width(), handle.height()];
    if handle.ispe_width() > 0 && handle.ispe_height() > 0 {
        let raw = [handle.ispe_width() as u32, handle.ispe_height() as u32];
        limits.rgba_bytes(raw[0], raw[1])?;
        if u64::from(raw[0]) * u64::from(raw[1])
            > u64::from(working_dimensions[0]) * u64::from(working_dimensions[1])
        {
            working_dimensions = raw;
        }
    }
    let memory = work_memory(limits, budget, working_dimensions[0], working_dimensions[1])?;
    let mut options = DecodingOptions::new().context("cannot allocate HEIF decoding options")?;
    options.set_strict_decoding(true);
    let image = lib.decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), Some(options))?;
    let plane = image
        .planes()
        .interleaved
        .context("HEIF decoder did not produce RGBA")?;
    limits.rgba_bytes(plane.width, plane.height)?;
    let mut rgba = RgbaImage::new(plane.width, plane.height);
    let row = plane.width as usize * 4;
    ensure!(
        plane.stride >= row && plane.data.len() >= plane.stride * plane.height as usize,
        "invalid HEIF plane stride"
    );
    for (dst, src) in rgba
        .as_mut()
        .chunks_exact_mut(row)
        .zip(plane.data.chunks(plane.stride))
    {
        dst.copy_from_slice(&src[..row]);
    }
    if image.is_premultiplied_alpha() {
        color::unpremultiply_srgb(rgba.as_mut());
    }
    let mut warnings = Vec::new();
    if let Some(icc) = handle.color_profile_raw() {
        if let Err(e) = color::convert_icc(rgba.as_mut(), &icc.data) {
            warnings.push(format!("HEIF ICC: {e}; assuming sRGB"));
        }
    } else if let Some(nclx) = handle.color_profile_nclx() {
        let cicp = moxcms::CicpProfile {
            color_primaries: (nclx.color_primaries() as u8).try_into()?,
            transfer_characteristics: (nclx.transfer_characteristics() as u8).try_into()?,
            matrix_coefficients: moxcms::MatrixCoefficients::Identity,
            full_range: true,
        };
        if cicp.color_primaries == moxcms::CicpColorPrimaries::Bt709
            && cicp.transfer_characteristics == moxcms::TransferCharacteristics::Srgb
        {
            // Native YCbCr conversion already produced sRGB; avoid a redundant CMS transform.
        } else if moxcms::ColorPrimaries::try_from(cicp.color_primaries).is_ok()
            && moxcms::ToneReprCurve::try_from(cicp.transfer_characteristics).is_ok()
        {
            let mut profile = moxcms::ColorProfile::new_srgb();
            // moxcms 0.9.1 updates the profile but returns false even on success.
            profile.update_rgb_colorimetry_from_cicp(cicp);
            if let Err(e) = color::convert_profile(rgba.as_mut(), &profile) {
                warnings.push(format!("HEIF NCLX: {e}; assuming sRGB"));
            }
        } else {
            warnings.push("unsupported HEIF NCLX; assuming sRGB".into());
        }
    }
    // libheif applies the container's rotation/mirroring exactly once.
    Ok(single(rgba, target, memory, warnings))
}

fn decode_svg(bytes: &[u8], limits: &Limits, budget: &Budget, target: Target) -> Result<Decoded> {
    use resvg::{tiny_skia, usvg};
    ensure!(
        bytes.len() <= 1024 * 1024,
        "SVG exceeds the 1 MiB source limit"
    );
    let _parser_memory = budget.reserve(32 * 1024 * 1024)?;
    let xml = std::str::from_utf8(bytes).context("SVG must be UTF-8")?;
    let document = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 10_000,
            ..Default::default()
        },
    )?;
    for node in document.descendants().filter(|n| n.is_element()) {
        ensure!(
            node.ancestors().take(66).count() <= 64,
            "SVG nesting exceeds 64"
        );
        ensure!(
            node.tag_name().name() != "use",
            "SVG use expansion exceeds the supported safe subset"
        );
    }
    let text_bytes: usize = document
        .descendants()
        .filter(|n| {
            n.is_text()
                && n.ancestors()
                    .any(|p| p.is_element() && p.tag_name().name() == "text")
        })
        .map(|n| n.text().map_or(0, str::len))
        .sum();
    ensure!(
        text_bytes <= 16_384,
        "SVG text exceeds the 16 KiB shaping limit"
    );
    static FONTS: std::sync::OnceLock<std::sync::Arc<usvg::fontdb::Database>> =
        std::sync::OnceLock::new();
    let needs_fonts = document
        .descendants()
        .any(|n| n.is_element() && n.tag_name().name() == "text");
    let fonts = if needs_fonts {
        FONTS
            .get_or_init(|| {
                let mut db = usvg::fontdb::Database::new();
                db.load_system_fonts();
                std::sync::Arc::new(db)
            })
            .clone()
    } else {
        std::sync::Arc::new(usvg::fontdb::Database::new())
    };
    let options = usvg::Options {
        fontdb: fonts,
        // Disable file/network/embedded image resolution. SVG is vector-only.
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            resolve_data: Box::new(|_, _, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options)?;
    let size = tree.size().to_int_size();
    let memory = work_memory(limits, budget, size.width(), size.height())?;
    let out = target.fit(size.width(), size.height());
    // resvg may create offscreen layers up to 16x the canvas area. Account
    // conservatively before rendering; reject expensive filter/mask/pattern graphs.
    ensure!(
        tree.filters().is_empty() && tree.masks().is_empty(),
        "SVG filters and masks exceed the supported safe subset"
    );
    let depth = svg_layer_depth(tree.root(), 0)?;
    let layer_bytes = u64::from(out[0]) * u64::from(out[1]) * 4;
    let _layers = budget.reserve(layer_bytes.saturating_mul(depth as u64).saturating_mul(16))?;
    let mut pixmap =
        tiny_skia::Pixmap::new(out[0], out[1]).context("cannot allocate SVG surface")?;
    let transform = tiny_skia::Transform::from_scale(
        out[0] as f32 / size.width() as f32,
        out[1] as f32 / size.height() as f32,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut rgba =
        RgbaImage::from_raw(out[0], out[1], pixmap.take()).context("invalid SVG buffer")?;
    color::unpremultiply_srgb(rgba.as_mut());
    let mut decoded = single(rgba, target, memory, Vec::new());
    decoded.original = [size.width(), size.height()];
    Ok(decoded)
}

fn svg_layer_depth(group: &resvg::usvg::Group, depth: usize) -> Result<usize> {
    use resvg::usvg::{Node, Paint};
    ensure!(depth <= 64, "SVG layer nesting exceeds 64");
    let current = usize::from(group.should_isolate());
    let mut deepest = 0;
    for node in group.children() {
        let nested = match node {
            Node::Group(g) => svg_layer_depth(g, depth + 1)?,
            Node::Text(t) => svg_layer_depth(t.flattened(), depth + 1)?,
            Node::Path(p) => {
                let pattern = p
                    .fill()
                    .is_some_and(|f| matches!(f.paint(), Paint::Pattern(_)))
                    || p.stroke()
                        .is_some_and(|s| matches!(s.paint(), Paint::Pattern(_)));
                ensure!(!pattern, "SVG patterns exceed the supported safe subset");
                0
            }
            _ => 0,
        };
        deepest = deepest.max(nested);
    }
    Ok(current + deepest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn content_detection_and_short_headers() {
        for n in 0..32 {
            assert!(detect(&vec![0; n]).is_err());
        }
        assert_eq!(
            detect(b"\x89PNG\r\n\x1a\n").unwrap(),
            Format::Raster(ImageFormat::Png)
        );
        assert_eq!(
            detect(b"\xef\xbb\xbf <?xml version='1.0'?><svg xmlns='http://www.w3.org/2000/svg'/>")
                .unwrap(),
            Format::Svg
        );
        assert_eq!(
            detect(b"\0\0\0\x18ftypheic\0\0\0\0mif1heic").unwrap(),
            Format::Heif
        );
        assert_eq!(
            detect(b"\0\0\0\x18ftypmif1\0\0\0\0avifmif1").unwrap(),
            Format::Raster(ImageFormat::Avif)
        );
    }
    #[test]
    fn texture_downscale_obeys_both_limits() {
        let t = Target {
            max_dimension: 1024,
            gpu_bytes: 1024 * 1024 * 4,
        };
        assert_eq!(t.fit(4000, 2000), [1024, 512]);
        let t = Target {
            max_dimension: 8192,
            gpu_bytes: 4 * 100 * 100,
        };
        assert_eq!(t.fit(1000, 1000), [100, 100]);
        let t = Target {
            max_dimension: 8192,
            gpu_bytes: 4,
        };
        assert_eq!(t.fit(10000, 1), [1, 1]);
        assert_eq!(t.fit(1, 10000), [1, 1]);
    }
}
