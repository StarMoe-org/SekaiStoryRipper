//! Decoded textures → PNG (decision D8). Pixels are written as decoded; no alpha conversion.

use ripper_unity::RgbaImage;

use crate::{Result, malformed};

pub fn encode_png(image: &RgbaImage) -> Result<Vec<u8>> {
    let expected = image.width as usize * image.height as usize * 4;
    if image.pixels.len() != expected {
        return Err(malformed(
            "texture",
            format!(
                "{} bytes for {}x{} RGBA",
                image.pixels.len(),
                image.width,
                image.height
            ),
        ));
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.pixels)?;
    writer.finish()?;
    Ok(out)
}

/// Wraps mip 0 of an ASTC texture (Unity formats 48–59) in a standard `.astc` file: 16-byte header
/// (magic `0x5CA1AB13`, block size, 24-bit extents) followed by the blocks. Rows stay in Unity's
/// bottom-up order. `None` for non-ASTC formats.
pub fn astc_file(raw: &ripper_unity::RawTexture) -> Option<Vec<u8>> {
    if !(48..=59).contains(&raw.format) {
        return None;
    }
    let block = [4u32, 5, 6, 8, 10, 12][((raw.format - 48) % 6) as usize];
    let mip0 = raw.width.div_ceil(block) as usize * raw.height.div_ceil(block) as usize * 16;
    let blocks = raw.data.get(..mip0)?;
    let mut out = Vec::with_capacity(16 + mip0);
    out.extend_from_slice(&0x5CA1_AB13u32.to_le_bytes());
    out.extend_from_slice(&[block as u8, block as u8, 1]);
    for extent in [raw.width, raw.height, 1] {
        out.extend_from_slice(&extent.to_le_bytes()[..3]);
    }
    out.extend_from_slice(blocks);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_astc_mip0_with_a_standard_header() {
        let raw = ripper_unity::RawTexture {
            width: 12,
            height: 7,
            format: 50,
            data: vec![7; 2 * 2 * 16 + 100],
        };
        let file = astc_file(&raw).unwrap();
        assert_eq!(&file[..4], &[0x13, 0xAB, 0xA1, 0x5C]);
        assert_eq!(&file[4..7], &[6, 6, 1]);
        assert_eq!(&file[7..10], &[12, 0, 0]);
        assert_eq!(&file[10..13], &[7, 0, 0]);
        assert_eq!(file.len(), 16 + 64);
        assert!(astc_file(&ripper_unity::RawTexture { format: 4, ..raw }).is_none());
    }

    #[test]
    fn encodes_and_rejects_wrong_sizes() {
        let image = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 255, 0, 128],
        };
        let png = encode_png(&image).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let broken = RgbaImage {
            width: 2,
            height: 2,
            pixels: vec![0; 4],
        };
        assert!(encode_png(&broken).is_err());
    }
}
