//! Photo adjustment and filter engine (P3-13 / `tools.edit_image`).
//!
//! Provides real-time interactive adjustments for photographs:
//! - Exposure, Brightness, and Contrast
//! - Saturation, Temperature (warmth), and Tint
//! - Sharpness and Vignette
//! Matches the Swift `EditImageToolView` / `PhotoAdjustments` specification.

use std::path::Path;
use image::{imageops, DynamicImage, Rgba, RgbaImage};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhotoAdjustments {
    /// Exposure adjustment: -200 to +200 (-2.0 EV to +2.0 EV). Default: 0.
    pub exposure_x100: i32,
    /// Brightness adjustment: -50 to +50 (-0.5 to +0.5). Default: 0.
    pub brightness_x100: i32,
    /// Contrast adjustment: 50 to 150 (0.5 to 1.5, 100 = 1.0). Default: 100.
    pub contrast_x100: i32,
    /// Saturation adjustment: 0 to 200 (0.0 to 2.0, 100 = 1.0). Default: 100.
    pub saturation_x100: i32,
    /// Color temperature (warmth): -100 to +100 (-cool .. +warm). Default: 0.
    pub temperature: i32,
    /// Tint (green to magenta): -100 to +100. Default: 0.
    pub tint: i32,
    /// Sharpness amount: 0 to 200 (0.0 to 2.0). Default: 0.
    pub sharpness_x100: i32,
    /// Vignette amount: 0 to 200 (0.0 to 2.0). Default: 0.
    pub vignette_x100: i32,
}

impl PhotoAdjustments {
    pub fn is_identity(&self) -> bool {
        self.exposure_x100 == 0
            && self.brightness_x100 == 0
            && self.contrast_x100 == 100
            && self.saturation_x100 == 100
            && self.temperature == 0
            && self.tint == 0
            && self.sharpness_x100 == 0
            && self.vignette_x100 == 0
    }
}

#[derive(Debug)]
pub enum AdjustError {
    ReadFailed(String),
    SaveFailed(String),
}

impl std::fmt::Display for AdjustError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadFailed(msg) => write!(f, "Failed to read image: {msg}"),
            Self::SaveFailed(msg) => write!(f, "Failed to save adjusted image: {msg}"),
        }
    }
}

impl std::error::Error for AdjustError {}

/// Applies adjustments to a DynamicImage, returning an adjusted RgbaImage.
pub fn apply_adjustments(
    source: &DynamicImage,
    adj: &PhotoAdjustments,
) -> RgbaImage {
    let mut canvas = source.to_rgba8();
    let (w, h) = canvas.dimensions();
    if w == 0 || h == 0 || adj.is_identity() {
        return canvas;
    }

    let exp_factor = 2.0f32.powf(adj.exposure_x100 as f32 / 100.0);
    let brightness = adj.brightness_x100 as f32 / 100.0;
    let contrast = adj.contrast_x100 as f32 / 100.0;
    let saturation = adj.saturation_x100 as f32 / 100.0;

    let temp = adj.temperature as f32 / 100.0;
    let tint = adj.tint as f32 / 100.0;
    let vignette = adj.vignette_x100 as f32 / 100.0;

    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    let max_dist = (cx * cx + cy * cy).sqrt();

    for y in 0..h {
        let dy = y as f32 - cy;
        for x in 0..w {
            let dx = x as f32 - cx;
            let p = canvas.get_pixel_mut(x, y);

            let mut r = p.0[0] as f32 / 255.0;
            let mut g = p.0[1] as f32 / 255.0;
            let mut b = p.0[2] as f32 / 255.0;

            // 1. Exposure
            if adj.exposure_x100 != 0 {
                r *= exp_factor;
                g *= exp_factor;
                b *= exp_factor;
            }

            // 2. Brightness
            if adj.brightness_x100 != 0 {
                r += brightness;
                g += brightness;
                b += brightness;
            }

            // 3. Contrast
            if adj.contrast_x100 != 100 {
                r = (r - 0.5) * contrast + 0.5;
                g = (g - 0.5) * contrast + 0.5;
                b = (b - 0.5) * contrast + 0.5;
            }

            // 4. Color Temperature (warm/cool shift)
            if adj.temperature != 0 {
                r += temp * 0.15;
                b -= temp * 0.15;
            }

            // 5. Tint (magenta/green shift)
            if adj.tint != 0 {
                r += tint * 0.1;
                g -= tint * 0.15;
                b += tint * 0.1;
            }

            // 6. Saturation
            if adj.saturation_x100 != 100 {
                let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                r = lum + (r - lum) * saturation;
                g = lum + (g - lum) * saturation;
                b = lum + (b - lum) * saturation;
            }

            // 7. Vignette
            if adj.vignette_x100 > 0 {
                let dist = (dx * dx + dy * dy).sqrt() / max_dist;
                let vig_falloff = 1.0 - (dist * dist * vignette * 0.7).clamp(0.0, 1.0);
                r *= vig_falloff;
                g *= vig_falloff;
                b *= vig_falloff;
            }

            p.0[0] = (r * 255.0).round().clamp(0.0, 255.0) as u8;
            p.0[1] = (g * 255.0).round().clamp(0.0, 255.0) as u8;
            p.0[2] = (b * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    // 8. Sharpness unsharp mask
    if adj.sharpness_x100 > 0 {
        let blurred = imageops::blur(&canvas, 1.5);
        let amount = adj.sharpness_x100 as f32 / 100.0;
        for y in 0..h {
            for x in 0..w {
                let orig = *canvas.get_pixel(x, y);
                let blur = *blurred.get_pixel(x, y);
                let p = canvas.get_pixel_mut(x, y);

                for c in 0..3 {
                    let diff = orig.0[c] as f32 - blur.0[c] as f32;
                    let val = orig.0[c] as f32 + diff * amount;
                    p.0[c] = val.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    canvas
}

/// Applies photo adjustments to an input file and saves to output.
pub fn adjust_file(
    input: &Path,
    output: &Path,
    adj: &PhotoAdjustments,
) -> Result<(), AdjustError> {
    let img = image::open(input).map_err(|e| AdjustError::ReadFailed(e.to_string()))?;
    let adjusted = apply_adjustments(&img, adj);

    if let Some(parent) = output.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext == "jpg" || ext == "jpeg" {
        let rgb = DynamicImage::ImageRgba8(adjusted).into_rgb8();
        rgb.save(output)
            .map_err(|e| AdjustError::SaveFailed(e.to_string()))
    } else {
        adjusted
            .save(output)
            .map_err(|e| AdjustError::SaveFailed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adjustments_identity_noop() {
        let base = DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, Rgba([128, 128, 128, 255])));
        let adj = PhotoAdjustments::default();
        let res = apply_adjustments(&base, &adj);
        assert_eq!(res.get_pixel(50, 50), &Rgba([128, 128, 128, 255]));
    }

    #[test]
    fn test_adjustments_brightness_contrast() {
        let base = DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, Rgba([100, 100, 100, 255])));
        let adj = PhotoAdjustments {
            brightness_x100: 20, // +0.20
            contrast_x100: 120,   // 1.2x
            ..Default::default()
        };
        let res = apply_adjustments(&base, &adj);
        let p = res.get_pixel(50, 50);
        assert!(p.0[0] > 100);
    }
}
