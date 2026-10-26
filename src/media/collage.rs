//! Collage compositing and layout engine (P3-18 / `tools.collage`).
//!
//! Arranges 2 or more photographs into unified grid, column, row, or
//! featured hero compositions with customizable padding, corner radius,
//! aspect ratio, and background colors. Matches the Swift
//! `CollageToolView` / `ImageOps.collage` specification.

use std::path::{Path, PathBuf};
use image::{imageops, DynamicImage, GenericImageView, Rgba, RgbaImage};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CollageLayout {
    Grid,
    Columns,
    Rows,
    Featured,
}

impl Default for CollageLayout {
    fn default() -> Self {
        Self::Grid
    }
}

impl CollageLayout {
    pub fn from_str_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "columns" | "column" => Self::Columns,
            "rows" | "row" => Self::Rows,
            "featured" | "hero" => Self::Featured,
            _ => Self::Grid,
        }
    }

    #[allow(dead_code)]
    pub fn title(&self) -> &'static str {
        match self {
            Self::Grid => "Grid",
            Self::Columns => "Columns",
            Self::Rows => "Rows",
            Self::Featured => "Featured",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Debug)]
pub struct CollageSettings {
    pub layout: CollageLayout,
    pub width: u32,
    pub height: u32,
    pub spacing: f32,
    pub corner_radius: f32,
    pub background: [u8; 4],
}

impl Default for CollageSettings {
    fn default() -> Self {
        Self {
            layout: CollageLayout::Grid,
            width: 2400,
            height: 2400,
            spacing: 24.0,
            corner_radius: 16.0,
            background: [255, 255, 255, 255], // White
        }
    }
}

#[derive(Debug)]
pub enum CollageError {
    EmptyInput,
    ReadFailed(String),
    SaveFailed(String),
}

impl std::fmt::Display for CollageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyInput => write!(f, "At least two images are required for a collage"),
            Self::ReadFailed(msg) => write!(f, "Failed to read image: {msg}"),
            Self::SaveFailed(msg) => write!(f, "Failed to save collage: {msg}"),
        }
    }
}

impl std::error::Error for CollageError {}

/// Calculates tile bounding boxes for a collage layout.
pub fn compute_frames(
    count: usize,
    canvas_w: f32,
    canvas_h: f32,
    spacing: f32,
    layout: CollageLayout,
) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let s = spacing.max(0.0);
    let mut frames = Vec::with_capacity(count);

    match layout {
        CollageLayout::Columns => {
            let n = count as f32;
            let cell_w = ((canvas_w - s * (n + 1.0)) / n).max(1.0);
            let cell_h = (canvas_h - 2.0 * s).max(1.0);
            for i in 0..count {
                frames.push(Rect {
                    x: s + i as f32 * (cell_w + s),
                    y: s,
                    w: cell_w,
                    h: cell_h,
                });
            }
        }
        CollageLayout::Rows => {
            let n = count as f32;
            let cell_w = (canvas_w - 2.0 * s).max(1.0);
            let cell_h = ((canvas_h - s * (n + 1.0)) / n).max(1.0);
            for i in 0..count {
                frames.push(Rect {
                    x: s,
                    y: s + i as f32 * (cell_h + s),
                    w: cell_w,
                    h: cell_h,
                });
            }
        }
        CollageLayout::Grid => {
            let cols = (count as f64).sqrt().ceil() as usize;
            let rows = ((count as f64) / (cols as f64)).ceil() as usize;
            let cell_w = ((canvas_w - s * (cols as f32 + 1.0)) / cols as f32).max(1.0);
            let cell_h = ((canvas_h - s * (rows as f32 + 1.0)) / rows as f32).max(1.0);

            for i in 0..count {
                let col = i % cols;
                let row = i / cols;
                frames.push(Rect {
                    x: s + col as f32 * (cell_w + s),
                    y: s + row as f32 * (cell_h + s),
                    w: cell_w,
                    h: cell_h,
                });
            }
        }
        CollageLayout::Featured => {
            if count == 1 {
                frames.push(Rect {
                    x: s,
                    y: s,
                    w: (canvas_w - 2.0 * s).max(1.0),
                    h: (canvas_h - 2.0 * s).max(1.0),
                });
            } else {
                let feature_h = ((canvas_h - 3.0 * s) * 0.62).max(1.0);
                frames.push(Rect {
                    x: s,
                    y: s,
                    w: (canvas_w - 2.0 * s).max(1.0),
                    h: feature_h,
                });

                let rest_count = count - 1;
                let rest_n = rest_count as f32;
                let cell_w = ((canvas_w - s * (rest_n + 1.0)) / rest_n).max(1.0);
                let cell_h = ((canvas_h - 3.0 * s) * 0.38).max(1.0);
                let rest_y = s * 2.0 + feature_h;

                for i in 0..rest_count {
                    frames.push(Rect {
                        x: s + i as f32 * (cell_w + s),
                        y: rest_y,
                        w: cell_w,
                        h: cell_h,
                    });
                }
            }
        }
    }

    frames
}

/// Renders a multi-image collage onto an RGBA canvas.
pub fn render_collage(
    images: &[DynamicImage],
    settings: &CollageSettings,
) -> Result<RgbaImage, CollageError> {
    if images.is_empty() {
        return Err(CollageError::EmptyInput);
    }

    let cw = settings.width.max(64);
    let ch = settings.height.max(64);
    let mut canvas = RgbaImage::new(cw, ch);

    // 1. Fill background color
    let bg_px = Rgba(settings.background);
    for p in canvas.pixels_mut() {
        *p = bg_px;
    }

    // 2. Compute tile frames
    let frames = compute_frames(
        images.len(),
        cw as f32,
        ch as f32,
        settings.spacing,
        settings.layout,
    );

    // 3. Draw each image tile with aspect-fill and rounded corners
    for (img, frame) in images.iter().zip(frames.iter()) {
        let (iw, ih) = img.dimensions();
        if iw == 0 || ih == 0 || frame.w <= 0.0 || frame.h <= 0.0 {
            continue;
        }

        let scale = (frame.w / iw as f32).max(frame.h / ih as f32);
        let dw = ((iw as f32 * scale).round() as u32).max(1);
        let dh = ((ih as f32 * scale).round() as u32).max(1);

        let resized = img.resize_exact(dw, dh, imageops::FilterType::Triangle).to_rgba8();

        let ox = (frame.x + (frame.w - dw as f32) / 2.0).round() as i32;
        let oy = (frame.y + (frame.h - dh as f32) / 2.0).round() as i32;

        draw_clipped_tile(
            &mut canvas,
            &resized,
            ox,
            oy,
            frame,
            settings.corner_radius,
        );
    }

    Ok(canvas)
}

/// Draws an image into `canvas` clipped to `frame` with antialiased rounded corners.
fn draw_clipped_tile(
    canvas: &mut RgbaImage,
    tile: &RgbaImage,
    ox: i32,
    oy: i32,
    frame: &Rect,
    corner_radius: f32,
) {
    let cw = canvas.width() as i32;
    let ch = canvas.height() as i32;
    let tw = tile.width() as i32;
    let th = tile.height() as i32;

    let fx0 = frame.x.round() as i32;
    let fy0 = frame.y.round() as i32;
    let fx1 = (frame.x + frame.w).round() as i32;
    let fy1 = (frame.y + frame.h).round() as i32;

    let min_x = ox.max(fx0).max(0);
    let max_x = (ox + tw).min(fx1).min(cw);
    let min_y = oy.max(fy0).max(0);
    let max_y = (oy + th).min(fy1).min(ch);

    let r = corner_radius.min(frame.w / 2.0).min(frame.h / 2.0).max(0.0);

    for dest_y in min_y..max_y {
        let src_y = (dest_y - oy) as u32;
        let local_y = dest_y as f32 - frame.y;

        for dest_x in min_x..max_x {
            let src_x = (dest_x - ox) as u32;
            let local_x = dest_x as f32 - frame.x;

            let calpha = corner_alpha(local_x, local_y, frame.w, frame.h, r);
            if calpha <= 0.0 {
                continue;
            }

            let spx = tile.get_pixel(src_x, src_y);
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

/// Computes smooth sub-pixel alpha for rounded rectangle corners.
#[inline]
fn corner_alpha(x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    if r <= 0.5 {
        return 1.0;
    }

    let cx = if x < r {
        r
    } else if x > w - r {
        w - r
    } else {
        return 1.0;
    };

    let cy = if y < r {
        r
    } else if y > h - r {
        h - r
    } else {
        return 1.0;
    };

    let dx = x - cx;
    let dy = y - cy;
    let dist = (dx * dx + dy * dy).sqrt();

    if dist <= r - 0.5 {
        1.0
    } else if dist >= r + 0.5 {
        0.0
    } else {
        (r + 0.5 - dist).clamp(0.0, 1.0)
    }
}

/// Creates a collage file from input paths and saves it to `output`.
pub fn create_collage_file(
    inputs: &[PathBuf],
    output: &Path,
    settings: &CollageSettings,
) -> Result<(), CollageError> {
    if inputs.is_empty() {
        return Err(CollageError::EmptyInput);
    }

    let mut images = Vec::with_capacity(inputs.len());
    for p in inputs {
        let img = image::open(p).map_err(|e| CollageError::ReadFailed(e.to_string()))?;
        images.push(img);
    }

    let collage = render_collage(&images, settings)?;

    if let Some(parent) = output.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext == "jpg" || ext == "jpeg" {
        let rgb = image::DynamicImage::ImageRgba8(collage).into_rgb8();
        rgb.save(output)
            .map_err(|e| CollageError::SaveFailed(e.to_string()))
    } else {
        collage
            .save(output)
            .map_err(|e| CollageError::SaveFailed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_frames() {
        let frames_grid = compute_frames(4, 1000.0, 1000.0, 20.0, CollageLayout::Grid);
        assert_eq!(frames_grid.len(), 4);
        assert!(frames_grid[0].w > 400.0);
        assert!(frames_grid[0].h > 400.0);

        let frames_cols = compute_frames(3, 900.0, 600.0, 10.0, CollageLayout::Columns);
        assert_eq!(frames_cols.len(), 3);
        assert!((frames_cols[0].w - 286.66).abs() < 1.0);

        let frames_rows = compute_frames(2, 600.0, 800.0, 10.0, CollageLayout::Rows);
        assert_eq!(frames_rows.len(), 2);
        assert_eq!(frames_rows[0].w, 580.0);

        let frames_featured = compute_frames(3, 1000.0, 1000.0, 10.0, CollageLayout::Featured);
        assert_eq!(frames_featured.len(), 3);
        assert_eq!(frames_featured[0].w, 980.0);
    }

    #[test]
    fn test_render_collage_solid() {
        let img1 = DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 200, Rgba([255, 0, 0, 255])));
        let img2 = DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 200, Rgba([0, 0, 255, 255])));

        let settings = CollageSettings {
            layout: CollageLayout::Columns,
            width: 400,
            height: 200,
            spacing: 10.0,
            corner_radius: 8.0,
            background: [0, 0, 0, 255],
        };

        let result = render_collage(&[img1, img2], &settings).expect("renders ok");
        assert_eq!(result.width(), 400);
        assert_eq!(result.height(), 200);

        // Verify background at origin (0, 0) is black
        assert_eq!(result.get_pixel(0, 0), &Rgba([0, 0, 0, 255]));
    }
}
