//! Bounded PSD v1 composite reader. Layer pixels are never loaded or rendered.
//! See Adobe's Photoshop File Formats Specification and docs/limitations.md.
use crate::{
    color,
    limits::{Budget, Limits, Reservation},
};
use anyhow::{Context, Result, ensure};
use flate2::{Decompress, FlushDecompress, Status};
use image::RgbaImage;
use moxcms::{ColorProfile, DataColorSpace};
use std::{
    fs::File,
    io::{BufReader, Read},
};

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_BLOCKS: usize = 10_000;
const MISSING_COMPOSITE: &str =
    "PSD has no usable merged image; save with 'Maximize PSD and PSB File Compatibility' enabled";

struct Header {
    width: u32,
    height: u32,
    channels: u16,
    depth: u16,
    mode: u16,
}

impl Header {
    fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() >= 26, "truncated PSD header");
        ensure!(&bytes[..4] == b"8BPS", "invalid PSD signature");
        let short = |at| u16::from_be_bytes([bytes[at], bytes[at + 1]]);
        ensure!(
            short(4) == 1,
            "PSB/unknown Photoshop versions are unsupported"
        );
        ensure!(bytes[6..12] == [0; 6], "invalid PSD reserved header bytes");
        let header = Self {
            channels: short(12),
            height: u32::from_be_bytes(bytes[14..18].try_into()?),
            width: u32::from_be_bytes(bytes[18..22].try_into()?),
            depth: short(22),
            mode: short(24),
        };
        ensure!(
            (1..=56).contains(&header.channels),
            "invalid PSD channel count"
        );
        ensure!(
            (1..=30_000).contains(&header.width) && (1..=30_000).contains(&header.height),
            "invalid PSD dimensions"
        );
        ensure!(matches!(header.depth, 1 | 8 | 16 | 32), "invalid PSD depth");
        ensure!(
            matches!(header.mode, 0..=4 | 7..=9),
            "invalid PSD color mode"
        );
        Ok(header)
    }
    fn colors(&self) -> u16 {
        if self.mode == 1 { 1 } else { 3 }
    }
    fn row_bytes(&self) -> usize {
        self.width as usize * usize::from(self.depth / 8)
    }
}

pub(crate) fn validate_header(bytes: &[u8]) -> Result<()> {
    Header::parse(bytes).map(|_| ())
}

/// Counts bytes actually inspected; seeking across unused layer/resource data
/// does not consume the encoded-input allowance. All sections have file bounds.
struct Input {
    file: BufReader<File>,
    size: u64,
    pos: u64,
    read: u64,
    limit: u64,
}

impl Input {
    fn end(&self, len: u64, parent: u64) -> Result<u64> {
        let end = self.pos.checked_add(len).context("PSD section overflow")?;
        ensure!(
            end <= parent && parent <= self.size,
            "truncated PSD section"
        );
        Ok(end)
    }
    fn read_into(&mut self, out: &mut [u8], end: u64) -> Result<()> {
        self.end(out.len() as u64, end)?;
        ensure!(
            out.len() as u64 <= self.limit.saturating_sub(self.read),
            "PSD inspected data exceeds the input size limit"
        );
        self.file.read_exact(out).context("truncated PSD data")?;
        self.pos += out.len() as u64;
        self.read += out.len() as u64;
        Ok(())
    }
    fn bytes<const N: usize>(&mut self, end: u64) -> Result<[u8; N]> {
        let mut bytes = [0; N];
        self.read_into(&mut bytes, end)?;
        Ok(bytes)
    }
    fn short(&mut self, end: u64) -> Result<u16> {
        Ok(u16::from_be_bytes(self.bytes(end)?))
    }
    fn word(&mut self, end: u64) -> Result<u32> {
        Ok(u32::from_be_bytes(self.bytes(end)?))
    }
    fn section(&mut self, parent: u64) -> Result<u64> {
        let len = self.word(parent)?;
        self.end(u64::from(len), parent)
    }
    fn skip_to(&mut self, end: u64) -> Result<()> {
        ensure!(
            end >= self.pos && end <= self.size,
            "invalid PSD section offset"
        );
        self.file.seek_relative((end - self.pos) as i64)?;
        self.pos = end;
        Ok(())
    }
    fn padding(&mut self, len: u64, alignment: u64, end: u64) -> Result<()> {
        let count = (alignment - len % alignment) % alignment;
        let to = self.end(count, end)?;
        self.skip_to(to)
    }
}

fn zeros<T: Clone + Default>(len: usize) -> Result<Vec<T>> {
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(len)?;
    buffer.resize(len, T::default());
    Ok(buffer)
}

#[derive(Default)]
struct Metadata {
    icc: Option<Vec<u8>>,
    alpha: Option<u16>,
    alpha_ids: bool,
}

fn resources(input: &mut Input, header: &Header, cancelled: &dyn Fn() -> bool) -> Result<Metadata> {
    let end = input.section(input.size)?;
    let mut meta = Metadata::default();
    let mut count = 0;
    while input.pos < end {
        ensure!(!cancelled(), "request superseded");
        count += 1;
        ensure!(count <= MAX_BLOCKS, "PSD has too many image resources");
        let signature = input.bytes::<4>(end)?;
        ensure!(
            matches!(&signature, b"8BIM" | b"MeSa" | b"AgHg" | b"PHUT" | b"DCSR"),
            "invalid PSD resource signature"
        );
        let id = input.short(end)?;
        let name_len = u64::from(input.bytes::<1>(end)?[0]);
        let name_end = input.end(name_len, end)?;
        input.skip_to(name_end)?;
        input.padding(name_len + 1, 2, end)?;
        let len = u64::from(input.word(end)?);
        let data_end = input.end(len, end)?;
        match if &signature == b"8BIM" { id } else { 0 } {
            1039 => {
                ensure!(meta.icc.is_none(), "duplicate PSD ICC profile");
                ensure!(
                    len <= color::MAX_ICC_BYTES as u64,
                    "PSD ICC profile exceeds 1 MiB"
                );
                let mut bytes = zeros(len as usize)?;
                input.read_into(&mut bytes, data_end)?;
                meta.icc = Some(bytes);
            }
            1057 => {
                ensure!(
                    input.word(data_end)? == 1,
                    "unsupported PSD Version Info resource"
                );
                let merged = input.bytes::<1>(data_end)?[0];
                ensure!(merged <= 1, "invalid PSD merged-image flag");
                ensure!(merged == 1, MISSING_COMPOSITE);
            }
            1053 => {
                ensure!(!meta.alpha_ids, "duplicate PSD alpha identifiers");
                meta.alpha_ids = true;
                ensure!(
                    len % 4 == 0 && len / 4 <= u64::from(header.channels - header.colors()),
                    "invalid PSD alpha identifiers"
                );
                for index in 0..len / 4 {
                    if input.word(data_end)? == 0 {
                        ensure!(
                            meta.alpha.is_none(),
                            "ambiguous PSD transparency identifiers"
                        );
                        meta.alpha = Some(header.channels - (len / 4) as u16 + index as u16);
                    }
                }
            }
            _ => {}
        }
        input.skip_to(data_end)?;
        input.padding(len, 2, end)?;
    }
    Ok(meta)
}

fn layer_flags(
    input: &mut Input,
    depth: u16,
    cancelled: &dyn Fn() -> bool,
) -> Result<(bool, bool)> {
    let end = input.section(input.size)?;
    if input.pos == end {
        return Ok((false, false));
    }
    let info_end = input.section(end)?;
    let mut first_alpha = false;
    let mut marked_alpha = false;
    if input.pos < info_end {
        first_alpha = i16::from_be_bytes(input.bytes(info_end)?) < 0;
        input.skip_to(info_end)?;
    }
    // Some old writers stop after the layer info instead of adding a mask word.
    if input.pos == end {
        return Ok((first_alpha, marked_alpha));
    }
    let mask_end = input.section(end)?;
    input.skip_to(mask_end)?;
    let mut count = 0;
    while input.pos < end {
        ensure!(!cancelled(), "request superseded");
        count += 1;
        ensure!(
            count <= MAX_BLOCKS,
            "PSD has too many additional layer blocks"
        );
        if end - input.pos < 12 {
            let mut padding = [0; 11];
            let len = (end - input.pos) as usize;
            input.read_into(&mut padding[..len], end)?;
            ensure!(
                padding[..len].iter().all(|&b| b == 0),
                "invalid PSD layer padding"
            );
            break;
        }
        let signature = input.bytes::<4>(end)?;
        ensure!(
            matches!(&signature, b"8BIM" | b"8B64"),
            "invalid PSD additional layer signature"
        );
        let key = input.bytes::<4>(end)?;
        let len = u64::from(input.word(end)?);
        let data_end = input.end(len, end)?;
        if (depth == 8 && &key == b"Mtrn") || (depth == 16 && &key == b"Mt16") {
            marked_alpha = true;
        }
        if (depth == 8 && &key == b"Layr") || (depth == 16 && &key == b"Lr16") {
            first_alpha |= i16::from_be_bytes(input.bytes(data_end)?) < 0;
        }
        input.skip_to(data_end)?;
        input.padding(len, 4, end)?;
    }
    Ok((first_alpha, marked_alpha))
}

enum Pixels {
    Eight(Vec<u8>),
    Sixteen(Vec<u16>),
}

impl Pixels {
    fn new(header: &Header) -> Result<Self> {
        let count = header.width as usize * header.height as usize * 4;
        if header.depth == 8 {
            let mut pixels: Vec<u8> = zeros(count)?;
            for p in pixels.as_chunks_mut::<4>().0 {
                p[3] = 255;
            }
            Ok(Self::Eight(pixels))
        } else {
            let mut pixels: Vec<u16> = zeros(count)?;
            for p in pixels.as_chunks_mut::<4>().0 {
                p[3] = 65535;
            }
            Ok(Self::Sixteen(pixels))
        }
    }
    fn row(&mut self, header: &Header, alpha: Option<u16>, channel: u16, y: u32, row: &[u8]) {
        let offset = if channel < header.colors() {
            channel as usize
        } else if Some(channel) == alpha {
            3
        } else {
            return; // Saved masks/spot channels are not the composite's opacity.
        };
        let start = y as usize * header.width as usize * 4;
        let end = start + header.width as usize * 4;
        match self {
            Self::Eight(pixels) => {
                for (p, &value) in pixels[start..end]
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(row)
                {
                    p[offset] = value;
                }
            }
            Self::Sixteen(pixels) => {
                for (p, value) in pixels[start..end]
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(row.as_chunks::<2>().0)
                {
                    p[offset] = u16::from_be_bytes(*value);
                }
            }
        }
    }
    fn finish(self, header: &Header, meta: Metadata) -> Result<(RgbaImage, Vec<String>)> {
        let mut warnings = Vec::new();
        let profile = meta
            .icc
            .as_ref()
            .map(|bytes| -> Result<ColorProfile> {
                let profile = ColorProfile::new_from_slice(bytes)?;
                let expected = if header.mode == 1 {
                    DataColorSpace::Gray
                } else {
                    DataColorSpace::Rgb
                };
                ensure!(
                    profile.color_space == expected,
                    "PSD ICC color space does not match the document"
                );
                Ok(profile)
            })
            .transpose();
        let profile = match profile {
            Ok(profile) => profile,
            Err(e) => {
                warnings.push(format!(
                    "invalid/unsupported PSD ICC profile; assuming sRGB: {e}"
                ));
                None
            }
        };
        let pixels = match self {
            Self::Eight(mut pixels) => {
                prepare(&mut pixels, header.mode == 1, meta.alpha.is_some(), 255);
                if let Some(profile) = &profile
                    && let Err(e) = color::convert_profile(&mut pixels, profile)
                {
                    warnings.push(format!("unsupported PSD ICC transform; assuming sRGB: {e}"));
                }
                pixels
            }
            Self::Sixteen(mut pixels) => {
                prepare(&mut pixels, header.mode == 1, meta.alpha.is_some(), 65535);
                if let Some(profile) = &profile
                    && let Err(e) = color::convert_profile16(&mut pixels, profile)
                {
                    warnings.push(format!("unsupported PSD ICC transform; assuming sRGB: {e}"));
                }
                let mut rgba = zeros(pixels.len())?;
                for (dst, value) in rgba.iter_mut().zip(pixels) {
                    *dst = ((u32::from(value) + 128) / 257) as u8;
                }
                rgba
            }
        };
        let rgba = RgbaImage::from_raw(header.width, header.height, pixels)
            .context("invalid PSD pixel buffer")?;
        Ok((rgba, warnings))
    }
}

/// Photoshop composites transparent colors over white in source sample space.
/// Remove that matte before ICC conversion and Vysyn's linear premultiplication.
fn prepare<T: Copy + From<u8> + Into<u64> + TryFrom<u64>>(
    pixels: &mut [T],
    gray: bool,
    transparent: bool,
    max: u64,
) {
    for p in pixels.as_chunks_mut::<4>().0 {
        if gray {
            p[1] = p[0];
            p[2] = p[0];
        }
        if transparent {
            let alpha = p[3].into();
            for value in &mut p[..3] {
                let unmatte = ((*value).into().saturating_sub(max - alpha) * max + alpha / 2)
                    .checked_div(alpha)
                    .unwrap_or(0)
                    .min(max);
                *value = T::try_from(unmatte).unwrap_or_else(|_| T::from(0));
            }
        }
    }
}

fn unpack_rle(packed: &[u8], row: &mut [u8]) -> Result<()> {
    let (mut src, mut dst) = (0, 0);
    while src < packed.len() {
        let code = packed[src];
        src += 1;
        match code {
            0..=127 => {
                let len = usize::from(code) + 1;
                ensure!(
                    src + len <= packed.len() && dst + len <= row.len(),
                    "invalid PSD RLE literal"
                );
                row[dst..dst + len].copy_from_slice(&packed[src..src + len]);
                src += len;
                dst += len;
            }
            128 => {} // PackBits no-op, bounded by the encoded row's byte count.
            _ => {
                let len = 257 - usize::from(code);
                ensure!(
                    src < packed.len() && dst + len <= row.len(),
                    "invalid PSD RLE run"
                );
                row[dst..dst + len].fill(packed[src]);
                src += 1;
                dst += len;
            }
        }
    }
    ensure!(dst == row.len(), "truncated PSD RLE row");
    Ok(())
}

fn prediction(row: &mut [u8], depth: u16) {
    if depth == 8 {
        let mut previous = 0_u8;
        for value in row {
            *value = value.wrapping_add(previous);
            previous = *value;
        }
    } else {
        let mut previous = 0_u16;
        for bytes in row.as_chunks_mut::<2>().0 {
            previous = u16::from_be_bytes(*bytes).wrapping_add(previous);
            *bytes = previous.to_be_bytes();
        }
    }
}

struct Zip<'a> {
    source: &'a mut Input,
    inflater: Decompress,
    input: Vec<u8>,
    pos: usize,
    len: usize,
    ended: bool,
}

impl<'a> Zip<'a> {
    fn new(source: &'a mut Input) -> Result<Self> {
        Ok(Self {
            source,
            inflater: Decompress::new(true),
            input: zeros(32 * 1024)?,
            pos: 0,
            len: 0,
            ended: false,
        })
    }
    fn step(&mut self, output: &mut [u8], cancelled: &dyn Fn() -> bool) -> Result<usize> {
        ensure!(!cancelled(), "request superseded");
        if self.ended {
            return Ok(0);
        }
        if self.pos == self.len {
            self.len = (self.source.size - self.source.pos).min(self.input.len() as u64) as usize;
            self.source
                .read_into(&mut self.input[..self.len], self.source.size)?;
            self.pos = 0;
        }
        let (before_in, before_out) = (self.inflater.total_in(), self.inflater.total_out());
        let status = self
            .inflater
            .decompress(
                &self.input[self.pos..self.len],
                output,
                FlushDecompress::None,
            )
            .context("invalid PSD ZIP stream/checksum")?;
        let consumed = (self.inflater.total_in() - before_in) as usize;
        let produced = (self.inflater.total_out() - before_out) as usize;
        self.pos += consumed;
        self.ended = status == Status::StreamEnd;
        ensure!(
            self.ended || consumed > 0 || produced > 0,
            "truncated/stalled PSD ZIP stream"
        );
        Ok(produced)
    }
    fn row(&mut self, row: &mut [u8], cancelled: &dyn Fn() -> bool) -> Result<()> {
        let mut written = 0;
        while written < row.len() {
            ensure!(!self.ended, "truncated PSD ZIP pixels");
            written += self.step(&mut row[written..], cancelled)?;
        }
        Ok(())
    }
    fn finish(&mut self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        while !self.ended {
            ensure!(
                self.step(&mut [0], cancelled)? == 0,
                "PSD ZIP expands beyond expected pixels"
            );
        }
        ensure!(
            self.pos == self.len && self.source.pos == self.source.size,
            "trailing PSD ZIP data"
        );
        Ok(())
    }
}

pub(crate) fn decode(
    file: File,
    size: u64,
    limits: &Limits,
    budget: &Budget,
    cancelled: &dyn Fn() -> bool,
) -> Result<(RgbaImage, Reservation, Vec<String>)> {
    ensure!(
        size <= MAX_FILE_BYTES,
        "PSD file exceeds the 2 GiB format limit"
    );
    ensure!(!cancelled(), "request superseded");
    let mut input = Input {
        file: BufReader::new(file),
        size,
        pos: 0,
        read: 0,
        limit: limits.file_bytes,
    };
    let header = Header::parse(&input.bytes::<26>(size)?)?;
    ensure!(
        matches!(header.depth, 8 | 16),
        "PSD supports only 8/16-bit samples"
    );
    ensure!(
        matches!(header.mode, 1 | 3),
        "PSD supports only RGB/grayscale; CMYK/Lab and other modes are unsupported"
    );
    ensure!(
        header.channels >= header.colors(),
        "PSD is missing color channels"
    );
    let rgba_bytes = limits.rgba_bytes(header.width, header.height)?;
    let native_bytes = rgba_bytes * u64::from(header.depth / 8);
    let plane_bytes = u64::from(header.width)
        * u64::from(header.height)
        * u64::from(header.channels)
        * u64::from(header.depth / 8);
    ensure!(
        native_bytes <= limits.decoded_bytes && plane_bytes <= limits.decoded_bytes,
        "PSD native/combined channel buffers exceed the decoded limit"
    );
    // Includes simultaneous native/output/downscale buffers, ICC, bounded row
    // tables, CMS working space and the inflater. This is not an OS RSS ceiling.
    let memory = budget.reserve(native_bytes + rgba_bytes + 8 * 1024 * 1024)?;
    let mode_end = input.section(size)?;
    input.skip_to(mode_end)?;
    let mut meta = resources(&mut input, &header, cancelled)?;
    let (first_alpha, marked_alpha) = layer_flags(&mut input, header.depth, cancelled)?;
    if first_alpha {
        meta.alpha = Some(header.colors());
    } else if marked_alpha {
        meta.alpha = meta.alpha.or(Some(header.colors()));
    }
    ensure!(
        meta.alpha
            .is_none_or(|alpha| alpha >= header.colors() && alpha < header.channels),
        "PSD merged transparency channel is missing"
    );
    ensure!(size - input.pos >= 2, MISSING_COMPOSITE);
    let compression = input.short(size)?;
    ensure!(compression <= 3, "unsupported PSD compression");
    ensure!(
        size - input.pos <= limits.file_bytes.saturating_sub(input.read),
        "PSD composite exceeds the input size limit"
    );
    let mut pixels = Pixels::new(&header)?;
    let mut row = zeros(header.row_bytes())?;
    match compression {
        0 | 1 => {
            let table = if compression == 1 {
                let mut table = zeros(header.height as usize * usize::from(header.channels) * 2)?;
                input.read_into(&mut table, size)?;
                let encoded: u64 = table
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|bytes| u64::from(u16::from_be_bytes(*bytes)))
                    .sum();
                ensure!(encoded == size - input.pos, "invalid PSD RLE row lengths");
                table
            } else {
                ensure!(
                    plane_bytes == size - input.pos,
                    "invalid PSD raw pixel length"
                );
                Vec::new()
            };
            let mut packed = if compression == 1 {
                zeros(65535)?
            } else {
                Vec::new()
            };
            for channel in 0..header.channels {
                for y in 0..header.height {
                    ensure!(!cancelled(), "request superseded");
                    if compression == 0 {
                        input.read_into(&mut row, size)?;
                    } else {
                        let at = (usize::from(channel) * header.height as usize + y as usize) * 2;
                        let len = u16::from_be_bytes([table[at], table[at + 1]]) as usize;
                        input.read_into(&mut packed[..len], size)?;
                        unpack_rle(&packed[..len], &mut row)?;
                    }
                    pixels.row(&header, meta.alpha, channel, y, &row);
                }
            }
        }
        2 | 3 => {
            let mut zip = Zip::new(&mut input)?;
            for channel in 0..header.channels {
                for y in 0..header.height {
                    zip.row(&mut row, cancelled)?;
                    if compression == 3 {
                        prediction(&mut row, header.depth);
                    }
                    pixels.row(&header, meta.alpha, channel, y, &row);
                }
            }
            zip.finish(cancelled)?;
        }
        _ => unreachable!(),
    }
    ensure!(!cancelled(), "request superseded");
    // The loader also rechecks size/mtime after decode for cache invalidation.
    ensure!(
        input.file.get_ref().metadata()?.len() == size,
        "PSD file changed while reading"
    );
    let (rgba, warnings) = pixels.finish(&header, meta)?;
    Ok((rgba, memory, warnings))
}
