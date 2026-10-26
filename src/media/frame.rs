//! Image framing and background compositing.
//!
//! Places a photograph on a new canvas with configurable padding,
//! rounded corners, drop shadow, and background styles (gradients,
//! solid colors, blurred source photo, or custom background image).
//! Matches the Swift `FrameImageToolView` / `ImageOps.framed` specification.

use std::path::Path;
use image::{imageops, DynamicImage, GenericImageView, ImageBuffer, Rgba, RgbaImage};

#[derive(Clone, Debug, PartialEq)]
pub enum FrameBackground {
    Color([u8; 4]),
    Gradient {
        color_a: [u8; 4],
        color_b: [u8; 4],
        angle_deg: f32,
    },
    Blur {
        radius: f32,
    },
    Image(RgbaImage),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FrameRatio {
    Auto,
    Square1x1,
    Wide16x9,
    Tall4x5,
}

#[derive(Clone, Debug)]
pub struct FrameSettings {
    pub aspect: FrameRatio,
    pub padding_fraction: f32,
    pub corner_radius: f32,
    pub shadow_radius: f32,
    pub background: FrameBackground,
}

impl Default for FrameSettings {
    fn default() -> Self {
        Self {
            aspect: FrameRatio::Auto,
            padding_fraction: 0.08,
            corner_radius: 24.0,
            shadow_radius: 24.0,
            background: FrameBackground::Blur { radius: 30.0 },
        }
    }
}

#[derive(Debug)]
pub enum FrameError {
    ReadFailed(String),
    SaveFailed(String),
    EncodeFailed(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadFailed(msg) => write!(f, "Could not read image: {}", msg),
            Self::SaveFailed(msg) => write!(f, "Could not save image: {}", msg),
            Self::EncodeFailed(msg) => write!(f, "Could not encode framed image: {}", msg),
        }
    }
}

impl std::error::Error for FrameError {}

/// Calculates the layout geometry for framing.
/// Returns: (canvas_width, canvas_height, drawn_width, drawn_height, origin_x, origin_y).
pub fn calculate_layout(
    img_width: u32,
    img_height: u32,
    aspect: FrameRatio,
    padding_fraction: f32,
) -> (u32, u32, u32, u32, i32, i32) {
    let img_w = img_width.max(1) as f32;
    let img_h = img_height.max(1) as f32;
    let img_ratio = img_w / img_h;

    let target_aspect = match aspect {
        FrameRatio::Auto => None,
        FrameRatio::Square1x1 => Some(1.0f32),
        FrameRatio::Wide16x9 => Some(16.0 / 9.0),
        FrameRatio::Tall4x5 => Some(4.0 / 5.0),
    };

    let mut canvas_w = img_w;
    let mut canvas_h = img_h;

    if let Some(ratio) = target_aspect {
        if img_ratio > ratio {
            canvas_w = img_w;
            canvas_h = img_w / ratio;
        } else {
            canvas_w = img_h * ratio;
            canvas_h = img_h;
        }
    }

    let pad = padding_fraction.max(0.0) * canvas_w.min(canvas_h);
    if target_aspect.is_some() {
        let factor = ((canvas_w + pad * 2.0) / canvas_w).max((canvas_h + pad * 2.0) / canvas_h);
        canvas_w *= factor;
        canvas_h *= factor;
    } else {
        canvas_w += pad * 2.0;
        canvas_h += pad * 2.0;
    }

    let scale = ((canvas_w - pad * 2.0) / img_w).min((canvas_h - pad * 2.0) / img_h);
    let drawn_w = (img_w * scale).round().max(1.0) as u32;
    let drawn_h = (img_h * scale).round().max(1.0) as u32;

    let cw = canvas_w.round().max(1.0) as u32;
    let ch = canvas_h.round().max(1.0) as u32;

    let origin_x = ((cw as f32 - drawn_w as f32) / 2.0).round() as i32;
    let origin_y = ((ch as f32 - drawn_h as f32) / 2.0).round() as i32;

    (cw, ch, drawn_w, drawn_h, origin_x, origin_y)
}

/// Renders a framed image with the given settings.
pub fn render_frame(
    source: &DynamicImage,
    settings: &FrameSettings,
    high_quality: bool,
) -> Result<RgbaImage, FrameError> {
    let (sw, sh) = source.dimensions();
    if sw == 0 || sh == 0 {
        return Err(FrameError::EncodeFailed("Empty source image".into()));
    }

    let (cw, ch, dw, dh, ox, oy) = calculate_layout(
        sw,
        sh,
        settings.aspect,
        settings.padding_fraction,
    );

    let mut canvas = RgbaImage::new(cw, ch);

    // 1. Draw background
    match &settings.background {
        FrameBackground::Color(c) => {
            let px = Rgba(*c);
            for p in canvas.pixels_mut() {
                *p = px;
            }
        }
        FrameBackground::Gradient {
            color_a,
            color_b,
            angle_deg,
        } => {
            draw_gradient(&mut canvas, *color_a, *color_b, *angle_deg);
        }
        FrameBackground::Blur { radius } => {
            draw_blurred_bg(&mut canvas, source, *radius);
        }
        FrameBackground::Image(bg_img) => {
            draw_image_bg(&mut canvas, bg_img);
        }
    }

    // 2. Draw drop shadow
    if settings.shadow_radius > 0.5 {
        draw_drop_shadow(
            &mut canvas,
            ox,
            oy,
            dw,
            dh,
            settings.corner_radius,
            settings.shadow_radius,
        );
    }

    // 3. Draw rounded source image
    let filter = if high_quality {
        imageops::FilterType::Lanczos3
    } else {
        imageops::FilterType::Triangle
    };
    let resized = source.resize_exact(dw, dh, filter).to_rgba8();
    draw_rounded_image(&mut canvas, &resized, ox, oy, settings.corner_radius);

    Ok(canvas)
}

/// Frames an image file and saves the result to `output`.
pub fn frame_file(
    input: &Path,
    output: &Path,
    settings: &FrameSettings,
) -> Result<(), FrameError> {
    let source = image::open(input).map_err(|e| FrameError::ReadFailed(e.to_string()))?;
    let framed = render_frame(&source, settings, true)?;
    framed
        .save(output)
        .map_err(|e| FrameError::SaveFailed(e.to_string()))?;
    Ok(())
}

fn draw_gradient(
    canvas: &mut RgbaImage,
    color_a: [u8; 4],
    color_b: [u8; 4],
    angle_deg: f32,
) {
    let w = canvas.width() as f32;
    let h = canvas.height() as f32;
    if w <= 0.0 || h <= 0.0 {
        return;
    }

    let cx = w / 2.0;
    let cy = h / 2.0;
    let radians = angle_deg.to_radians();
    let dx = radians.cos();
    let dy = radians.sin();
    let len = ((w * dx).abs() + (h * dy).abs()) / 2.0;
    let denom = if len > 0.001 { len * 2.0 } else { 1.0 };

    for y in 0..canvas.height() {
        for x in 0..canvas.width() {
            let px = x as f32 + 0.5 - cx;
            let py = y as f32 + 0.5 - cy;
            let proj = px * dx + py * dy;
            let t = ((proj + len) / denom).clamp(0.0, 1.0);

            let r = lerp_u8(color_a[0], color_b[0], t);
            let g = lerp_u8(color_a[1], color_b[1], t);
            let b = lerp_u8(color_a[2], color_b[2], t);
            let a = lerp_u8(color_a[3], color_b[3], t);
            canvas.put_pixel(x, y, Rgba([r, g, b, a]));
        }
    }
}

#[inline]
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let v = a as f32 * (1.0 - t) + b as f32 * t;
    v.round().clamp(0.0, 255.0) as u8
}

fn draw_blurred_bg(canvas: &mut RgbaImage, source: &DynamicImage, blur_radius: f32) {
    let cw = canvas.width();
    let ch = canvas.height();
    if cw == 0 || ch == 0 {
        return;
    }

    // Downsample source to a reasonable size for fast, smooth blur
    let target_w = 320.min(cw);
    let target_h = ((target_w as f32 * ch as f32) / cw as f32).round().max(1.0) as u32;

    let small = source.resize_to_fill(target_w, target_h, imageops::FilterType::Triangle).to_rgba8();
    let sigma = (blur_radius * (target_w as f32 / cw as f32)).max(1.0).min(25.0);
    let blurred_small = imageops::blur(&small, sigma);

    // Upscale blurred result to fill canvas
    let upscaled = image::DynamicImage::ImageRgba8(blurred_small).resize_exact(
        cw,
        ch,
        imageops::FilterType::Triangle,
    ).to_rgba8();

    *canvas = upscaled;
}

fn draw_image_bg(canvas: &mut RgbaImage, bg: &RgbaImage) {
    let cw = canvas.width();
    let ch = canvas.height();
    if cw == 0 || ch == 0 || bg.width() == 0 || bg.height() == 0 {
        return;
    }

    let dyn_bg = image::DynamicImage::ImageRgba8(bg.clone());
    let resized = dyn_bg.resize_to_fill(cw, ch, imageops::FilterType::Triangle).to_rgba8();
    *canvas = resized;
}

fn draw_drop_shadow(
    canvas: &mut RgbaImage,
    ox: i32,
    oy: i32,
    dw: u32,
    dh: u32,
    corner_radius: f32,
    shadow_radius: f32,
) {
    let cw = canvas.width();
    let ch = canvas.height();
    if cw == 0 || ch == 0 || dw == 0 || dh == 0 {
        return;
    }

    // Drop shadow offset downwards by shadow_radius / 3.0
    let dy = (shadow_radius / 3.0).round() as i32;
    let soy = oy + dy;

    // Downscale factor for blur acceleration
    let scale = if cw > 400 || ch > 400 { 0.5f32 } else { 1.0f32 };
    let mw = ((cw as f32) * scale).round().max(1.0) as u32;
    let mh = ((ch as f32) * scale).round().max(1.0) as u32;

    let mut mask = ImageBuffer::from_pixel(mw, mh, image::Luma([0u8]));
    let mox = ((ox as f32) * scale).round() as i32;
    let msoy = ((soy as f32) * scale).round() as i32;
    let mdw = ((dw as f32) * scale).round().max(1.0) as u32;
    let mdh = ((dh as f32) * scale).round().max(1.0) as u32;
    let mrad = (corner_radius * scale).max(0.0);

    // Fill rounded rect in mask
    let shadow_intensity = 110u8; // ~43% black
    for y in 0..mdh {
        let py = msoy + y as i32;
        if py < 0 || py >= mh as i32 {
            continue;
        }
        for x in 0..mdw {
            let px = mox + x as i32;
            if px < 0 || px >= mw as i32 {
                continue;
            }
            let alpha = corner_alpha(x as f32, y as f32, mdw as f32, mdh as f32, mrad);
            if alpha > 0.0 {
                let v = (shadow_intensity as f32 * alpha).round() as u8;
                mask.put_pixel(px as u32, py as u32, image::Luma([v]));
            }
        }
    }

    let sigma = (shadow_radius * scale * 0.5).max(1.0);
    let blurred_mask = imageops::blur(&mask, sigma);

    // Upscale mask if downscaled
    let full_mask = if scale < 0.99 {
        image::DynamicImage::ImageLuma8(blurred_mask)
            .resize_exact(cw, ch, imageops::FilterType::Triangle)
            .to_luma8()
    } else {
        blurred_mask
    };

    // Composite shadow onto canvas (darken background)
    for y in 0..ch {
        for x in 0..cw {
            let shadow_val = full_mask.get_pixel(x, y).0[0];
            if shadow_val > 0 {
                let p = canvas.get_pixel_mut(x, y);
                let inv = 255 - shadow_val;
                p.0[0] = ((p.0[0] as u32 * inv as u32) / 255) as u8;
                p.0[1] = ((p.0[1] as u32 * inv as u32) / 255) as u8;
                p.0[2] = ((p.0[2] as u32 * inv as u32) / 255) as u8;
            }
        }
    }
}

fn draw_rounded_image(
    canvas: &mut RgbaImage,
    resized_src: &RgbaImage,
    ox: i32,
    oy: i32,
    corner_radius: f32,
) {
    let cw = canvas.width() as i32;
    let ch = canvas.height() as i32;
    let dw = resized_src.width();
    let dh = resized_src.height();
    let r = corner_radius.min(dw as f32 / 2.0).min(dh as f32 / 2.0).max(0.0);

    for y in 0..dh {
        let dest_y = oy + y as i32;
        if dest_y < 0 || dest_y >= ch {
            continue;
        }
        for x in 0..dw {
            let dest_x = ox + x as i32;
            if dest_x < 0 || dest_x >= cw {
                continue;
            }

            let spx = resized_src.get_pixel(x, y);
            let calpha = corner_alpha(x as f32, y as f32, dw as f32, dh as f32, r);
            if calpha <= 0.0 {
                continue;
            }

            let src_a = (spx.0[3] as f32 / 255.0) * calpha;
            if src_a <= 0.001 {
                continue;
            }

            let dpx = canvas.get_pixel_mut(dest_x as u32, dest_y as u32);
            if src_a >= 0.999 {
                dpx.0[0] = spx.0[0];
                dpx.0[1] = spx.0[1];
                dpx.0[2] = spx.0[2];
                dpx.0[3] = 255;
            } else {
                let inv = 1.0 - src_a;
                dpx.0[0] = (spx.0[0] as f32 * src_a + dpx.0[0] as f32 * inv).round() as u8;
                dpx.0[1] = (spx.0[1] as f32 * src_a + dpx.0[1] as f32 * inv).round() as u8;
                dpx.0[2] = (spx.0[2] as f32 * src_a + dpx.0[2] as f32 * inv).round() as u8;
                dpx.0[3] = 255;
            }
        }
    }
}

#[inline]
fn corner_alpha(x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    if r <= 0.5 {
        return 1.0;
    }

    let left = x < r;
    let right = x >= w - r;
    let top = y < r;
    let bottom = y >= h - r;

    if !(left || right) || !(top || bottom) {
        return 1.0;
    }

    let cx = if left { r } else { w - r };
    let cy = if top { r } else { h - r };

    let dx = x - cx;
    let dy = y - cy;
    let dist = (dx * dx + dy * dy).sqrt();

    if dist <= r - 0.5 {
        1.0
    } else if dist >= r + 0.5 {
        0.0
    } else {
        1.0 - (dist - (r - 0.5))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_auto_adds_padding_proportionally() {
        let (cw, ch, dw, dh, ox, oy) = calculate_layout(200, 100, FrameRatio::Auto, 0.1);
        assert!(cw > 200);
        assert!(ch > 100);
        assert_eq!(dw, 200);
        assert_eq!(dh, 100);
        assert_eq!(ox, ((cw - dw) / 2) as i32);
        assert_eq!(oy, ((ch - dh) / 2) as i32);
    }

    #[test]
    fn layout_square_forces_1_to_1() {
        let (cw, ch, dw, dh, ox, oy) = calculate_layout(200, 100, FrameRatio::Square1x1, 0.0);
        assert_eq!(cw, ch);
        assert_eq!(dw, 200);
        assert_eq!(dh, 100);
        assert_eq!(ox, 0);
        assert_eq!(oy, ((ch - dh) / 2) as i32);
    }

    #[test]
    fn frame_renders_solid_color() {
        let src = DynamicImage::new_rgb8(100, 100);
        let settings = FrameSettings {
            aspect: FrameRatio::Auto,
            padding_fraction: 0.1,
            corner_radius: 10.0,
            shadow_radius: 0.0,
            background: FrameBackground::Color([255, 0, 0, 255]),
        };
        let framed = render_frame(&src, &settings, false).expect("frame renders");
        assert!(framed.width() > 100);
        assert!(framed.height() > 100);
        // Top-left pixel should be the red background
        let p = framed.get_pixel(0, 0);
        assert_eq!(p.0[0], 255);
        assert_eq!(p.0[1], 0);
        assert_eq!(p.0[2], 0);
    }

    #[test]
    fn frame_renders_gradient() {
        let src = DynamicImage::new_rgb8(60, 60);
        let settings = FrameSettings {
            aspect: FrameRatio::Square1x1,
            padding_fraction: 0.2,
            corner_radius: 12.0,
            shadow_radius: 10.0,
            background: FrameBackground::Gradient {
                color_a: [255, 100, 50, 255],
                color_b: [50, 100, 255, 255],
                angle_deg: 90.0,
            },
        };
        let framed = render_frame(&src, &settings, false).expect("gradient renders");
        assert_eq!(framed.width(), framed.height());
    }
}
