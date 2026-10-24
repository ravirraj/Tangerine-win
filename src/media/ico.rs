//! ICO container wrapper.
//!
//! The conversion program writes a PNG or BMP through the pipe. This wraps
//! that image in a single-entry ICO container so the result can be used
//! as a Windows icon. The container is 6-byte directory + 16-byte entry + image.

/// Wraps PNG/BMP bytes in a 1-entry ICO file.
/// Returns None if the image is too large (>256) or the header is bad.
pub fn wrap(image: &[u8], is_png: bool) -> Option<Vec<u8>> {
    // PNG signature
    const PNG_SIG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    // BMP signature
    const BMP_SIG: &[u8] = &[0x42, 0x4D];

    if is_png && image.len() < 24 {
        return None;
    }
    if !is_png && image.len() < 54 {
        return None;
    }

    // Read dimensions
    let (w, h): (u32, u32) = if is_png {
        if &image[..8] != PNG_SIG {
            return None;
        }
        // IHDR at offset 16 (4 bytes width, 4 bytes height, big-endian)
        let w = u32::from_be_bytes([image[16], image[17], image[18], image[19]]);
        let h = u32::from_be_bytes([image[20], image[21], image[22], image[23]]);
        (w, h)
    } else {
        if &image[..2] != BMP_SIG {
            return None;
        }
        let w = u32::from_le_bytes([image[18], image[19], image[20], image[21]]);
        let h = u32::from_le_bytes([image[22], image[23], image[24], image[25]]);
        (w, h)
    };

    if w == 0 || h == 0 || w > 256 || h > 256 {
        return None;
    }

    let mut out = Vec::with_capacity(22 + image.len());
    // Reserved / Type / Count
    out.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    // Directory entry: width, height (0 = 256), colors (0), reserved (0), planes (1), bitcount (32)
    out.push(if w >= 256 { 0 } else { w as u8 });
    out.push(if h >= 256 { 0 } else { h as u8 });
    out.extend_from_slice(&[0, 0, 1, 0, 32, 0]);
    // Size and offset
    let size = image.len() as u32;
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&[22, 0, 0, 0]); // offset 22
    // Image payload
    out.extend_from_slice(image);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_png_is_wrapped() {
        let png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
                   0, 0, 0, 13, b'I', b'H', b'D', b'R',
                   0, 0, 2, 0, 0, 0, 1, 0]; // 512 x 256
        let ico = wrap(&png, true).expect("png wrapped");
        assert_eq!(ico.len(), 22 + png.len());
        assert_eq!(ico[0], 0); assert_eq!(ico[2], 1); // reserved=0, type=1
        assert_eq!(ico[4], 1); // count=1
    }

    #[test]
    fn too_large_is_refused() {
        let png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
                   0, 0, 0, 13, b'I', b'H', b'D', b'R',
                   0, 0, 1, 0, 0, 0, 2, 0]; // 256 x 512 -> h > 256
        assert!(wrap(&png, true).is_none());
    }
}
