use anyhow::Result;
use image::RgbaImage;
use moxcms::{ColorProfile, DataColorSpace, Layout};
use std::sync::OnceLock;

pub const MAX_ICC_BYTES: usize = 1024 * 1024;

/// RGB and grayscale ICC profiles are converted to sRGB. Unsupported/invalid
/// profiles consistently fall back to sRGB, with a diagnostic from the caller.
pub fn convert_icc(pixels: &mut [u8], profile: &[u8]) -> Result<()> {
    anyhow::ensure!(profile.len() <= MAX_ICC_BYTES, "ICC profile exceeds 1 MiB");
    let source = ColorProfile::new_from_slice(profile)?;
    convert_profile(pixels, &source)
}

pub fn convert_profile(pixels: &mut [u8], source: &ColorProfile) -> Result<()> {
    let gray = source.color_space == DataColorSpace::Gray;
    let layout = if gray { Layout::Gray } else { Layout::Rgba };
    let transform = source.create_transform_8bit(
        layout,
        &ColorProfile::new_srgb(),
        Layout::Rgba,
        Default::default(),
    )?;
    // Bound scratch space instead of copying an entire image for color conversion.
    let mut input = vec![0; 16 * 1024];
    for chunk in pixels.chunks_mut(16 * 1024) {
        if gray {
            let count = chunk.len() / 4;
            for (i, p) in chunk.as_chunks::<4>().0.iter().enumerate() {
                input[i] = p[0];
            }
            let alpha: Vec<_> = chunk.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
            transform.transform(&input[..count], chunk)?;
            for (p, alpha) in chunk.as_chunks_mut::<4>().0.iter_mut().zip(alpha) {
                p[3] = alpha;
            }
        } else {
            input[..chunk.len()].copy_from_slice(chunk);
            transform.transform(&input[..chunk.len()], chunk)?;
        }
    }
    Ok(())
}

pub fn srgb_to_linear(v: u8) -> f32 {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        std::array::from_fn(|i| {
            let s = i as f32 / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        })
    })[usize::from(v)]
}

pub fn linear_to_srgb(v: f32) -> u8 {
    static LUT: OnceLock<Vec<u8>> = OnceLock::new();
    let table = LUT.get_or_init(|| {
        (0..=4096)
            .map(|i| {
                let l = i as f32 / 4096.0;
                let s = if l <= 0.0031308 {
                    l * 12.92
                } else {
                    1.055 * l.powf(1.0 / 2.4) - 0.055
                };
                (s * 255.0).round() as u8
            })
            .collect()
    });
    table[(v.clamp(0.0, 1.0) * 4096.0).round() as usize]
}

/// Fixed-exposure Reinhard mapping per linear RGB channel, followed by the sRGB
/// transfer function. Compress highlights before quantizing to the SDR surface.
pub(crate) fn tone_map_hdr(source: &image::Rgb32FImage) -> RgbaImage {
    RgbaImage::from_fn(source.width(), source.height(), |x, y| {
        let rgb = source.get_pixel(x, y).0.map(|value| {
            let mapped = if value.is_nan() || value <= 0.0 {
                0.0
            } else if value.is_infinite() {
                1.0
            } else {
                value / (1.0 + value)
            };
            linear_to_srgb(mapped)
        });
        image::Rgba([rgb[0], rgb[1], rgb[2], 255])
    })
}

/// Store sRGB-encoded, linear-premultiplied RGB. GPU sRGB sampling therefore
/// filters premultiplied linear light and produces the correct result on black.
pub fn premultiply_linear(pixels: &mut [u8]) {
    for p in pixels.as_chunks_mut::<4>().0 {
        if p[3] == 255 {
            continue;
        }
        let alpha = f32::from(p[3]) / 255.0;
        for v in &mut p[..3] {
            *v = linear_to_srgb(srgb_to_linear(*v) * alpha);
        }
    }
}

pub fn unpremultiply_srgb(pixels: &mut [u8]) {
    for p in pixels.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        if a > 0 && a < 255 {
            for v in &mut p[..3] {
                *v = ((u32::from(*v) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

pub fn downscale(source: RgbaImage, target: [u32; 2]) -> RgbaImage {
    if [source.width(), source.height()] == target {
        return source;
    }
    let sx = source.width() as f64 / f64::from(target[0]);
    let sy = source.height() as f64 / f64::from(target[1]);
    RgbaImage::from_fn(target[0], target[1], |x, y| {
        let fx = ((f64::from(x) + 0.5) * sx - 0.5).max(0.0);
        let fy = ((f64::from(y) + 0.5) * sy - 0.5).max(0.0);
        let x0 = (fx as u32).min(source.width() - 1);
        let y0 = (fy as u32).min(source.height() - 1);
        let x1 = (x0 + 1).min(source.width() - 1);
        let y1 = (y0 + 1).min(source.height() - 1);
        let dx = (fx - f64::from(x0)) as f32;
        let dy = (fy - f64::from(y0)) as f32;
        let points = [
            source.get_pixel(x0, y0),
            source.get_pixel(x1, y0),
            source.get_pixel(x0, y1),
            source.get_pixel(x1, y1),
        ];
        let weights = [
            (1.0 - dx) * (1.0 - dy),
            dx * (1.0 - dy),
            (1.0 - dx) * dy,
            dx * dy,
        ];
        image::Rgba(std::array::from_fn(|c| {
            let value: f32 = points
                .iter()
                .zip(weights)
                .map(|(p, w)| {
                    (if c == 3 {
                        f32::from(p[c]) / 255.0
                    } else {
                        srgb_to_linear(p[c])
                    }) * w
                })
                .sum();
            if c == 3 {
                (value * 255.0).round() as u8
            } else {
                linear_to_srgb(value)
            }
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transparency_blends_in_linear_light() {
        let mut p = [255, 255, 255, 128, 255, 0, 0, 0];
        premultiply_linear(&mut p);
        assert!((i16::from(p[0]) - 188).abs() <= 1);
        assert_eq!(&p[4..7], &[0, 0, 0]);
    }
    #[test]
    fn icc_conversion_changes_wide_gamut_and_preserves_alpha() {
        let mut p = [170, 100, 80, 99];
        convert_profile(&mut p, &ColorProfile::new_display_p3()).unwrap();
        assert_ne!(&p[..3], &[170, 100, 80]);
        assert_eq!(p[3], 99);
        assert!(convert_icc(&mut p, b"invalid ICC").is_err());
    }
    #[test]
    fn linear_downscale_has_no_dark_transparency_fringe() {
        let mut img = RgbaImage::from_raw(2, 1, vec![255, 255, 255, 255, 255, 0, 0, 0]).unwrap();
        premultiply_linear(img.as_mut());
        let reduced = downscale(img, [1, 1]);
        let p = reduced.get_pixel(0, 0);
        assert_eq!(p[0], p[1]);
        assert_eq!(p[1], p[2]);
        assert!((i16::from(p[0]) - 188).abs() <= 1);
    }
}
