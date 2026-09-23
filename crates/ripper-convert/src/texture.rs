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

#[cfg(test)]
mod tests {
    use super::*;

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
