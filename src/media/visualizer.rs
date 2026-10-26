//! Audio visualizer and MP4 video generation.
//!
//! Generates animated waveform or still-image video with audio track,
//! matching Swift `AudioToVideoToolView` and `AudioOps.visualize`.
//! Supports 16:9 Landscape, 9:16 Portrait, and 1:1 Square orientations.

use std::path::PathBuf;
use image::{imageops, GenericImageView, ImageBuffer, Rgba, RgbaImage};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VisualizerStyle {
    Waveform,
    Bars,
    StillImage,
}

impl VisualizerStyle {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "bars" | "frequency bars" => VisualizerStyle::Bars,
            "stillimage" | "still_image" | "still image" | "still" => VisualizerStyle::StillImage,
            _ => VisualizerStyle::Waveform,
        }
    }

    #[allow(dead_code)]
    pub fn title(&self) -> &'static str {
        match self {
            VisualizerStyle::Waveform => "Waveform",
            VisualizerStyle::Bars => "Frequency bars",
            VisualizerStyle::StillImage => "Still image",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VisualizerShape {
    Landscape16x9,
    Portrait9x16,
    Square1x1,
}

impl VisualizerShape {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "portrait" | "9:16" | "portrait · 9:16" => VisualizerShape::Portrait9x16,
            "square" | "1:1" | "square · 1:1" => VisualizerShape::Square1x1,
            _ => VisualizerShape::Landscape16x9,
        }
    }

    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            VisualizerShape::Landscape16x9 => (1920, 1080),
            VisualizerShape::Portrait9x16 => (1080, 1920),
            VisualizerShape::Square1x1 => (1080, 1080),
        }
    }

    #[allow(dead_code)]
    pub fn title(&self) -> &'static str {
        match self {
            VisualizerShape::Landscape16x9 => "Landscape · 16:9",
            VisualizerShape::Portrait9x16 => "Portrait · 9:16",
            VisualizerShape::Square1x1 => "Square · 1:1",
        }
    }
}

#[derive(Clone, Debug)]
pub struct VisualizerSettings {
    pub style: VisualizerStyle,
    #[allow(dead_code)]
    pub shape: VisualizerShape,
    pub accent_color: [u8; 4],
    pub bg_color: [u8; 4],
    pub image_path: Option<PathBuf>,
}

impl Default for VisualizerSettings {
    fn default() -> Self {
        Self {
            style: VisualizerStyle::Waveform,
            shape: VisualizerShape::Landscape16x9,
            accent_color: [255, 92, 0, 255], // Tangerine orange #FF5C00
            bg_color: [0, 0, 0, 255],        // Black #000000
            image_path: None,
        }
    }
}

/// Parses hex color strings like "#FF5C00", "0xFF5C00", or "FF5C00".
pub fn parse_hex_color(hex: &str) -> [u8; 4] {
    let clean = hex.trim().trim_start_matches('#').trim_start_matches("0x");
    if clean.len() >= 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&clean[2..4], 16).unwrap_or(92);
        let b = u8::from_str_radix(&clean[4..6], 16).unwrap_or(0);
        let a = if clean.len() >= 8 {
            u8::from_str_radix(&clean[6..8], 16).unwrap_or(255)
        } else {
            255
        };
        [r, g, b, a]
    } else {
        [255, 92, 0, 255]
    }
}

/// Renders a waveform visualizer frame or preview matching `ref-audioToVideo.png`.
pub fn render_visualizer_frame(
    peaks: &[f32],
    width: u32,
    height: u32,
    settings: &VisualizerSettings,
) -> RgbaImage {
    let mut canvas = ImageBuffer::from_pixel(width, height, Rgba(settings.bg_color));
    if width == 0 || height == 0 {
        return canvas;
    }

    match settings.style {
        VisualizerStyle::StillImage => {
            if let Some(path) = &settings.image_path {
                if let Ok(img) = image::open(path) {
                    let (iw, ih) = img.dimensions();
                    if iw > 0 && ih > 0 {
                        let scale_w = (width as f32 * 0.90) / iw as f32;
                        let scale_h = (height as f32 * 0.90) / ih as f32;
                        let scale = scale_w.min(scale_h);
                        let tw = ((iw as f32 * scale).round() as u32).max(1);
                        let th = ((ih as f32 * scale).round() as u32).max(1);
                        let resized = img.resize_exact(tw, th, imageops::FilterType::Lanczos3);
                        let ox = ((width - tw) / 2) as i64;
                        let oy = ((height - th) / 2) as i64;
                        imageops::overlay(&mut canvas, &resized.to_rgba8(), ox, oy);
                        return canvas;
                    }
                }
            }
            // If no still image selected, draw an elegant placeholder box
            let box_w = (width as f32 * 0.5).min(200.0) as u32;
            let box_h = (height as f32 * 0.5).min(200.0) as u32;
            let ox = (width.saturating_sub(box_w)) / 2;
            let oy = (height.saturating_sub(box_h)) / 2;
            let border_color = Rgba([80, 80, 80, 255]);
            for x in ox..(ox + box_w) {
                canvas.put_pixel(x, oy, border_color);
                canvas.put_pixel(x, oy + box_h - 1, border_color);
            }
            for y in oy..(oy + box_h) {
                canvas.put_pixel(ox, y, border_color);
                canvas.put_pixel(ox + box_w - 1, y, border_color);
            }
        }
        VisualizerStyle::Waveform => {
            // Symmetrical vertical bars centered vertically
            let margin_x = (width as f32 * 0.05).max(12.0) as u32;
            let usable_w = width.saturating_sub(margin_x * 2);
            let bar_pitch = if width > 1000 { 12u32 } else { 5u32 };
            let bar_w = if width > 1000 { 6u32 } else { 3u32 };
            let num_bars = (usable_w / bar_pitch).max(1) as usize;

            let bucketed = resample_peaks(peaks, num_bars);
            let cy = height as f32 / 2.0;
            let max_half_h = (height as f32 * 0.38).max(4.0);
            let min_half_h = if width > 1000 { 6.0 } else { 2.5 };

            let accent = Rgba(settings.accent_color);

            for (i, &amp) in bucketed.iter().enumerate() {
                let bx = margin_x + (i as u32 * bar_pitch);
                let half_h = (amp * max_half_h).max(min_half_h);
                let y0 = ((cy - half_h).round() as u32).min(height - 1);
                let y1 = ((cy + half_h).round() as u32).min(height - 1);

                for x in bx..(bx + bar_w).min(width) {
                    for y in y0..=y1 {
                        canvas.put_pixel(x, y, accent);
                    }
                }
            }
        }
        VisualizerStyle::Bars => {
            // Spectrum / frequency style bars rising from baseline
            let margin_x = (width as f32 * 0.05).max(12.0) as u32;
            let usable_w = width.saturating_sub(margin_x * 2);
            let bar_pitch = if width > 1000 { 12u32 } else { 5u32 };
            let bar_w = if width > 1000 { 6u32 } else { 3u32 };
            let num_bars = (usable_w / bar_pitch).max(1) as usize;

            let bucketed = resample_peaks(peaks, num_bars);
            let baseline = (height as f32 * 0.78).min(height as f32 - 4.0);
            let max_h = (height as f32 * 0.60).max(4.0);
            let min_h = if width > 1000 { 6.0 } else { 3.0 };

            let accent = Rgba(settings.accent_color);

            for (i, &amp) in bucketed.iter().enumerate() {
                let bx = margin_x + (i as u32 * bar_pitch);
                let h = (amp * max_h).max(min_h);
                let y0 = ((baseline - h).round() as u32).min(height - 1);
                let y1 = (baseline.round() as u32).min(height - 1);

                for x in bx..(bx + bar_w).min(width) {
                    for y in y0..=y1 {
                        canvas.put_pixel(x, y, accent);
                    }
                }
            }
        }
    }

    canvas
}

/// Resamples an arbitrary slice of peaks into exactly `target_count` normalized buckets.
fn resample_peaks(peaks: &[f32], target_count: usize) -> Vec<f32> {
    if target_count == 0 {
        return Vec::new();
    }
    if peaks.is_empty() {
        // Fallback test waveform pattern: burst, silence, burst, silence (like reference mockup)
        return (0..target_count)
            .map(|i| {
                let frac = i as f32 / target_count as f32;
                if (0.15..=0.40).contains(&frac) || (0.55..=0.80).contains(&frac) {
                    let sub = if frac < 0.5 { (frac - 0.15) / 0.25 } else { (frac - 0.55) / 0.25 };
                    0.20 + 0.80 * (sub * std::f32::consts::PI).sin()
                } else {
                    0.0
                }
            })
            .collect();
    }

    let mut out = Vec::with_capacity(target_count);
    let n = peaks.len() as f32;
    for i in 0..target_count {
        let start = ((i as f32 / target_count as f32) * n) as usize;
        let end = ((((i + 1) as f32 / target_count as f32) * n).ceil() as usize).min(peaks.len());
        if start >= end {
            out.push(peaks.get(start).copied().unwrap_or(0.0).clamp(0.0, 1.0));
        } else {
            let mut max_amp: f32 = 0.0;
            for &p in &peaks[start..end] {
                if p > max_amp {
                    max_amp = p;
                }
            }
            out.push(max_amp.clamp(0.0, 1.0));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_color_parses_various_formats() {
        assert_eq!(parse_hex_color("#FF5C00"), [255, 92, 0, 255]);
        assert_eq!(parse_hex_color("0xFF5C00"), [255, 92, 0, 255]);
        assert_eq!(parse_hex_color("000000"), [0, 0, 0, 255]);
        assert_eq!(parse_hex_color("#1A1A1AFF"), [26, 26, 26, 255]);
    }

    #[test]
    fn visualizer_style_and_shape_parsing() {
        assert_eq!(VisualizerStyle::parse("waveform"), VisualizerStyle::Waveform);
        assert_eq!(VisualizerStyle::parse("bars"), VisualizerStyle::Bars);
        assert_eq!(VisualizerStyle::parse("still image"), VisualizerStyle::StillImage);

        assert_eq!(VisualizerShape::parse("16:9"), VisualizerShape::Landscape16x9);
        assert_eq!(VisualizerShape::parse("9:16"), VisualizerShape::Portrait9x16);
        assert_eq!(VisualizerShape::parse("1:1"), VisualizerShape::Square1x1);

        assert_eq!(VisualizerShape::Landscape16x9.dimensions(), (1920, 1080));
        assert_eq!(VisualizerShape::Portrait9x16.dimensions(), (1080, 1920));
        assert_eq!(VisualizerShape::Square1x1.dimensions(), (1080, 1080));
    }

    #[test]
    fn render_visualizer_frame_produces_correct_dimensions_and_pixels() {
        let settings = VisualizerSettings::default();
        let frame = render_visualizer_frame(&[], 400, 225, &settings);
        assert_eq!(frame.width(), 400);
        assert_eq!(frame.height(), 225);

        // Center should have orange pixels from waveform
        let mut found_accent = false;
        let accent = Rgba(settings.accent_color);
        for p in frame.pixels() {
            if *p == accent {
                found_accent = true;
                break;
            }
        }
        assert!(found_accent, "visualizer frame must contain accent bars");
    }

    #[test]
    fn resample_peaks_falls_back_to_burst_pattern_when_empty() {
        let resampled = resample_peaks(&[], 60);
        assert_eq!(resampled.len(), 60);
        // Middle of first burst (around index 16..20) should have high amplitude
        assert!(resampled[17] > 0.5);
        // Silence region (around index 5) should be near 0
        assert_eq!(resampled[5], 0.0);
    }
}
