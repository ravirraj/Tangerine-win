//! Vector annotations for images (P3-14 / `tools.annotate`).
//!
//! Renders arrows, boxes, text callouts, pen strokes, and blur/privacy
//! overlays onto photographs. Uses normalized coordinates (0.0 .. 1.0)
//! so annotations scale cleanly between interactive stage previews and
//! full-resolution output images.

use std::path::Path;
use image::{imageops, DynamicImage, Rgba, RgbaImage};

#[derive(Clone, Debug, PartialEq)]
pub enum Annotation {
    Arrow {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        color: [u8; 4],
        width: f32,
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [u8; 4],
        width: f32,
        fill: bool,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        color: [u8; 4],
        scale: u32,
    },
    Blur {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    #[allow(dead_code)]
    Pen {
        points: Vec<(f32, f32)>,
        color: [u8; 4],
        width: f32,
    },
}

impl Eq for Annotation {}

#[derive(Debug)]
pub enum AnnotateError {
    ReadFailed(String),
    SaveFailed(String),
}

impl std::fmt::Display for AnnotateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadFailed(msg) => write!(f, "Failed to read image: {msg}"),
            Self::SaveFailed(msg) => write!(f, "Failed to save annotated image: {msg}"),
        }
    }
}

impl std::error::Error for AnnotateError {}

/// Standard 8x8 font bitmap for ASCII characters 32..127.
/// Each character is represented by 8 bytes (each byte is 1 row of 8 bits).
const FONT_8X8: [[u8; 8]; 96] = [
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00], // 32 ' '
    [0x18,0x3C,0x3C,0x18,0x18,0x00,0x18,0x00], // 33 '!'
    [0x66,0x66,0x24,0x00,0x00,0x00,0x00,0x00], // 34 '"'
    [0x6C,0x6C,0xFE,0x6C,0xFE,0x6C,0x6C,0x00], // 35 '#'
    [0x18,0x3E,0x60,0x3C,0x06,0x7C,0x18,0x00], // 36 '$'
    [0x00,0x66,0xA6,0xD4,0x2B,0x65,0x66,0x00], // 37 '%'
    [0x38,0x6C,0x38,0x76,0xDC,0xCC,0x76,0x00], // 38 '&'
    [0x30,0x30,0x10,0x20,0x00,0x00,0x00,0x00], // 39 '\''
    [0x0C,0x18,0x30,0x30,0x30,0x18,0x0C,0x00], // 40 '('
    [0x30,0x18,0x0C,0x0C,0x0C,0x18,0x30,0x00], // 41 ')'
    [0x00,0x66,0x3C,0xFF,0x3C,0x66,0x00,0x00], // 42 '*'
    [0x00,0x18,0x18,0x7E,0x18,0x18,0x00,0x00], // 43 '+'
    [0x00,0x00,0x00,0x00,0x00,0x18,0x18,0x30], // 44 ','
    [0x00,0x00,0x00,0x7E,0x00,0x00,0x00,0x00], // 45 '-'
    [0x00,0x00,0x00,0x00,0x00,0x18,0x18,0x00], // 46 '.'
    [0x06,0x0C,0x18,0x30,0x60,0xC0,0x80,0x00], // 47 '/'
    [0x7C,0xC6,0xCE,0xD6,0xE6,0xC6,0x7C,0x00], // 48 '0'
    [0x18,0x38,0x18,0x18,0x18,0x18,0x7E,0x00], // 49 '1'
    [0x7C,0xC6,0x06,0x1C,0x30,0x66,0xFE,0x00], // 50 '2'
    [0x7C,0xC6,0x06,0x3C,0x06,0xC6,0x7C,0x00], // 51 '3'
    [0x1C,0x3C,0x6C,0xCC,0xFE,0x0C,0x1E,0x00], // 52 '4'
    [0xFE,0xC0,0xFC,0x06,0x06,0xC6,0x7C,0x00], // 53 '5'
    [0x78,0x0C,0x06,0x7C,0xC6,0xC6,0x7C,0x00], // 54 '6'
    [0xFE,0xC6,0x0C,0x18,0x30,0x30,0x30,0x00], // 55 '7'
    [0x7C,0xC6,0xC6,0x7C,0xC6,0xC6,0x7C,0x00], // 56 '8'
    [0x7C,0xC6,0xC6,0x7E,0x06,0x0C,0x78,0x00], // 57 '9'
    [0x00,0x18,0x18,0x00,0x18,0x18,0x00,0x00], // 58 ':'
    [0x00,0x18,0x18,0x00,0x18,0x18,0x30,0x00], // 59 ';'
    [0x06,0x0C,0x18,0x30,0x18,0x0C,0x06,0x00], // 60 '<'
    [0x00,0x00,0x7E,0x00,0x7E,0x00,0x00,0x00], // 61 '='
    [0x60,0x30,0x18,0x0C,0x18,0x30,0x60,0x00], // 62 '>'
    [0x7C,0xC6,0x0C,0x18,0x18,0x00,0x18,0x00], // 63 '?'
    [0x7C,0xC6,0xDE,0xDE,0xDE,0xC0,0x78,0x00], // 64 '@'
    [0x38,0x6C,0xC6,0xFE,0xC6,0xC6,0xC6,0x00], // 65 'A'
    [0xFC,0x66,0x66,0x7C,0x66,0x66,0xFC,0x00], // 66 'B'
    [0x3C,0x66,0xC0,0xC0,0xC0,0x66,0x3C,0x00], // 67 'C'
    [0xF8,0x6C,0x66,0x66,0x66,0x6C,0xF8,0x00], // 68 'D'
    [0xFE,0x62,0x68,0x78,0x68,0x62,0xFE,0x00], // 69 'E'
    [0xFE,0x62,0x68,0x78,0x68,0x60,0xF0,0x00], // 70 'F'
    [0x3C,0x66,0xC0,0xC0,0xCE,0x66,0x3E,0x00], // 71 'G'
    [0xC6,0xC6,0xC6,0xFE,0xC6,0xC6,0xC6,0x00], // 72 'H'
    [0x3C,0x18,0x18,0x18,0x18,0x18,0x3C,0x00], // 73 'I'
    [0x1E,0x0C,0x0C,0x0C,0xCC,0xCC,0x78,0x00], // 74 'J'
    [0xE6,0x66,0x6C,0x78,0x6C,0x66,0xE6,0x00], // 75 'K'
    [0xF0,0x60,0x60,0x60,0x62,0x66,0xFE,0x00], // 76 'L'
    [0xC6,0xEE,0xFE,0xFE,0xD6,0xC6,0xC6,0x00], // 77 'M'
    [0xC6,0xE6,0xF6,0xDE,0xCE,0xC6,0xC6,0x00], // 78 'N'
    [0x7C,0xC6,0xC6,0xC6,0xC6,0xC6,0x7C,0x00], // 79 'O'
    [0xFC,0x66,0x66,0x7C,0x60,0x60,0xF0,0x00], // 80 'P'
    [0x7C,0xC6,0xC6,0xC6,0xD6,0xDE,0x7C,0x0E], // 81 'Q'
    [0xFC,0x66,0x66,0x7C,0x6C,0x66,0xE6,0x00], // 82 'R'
    [0x7C,0xC6,0x60,0x38,0x0C,0xC6,0x7C,0x00], // 83 'S'
    [0x7E,0x7E,0x18,0x18,0x18,0x18,0x18,0x00], // 84 'T'
    [0xC6,0xC6,0xC6,0xC6,0xC6,0xC6,0x7C,0x00], // 85 'U'
    [0xC6,0xC6,0xC6,0xC6,0xC6,0x6C,0x38,0x00], // 86 'V'
    [0xC6,0xC6,0xD6,0xFE,0xFE,0xEE,0xC6,0x00], // 87 'W'
    [0xC6,0xC6,0x6C,0x38,0x6C,0xC6,0xC6,0x00], // 88 'X'
    [0x66,0x66,0x66,0x3C,0x18,0x18,0x18,0x00], // 89 'Y'
    [0xFE,0xC6,0x0C,0x18,0x30,0x63,0xFE,0x00], // 90 'Z'
    [0x3C,0x30,0x30,0x30,0x30,0x30,0x3C,0x00], // 91 '['
    [0xC0,0x60,0x30,0x18,0x0C,0x06,0x02,0x00], // 92 '\'
    [0x3C,0x0C,0x0C,0x0C,0x0C,0x0C,0x3C,0x00], // 93 ']'
    [0x10,0x38,0x6C,0xC6,0x00,0x00,0x00,0x00], // 94 '^'
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0xFF], // 95 '_'
    [0x30,0x18,0x0C,0x00,0x00,0x00,0x00,0x00], // 96 '`'
    [0x00,0x00,0x78,0x0C,0x7C,0xCC,0x76,0x00], // 97 'a'
    [0xE0,0x60,0x7C,0x66,0x66,0x66,0xDC,0x00], // 98 'b'
    [0x00,0x00,0x78,0xCC,0xC0,0xCC,0x78,0x00], // 99 'c'
    [0x1C,0x0C,0x7C,0xCC,0xCC,0xCC,0x76,0x00], // 100 'd'
    [0x00,0x00,0x78,0xCC,0xFC,0xC0,0x78,0x00], // 101 'e'
    [0x38,0x6C,0x60,0xF0,0x60,0x60,0xF0,0x00], // 102 'f'
    [0x00,0x00,0x76,0xCC,0xCC,0x7C,0x0C,0xF8], // 103 'g'
    [0xE0,0x60,0x6C,0x76,0x66,0x66,0xE6,0x00], // 104 'h'
    [0x18,0x00,0x38,0x18,0x18,0x18,0x3C,0x00], // 105 'i'
    [0x0C,0x00,0x1C,0x0C,0x0C,0xCC,0xCC,0x78], // 106 'j'
    [0xE0,0x60,0x66,0x6C,0x78,0x6C,0xE6,0x00], // 107 'k'
    [0x38,0x18,0x18,0x18,0x18,0x18,0x3C,0x00], // 108 'l'
    [0x00,0x00,0xEC,0xFE,0xD6,0xD6,0xD6,0x00], // 109 'm'
    [0x00,0x00,0xDC,0x66,0x66,0x66,0x66,0x00], // 110 'n'
    [0x00,0x00,0x7C,0xC6,0xC6,0xC6,0x7C,0x00], // 111 'o'
    [0x00,0x00,0xDC,0x66,0x66,0x7C,0x60,0xF0], // 112 'p'
    [0x00,0x00,0x76,0xCC,0xCC,0x7C,0x0C,0x1E], // 113 'q'
    [0x00,0x00,0xDC,0x76,0x60,0x60,0xF0,0x00], // 114 'r'
    [0x00,0x00,0x7C,0xC0,0x7C,0x06,0xFC,0x00], // 115 's'
    [0x30,0x30,0xFC,0x30,0x30,0x34,0x18,0x00], // 116 't'
    [0x00,0x00,0xCC,0xCC,0xCC,0xCC,0x76,0x00], // 117 'u'
    [0x00,0x00,0xC6,0xC6,0xC6,0x6C,0x38,0x00], // 118 'v'
    [0x00,0x00,0xC6,0xD6,0xFE,0xEE,0xC6,0x00], // 119 'w'
    [0x00,0x00,0xC6,0x6C,0x38,0x6C,0xC6,0x00], // 120 'x'
    [0x00,0x00,0xC6,0xC6,0xCC,0x78,0x30,0x60], // 121 'y'
    [0x00,0x00,0xFE,0x4C,0x18,0x32,0xFE,0x00], // 122 'z'
    [0x0E,0x18,0x18,0x70,0x18,0x18,0x0E,0x00], // 123 '{'
    [0x18,0x18,0x18,0x00,0x18,0x18,0x18,0x00], // 124 '|'
    [0x70,0x18,0x18,0x0E,0x18,0x18,0x70,0x00], // 125 '}'
    [0x76,0xDC,0x00,0x00,0x00,0x00,0x00,0x00], // 126 '~'
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00], // 127
];

/// Renders a list of annotations onto an image.
pub fn render_annotations(
    source: &DynamicImage,
    annotations: &[Annotation],
) -> RgbaImage {
    let mut canvas = source.to_rgba8();
    let cw = canvas.width() as f32;
    let ch = canvas.height() as f32;

    for ann in annotations {
        match ann {
            Annotation::Arrow { x0, y0, x1, y1, color, width } => {
                let px0 = x0 * cw;
                let py0 = y0 * ch;
                let px1 = x1 * cw;
                let py1 = y1 * ch;
                draw_arrow(&mut canvas, px0, py0, px1, py1, *width, *color);
            }
            Annotation::Rect { x, y, w, h, color, width, fill } => {
                let px = x * cw;
                let py = y * ch;
                let pw = w * cw;
                let ph = h * ch;
                draw_rect(&mut canvas, px, py, pw, ph, *width, *fill, *color);
            }
            Annotation::Text { x, y, text, color, scale } => {
                let px = (x * cw).round() as i32;
                let py = (y * ch).round() as i32;
                draw_text(&mut canvas, px, py, text, *scale, *color);
            }
            Annotation::Blur { x, y, w, h } => {
                let px = (x * cw).round().max(0.0) as u32;
                let py = (y * ch).round().max(0.0) as u32;
                let pw = (w * cw).round().min(cw - px as f32) as u32;
                let ph = (h * ch).round().min(ch - py as f32) as u32;
                if pw > 0 && ph > 0 {
                    blur_region(&mut canvas, px, py, pw, ph);
                }
            }
            Annotation::Pen { points, color, width } => {
                if points.len() >= 2 {
                    for i in 0..points.len() - 1 {
                        let (x0, y0) = points[i];
                        let (x1, y1) = points[i + 1];
                        draw_thick_line(
                            &mut canvas,
                            x0 * cw,
                            y0 * ch,
                            x1 * cw,
                            y1 * ch,
                            *width,
                            *color,
                        );
                    }
                }
            }
        }
    }

    canvas
}

/// Draws an arrow from (x0, y0) to (x1, y1) with head at (x1, y1).
fn draw_arrow(
    canvas: &mut RgbaImage,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    width: f32,
    color: [u8; 4],
) {
    // 1. Shaft line
    draw_thick_line(canvas, x0, y0, x1, y1, width, color);

    // 2. Arrowhead wings
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 4.0 {
        return;
    }

    let ux = dx / len;
    let uy = dy / len;
    let head_len = (width * 4.0).max(16.0).min(len * 0.4);
    let head_angle: f32 = 0.52; // ~30 degrees in radians

    let cos_a = head_angle.cos();
    let sin_a = head_angle.sin();

    // Wing 1
    let rx1 = -(ux * cos_a - uy * sin_a) * head_len;
    let ry1 = -(ux * sin_a + uy * cos_a) * head_len;
    draw_thick_line(canvas, x1, y1, x1 + rx1, y1 + ry1, width, color);

    // Wing 2
    let rx2 = -(ux * cos_a + uy * sin_a) * head_len;
    let ry2 = -(-ux * sin_a + uy * cos_a) * head_len;
    draw_thick_line(canvas, x1, y1, x1 + rx2, y1 + ry2, width, color);
}

/// Draws a rectangle outline or filled rectangle.
fn draw_rect(
    canvas: &mut RgbaImage,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    width: f32,
    fill: bool,
    color: [u8; 4],
) {
    let x0 = x.min(x + w);
    let y0 = y.min(y + h);
    let x1 = x.max(x + w);
    let y1 = y.max(y + h);

    if fill {
        let cw = canvas.width() as i32;
        let ch = canvas.height() as i32;
        let min_x = (x0.round() as i32).max(0);
        let max_x = (x1.round() as i32).min(cw);
        let min_y = (y0.round() as i32).max(0);
        let max_y = (y1.round() as i32).min(ch);

        let blend_alpha = color[3] as f32 / 255.0;
        let inv_alpha = 1.0 - blend_alpha;

        for py in min_y..max_y {
            for px in min_x..max_x {
                let p = canvas.get_pixel_mut(px as u32, py as u32);
                p.0[0] = (color[0] as f32 * blend_alpha + p.0[0] as f32 * inv_alpha).round() as u8;
                p.0[1] = (color[1] as f32 * blend_alpha + p.0[1] as f32 * inv_alpha).round() as u8;
                p.0[2] = (color[2] as f32 * blend_alpha + p.0[2] as f32 * inv_alpha).round() as u8;
            }
        }
    } else {
        // Outline 4 edges
        draw_thick_line(canvas, x0, y0, x1, y0, width, color);
        draw_thick_line(canvas, x1, y0, x1, y1, width, color);
        draw_thick_line(canvas, x1, y1, x0, y1, width, color);
        draw_thick_line(canvas, x0, y1, x0, y0, width, color);
    }
}

/// Draws an antialiased thick line segment from (x0, y0) to (x1, y1).
fn draw_thick_line(
    canvas: &mut RgbaImage,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    width: f32,
    color: [u8; 4],
) {
    let cw = canvas.width() as i32;
    let ch = canvas.height() as i32;
    let radius = width / 2.0;

    let min_x = ((x0.min(x1) - radius - 1.0).round() as i32).max(0);
    let max_x = ((x0.max(x1) + radius + 1.0).round() as i32).min(cw);
    let min_y = ((y0.min(y1) - radius - 1.0).round() as i32).max(0);
    let max_y = ((y0.max(y1) + radius + 1.0).round() as i32).min(ch);

    let dx = x1 - x0;
    let dy = y1 - y0;
    let len_sq = dx * dx + dy * dy;

    let col_rgba = Rgba(color);
    let blend_a = color[3] as f32 / 255.0;

    for y in min_y..max_y {
        for x in min_x..max_x {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;

            // Distance to line segment
            let dist = if len_sq <= 0.001 {
                ((px - x0) * (px - x0) + (py - y0) * (py - y0)).sqrt()
            } else {
                let t = (((px - x0) * dx + (py - y0) * dy) / len_sq).clamp(0.0, 1.0);
                let proj_x = x0 + t * dx;
                let proj_y = y0 + t * dy;
                ((px - proj_x) * (px - proj_x) + (py - proj_y) * (py - proj_y)).sqrt()
            };

            if dist <= radius - 0.5 {
                canvas.put_pixel(x as u32, y as u32, col_rgba);
            } else if dist < radius + 0.5 {
                let cov = (radius + 0.5 - dist).clamp(0.0, 1.0) * blend_a;
                let inv = 1.0 - cov;
                let p = canvas.get_pixel_mut(x as u32, y as u32);
                p.0[0] = (color[0] as f32 * cov + p.0[0] as f32 * inv).round() as u8;
                p.0[1] = (color[1] as f32 * cov + p.0[1] as f32 * inv).round() as u8;
                p.0[2] = (color[2] as f32 * cov + p.0[2] as f32 * inv).round() as u8;
            }
        }
    }
}

/// Draws text onto the canvas using the embedded 8x8 font.
fn draw_text(
    canvas: &mut RgbaImage,
    origin_x: i32,
    origin_y: i32,
    text: &str,
    scale: u32,
    color: [u8; 4],
) {
    let scale = scale.max(1);
    let cw = canvas.width() as i32;
    let ch = canvas.height() as i32;
    let col = Rgba(color);

    let mut cursor_x = origin_x;
    let cursor_y = origin_y;

    for ch_byte in text.bytes() {
        if ch_byte == b'\n' {
            continue;
        }
        let font_idx = if ch_byte >= 32 && ch_byte <= 127 {
            (ch_byte - 32) as usize
        } else {
            0 // space
        };

        let glyph = FONT_8X8[font_idx];

        for row in 0..8 {
            let row_byte = glyph[row];
            for col_idx in 0..8 {
                if (row_byte & (0x80 >> col_idx)) != 0 {
                    for sy in 0..scale {
                        let py = cursor_y + (row as u32 * scale + sy) as i32;
                        if py < 0 || py >= ch {
                            continue;
                        }
                        for sx in 0..scale {
                            let px = cursor_x + (col_idx as u32 * scale + sx) as i32;
                            if px < 0 || px >= cw {
                                continue;
                            }
                            canvas.put_pixel(px as u32, py as u32, col);
                        }
                    }
                }
            }
        }

        cursor_x += (8 * scale) as i32;
    }
}

/// Blurs a rectangular subregion of the canvas.
fn blur_region(canvas: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32) {
    if w == 0 || h == 0 || x + w > canvas.width() || y + h > canvas.height() {
        return;
    }

    let sub = imageops::crop_imm(canvas, x, y, w, h).to_image();
    let blurred = imageops::blur(&sub, 12.0);
    imageops::overlay(canvas, &blurred, x as i64, y as i64);
}

/// Applies annotations to an image file and saves the output.
pub fn annotate_file(
    input: &Path,
    output: &Path,
    annotations: &[Annotation],
) -> Result<(), AnnotateError> {
    let img = image::open(input).map_err(|e| AnnotateError::ReadFailed(e.to_string()))?;
    let annotated = render_annotations(&img, annotations);

    if let Some(parent) = output.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext == "jpg" || ext == "jpeg" {
        let rgb = DynamicImage::ImageRgba8(annotated).into_rgb8();
        rgb.save(output)
            .map_err(|e| AnnotateError::SaveFailed(e.to_string()))
    } else {
        annotated
            .save(output)
            .map_err(|e| AnnotateError::SaveFailed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_annotations_arrow_rect_text() {
        let base = DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 200, Rgba([255, 255, 255, 255])));

        let annotations = vec![
            Annotation::Rect {
                x: 0.1,
                y: 0.1,
                w: 0.4,
                h: 0.4,
                color: [255, 92, 0, 255],
                width: 4.0,
                fill: false,
            },
            Annotation::Arrow {
                x0: 0.5,
                y0: 0.5,
                x1: 0.8,
                y1: 0.8,
                color: [10, 132, 255, 255],
                width: 4.0,
            },
            Annotation::Text {
                x: 0.1,
                y: 0.8,
                text: "TEST".to_string(),
                color: [0, 0, 0, 255],
                scale: 2,
            },
            Annotation::Blur {
                x: 0.6,
                y: 0.1,
                w: 0.3,
                h: 0.3,
            },
        ];

        let result = render_annotations(&base, &annotations);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 200);

        // Verify pixel changes from white
        let mut modified_pixels = 0;
        for p in result.pixels() {
            if p != &Rgba([255, 255, 255, 255]) {
                modified_pixels += 1;
            }
        }
        assert!(modified_pixels > 100, "Annotations were drawn onto canvas");
    }
}
