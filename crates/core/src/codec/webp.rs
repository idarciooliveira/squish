//! WebP pipeline (PRD 4.3, ADR-0028): decode with image-webp, re-encode with libwebp at a fixed quality.

use std::io::Cursor;

use crate::error::Error;

fn decode(bytes: &[u8], max_pixels: u64) -> Result<(Vec<u8>, u32, u32, bool), Error> {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(|e| Error::Decode(e.to_string()))?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > max_pixels {
        return Err(Error::Decode("image is too large".into()));
    }
    let alpha = decoder.has_alpha();
    let size = decoder.output_buffer_size().ok_or_else(|| Error::Decode("image too large".into()))?;
    let mut pixels = vec![0u8; size];
    decoder.read_image(&mut pixels).map_err(|e| Error::Decode(e.to_string()))?;
    Ok((pixels, width, height, alpha))
}

/// Decodes to RGBA8 for scoring.
pub(crate) fn decode_rgba(bytes: &[u8], max_pixels: u64) -> Result<(Vec<u8>, u32, u32), Error> {
    let (pixels, width, height, alpha) = decode(bytes, max_pixels)?;
    // `read_image` writes RGBA when the file has alpha and RGB otherwise.
    let rgba = if alpha { pixels } else { pixels.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect() };
    Ok((rgba, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32) -> Vec<u8> {
        let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
        for y in 0..height {
            for x in 0..width {
                rgb.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8]);
            }
        }
        rgb
    }

    #[test]
    fn lossless_round_trip_decodes_to_the_same_pixels() {
        let rgb = gradient(32, 24);
        let encoded = webp::Encoder::from_rgb(&rgb, 32, 24).encode_lossless();
        let (rgba, width, height) = decode_rgba(&encoded, 10_000).unwrap();
        assert_eq!((width, height), (32, 24));
        let expected: Vec<u8> = rgb.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        assert_eq!(rgba, expected);
    }

    #[test]
    fn garbage_after_a_webp_header_is_a_decode_error() {
        let mut bytes = b"RIFF\x10\x00\x00\x00WEBP".to_vec();
        bytes.extend_from_slice(&[0xAA; 16]);
        assert!(matches!(decode_rgba(&bytes, 10_000), Err(Error::Decode(_))));
    }
}
