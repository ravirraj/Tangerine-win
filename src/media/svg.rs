//! SVG rasterisation via resvg.
//!
//! resvg is already vendored transitively through Slint 1.18. This module
//! rasterises an SVG file to a PNG in memory, which can then be fed through
//! the existing FFmpeg image conversion pipeline.
//!
//! No FFmpeg demuxer/decoder exists for SVG, so this is a non-FFmpeg seam.

use std::path::Path;

use resvg::tiny_skia::Pixmap;
use usvg::Tree;
use usvg::Options;

/// Why an SVG rasterisation did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvgError {
    /// The SVG data could not be parsed.
    InvalidSvg,
    /// The rasterised picture could not be created.
    RenderFailed,
}

/// Rasterise an SVG file to PNG bytes in memory.
///
/// Returns the PNG-encoded bytes of the rasterised picture.
pub fn rasterise(input: &Path) -> Result<Vec<u8>, SvgError> {
    let data = std::fs::read(input).map_err(|_| SvgError::InvalidSvg)?;
    let tree = Tree::from_data(&data, &Options::default())
        .map_err(|_| SvgError::InvalidSvg)?;

    let size = tree.size();
    let width = size.width() as u32;
    let height = size.height() as u32;

    if width == 0 || height == 0 {
        return Err(SvgError::RenderFailed);
    }

    let mut pixmap = Pixmap::new(width, height)
        .ok_or(SvgError::RenderFailed)?;

    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixmap.as_mut());

    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|_| SvgError::RenderFailed)?;
        writer.write_image_data(&pixmap.data()).map_err(|_| SvgError::RenderFailed)?;
    }
    Ok(png_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_rasterises_to_png() {
        let svg = r#"<svg width="100" height="100" xmlns="http://www.w3.org/2000/svg">
  <rect width="100" height="100" fill="red"/>
</svg>"#;

        let svg_bytes = svg.as_bytes();
        let tree = Tree::from_data(svg_bytes, &Options::default()).expect("svg should parse");
        let size = tree.size();
        assert!(size.width() > 0.0);
        assert!(size.height() > 0.0);
    }

    #[test]
    fn invalid_svg_is_rejected() {
        let svg = b"not an svg";
        let tree = Tree::from_data(svg, &Options::default());
        assert!(tree.is_err());
    }

    #[test]
    fn empty_svg_is_refused() {
        let svg = b"";
        let tree = Tree::from_data(svg, &Options::default());
        assert!(tree.is_err());
    }
}
