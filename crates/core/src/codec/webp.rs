//! WebP pipeline (PRD 4.3, ADR-0028): decode with image-webp, re-encode with libwebp at a fixed quality.

use std::io::Cursor;

use super::Encoded;
use crate::Report;
use crate::error::Error;
use crate::format::{self, Format};
use crate::metadata;
use crate::options::{Effort, Options, Quality};

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

/// libwebp quality for lossy files at every effort (ADR-0028).
pub(crate) const QUALITY: u8 = 71;

fn method(effort: Effort) -> i32 {
    match effort {
        Effort::Fast => 2,
        Effort::Default => 4,
        Effort::Max => 6,
    }
}

fn encode(
    pixels: &[u8],
    width: u32,
    height: u32,
    alpha: bool,
    quality: Option<u8>,
    effort: Effort,
) -> Result<Vec<u8>, Error> {
    let encoder = if alpha {
        webp::Encoder::from_rgba(pixels, width, height)
    } else {
        webp::Encoder::from_rgb(pixels, width, height)
    };
    let mut config = webp::WebPConfig::new().map_err(|()| Error::Encode("libwebp config".into()))?;
    config.method = method(effort);
    match quality {
        Some(q) => {
            config.quality = f32::from(q);
            config.alpha_compression = 1;
        }
        None => {
            config.lossless = 1;
            config.exact = 1; // keep RGB under transparent pixels: lossless means no pixel changes
        }
    }
    let memory = encoder.encode_advanced(&config).map_err(|e| Error::Encode(format!("{e:?}")))?;
    Ok(memory.to_vec())
}

pub(crate) fn compress(bytes: &[u8], options: &Options) -> Result<Encoded, Error> {
    let lossless = metadata::webp_is_lossless(bytes);
    let meta = metadata::webp_meta(bytes, options.metadata);
    if !lossless && options.quality == Quality::Lossless {
        // A lossy file stays lossy: only the metadata policy runs, on the original bitstream.
        // A truncated file can still have readable dimensions, so require the complete
        // bitstream chunk: without it the assemble step emits a header-only file that the
        // size guard mistakes for a win.
        if !metadata::webp_chunks(bytes).iter().any(|c| &c.kind == b"VP8 ") {
            return Err(Error::Decode("the WebP bitstream is truncated".into()));
        }
        let (width, height) = format::dimensions(bytes, Format::WebP)
            .ok_or_else(|| Error::Decode("could not read the WebP canvas size".into()))?;
        return Ok(Encoded { bytes: metadata::webp_assemble(bytes, &meta, width, height), report: Report::default() });
    }
    let (pixels, width, height, alpha) = decode(bytes, options.max_pixels)?;
    let quality = match (lossless, options.quality) {
        (true, _) => None,
        (false, Quality::Fixed(n)) => Some(n),
        (false, _) => Some(QUALITY),
    };
    let encoded = encode(&pixels, width, height, alpha, quality, options.effort)?;
    Ok(Encoded {
        bytes: metadata::webp_assemble(&encoded, &meta, width, height),
        report: Report { setting: quality.map(u32::from), ..Report::default() },
    })
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

    fn noisy_rgb(width: u32, height: u32) -> Vec<u8> {
        // Deterministic noise, so the q95 input is big enough to shrink.
        let mut state = 0x9E3779B9u32;
        (0..width as usize * height as usize * 3)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 8) as u8
            })
            .collect()
    }

    fn lossy_input(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut config = webp::WebPConfig::new().unwrap();
        config.quality = 95.0;
        config.method = 4;
        webp::Encoder::from_rgb(pixels, width, height).encode_advanced(&config).unwrap().to_vec()
    }

    #[test]
    fn lossy_input_comes_out_smaller_at_q71() {
        let input = lossy_input(&noisy_rgb(128, 96), 128, 96);
        let out = compress(&input, &Options::default()).unwrap();
        assert!(out.bytes.len() < input.len(), "{} vs {}", out.bytes.len(), input.len());
        assert!(!metadata::webp_is_lossless(&out.bytes));
        assert_eq!(format::dimensions(&out.bytes, Format::WebP), Some((128, 96)));
        assert_eq!(out.report.setting, Some(71));
    }

    #[test]
    fn lossless_input_stays_lossless_with_identical_pixels() {
        // Transparent pixels with nonzero RGB: only `exact = 1` keeps them.
        let mut rgba = Vec::with_capacity(48 * 32 * 4);
        for y in 0..32u32 {
            for x in 0..48u32 {
                let a = if (x + y) % 3 == 0 { 0 } else { 255 };
                rgba.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, ((x * y) % 256) as u8, a]);
            }
        }
        let input = {
            // `exact = 1`, so the input itself keeps nonzero RGB under transparent pixels.
            let mut config = webp::WebPConfig::new().unwrap();
            config.lossless = 1;
            config.exact = 1;
            webp::Encoder::from_rgba(&rgba, 48, 32).encode_advanced(&config).unwrap().to_vec()
        };
        assert!(metadata::webp_is_lossless(&input));
        for effort in [Effort::Fast, Effort::Default, Effort::Max] {
            let out = compress(&input, &Options { effort, ..Options::default() }).unwrap();
            assert!(metadata::webp_is_lossless(&out.bytes), "{effort:?}");
            assert_eq!(decode_rgba(&out.bytes, 100_000).unwrap().0, rgba, "{effort:?}");
            assert_eq!(out.report.setting, None);
        }
    }

    #[test]
    fn fixed_quality_keeps_a_lossless_input_lossless() {
        let input = webp::Encoder::from_rgb(&noisy_rgb(64, 48), 64, 48).encode_lossless().to_vec();
        let options = Options { quality: Quality::Fixed(50), ..Options::default() };
        let out = compress(&input, &options).unwrap();
        assert!(metadata::webp_is_lossless(&out.bytes));
    }

    #[test]
    fn lossless_quality_on_lossy_input_keeps_the_bitstream() {
        let input = lossy_input(&noisy_rgb(64, 48), 64, 48);
        let options = Options { quality: Quality::Lossless, ..Options::default() };
        let out = compress(&input, &options).unwrap();
        let payload = |bytes: &[u8]| {
            metadata::webp_chunks(bytes).into_iter().find(|c| &c.kind == b"VP8 ").map(|c| c.payload.to_vec())
        };
        assert_eq!(payload(&out.bytes), payload(&input));
        assert_eq!(out.report, Report::default());
    }

    #[test]
    fn lossless_quality_on_a_truncated_input_is_a_decode_error() {
        let input = lossy_input(&noisy_rgb(64, 48), 64, 48);
        let options = Options { quality: Quality::Lossless, ..Options::default() };
        // Cut inside the VP8 payload: the dimensions still parse, the bitstream doesn't survive.
        assert!(matches!(compress(&input[..input.len() - 10], &options), Err(Error::Decode(_))));
        // A VP8X shell with a canvas but no image data fails the same way.
        let mut shell = b"RIFF\x12\x00\x00\x00WEBP".to_vec();
        shell.extend_from_slice(b"VP8X");
        shell.extend_from_slice(&10u32.to_le_bytes());
        shell.extend_from_slice(&[0, 0, 0, 0, 63, 0, 0, 63, 0, 0]);
        assert!(matches!(compress(&shell, &options), Err(Error::Decode(_))));
    }

    #[test]
    fn lossy_alpha_survives_with_the_alpha_flag_set() {
        let mut rgba = Vec::with_capacity(64 * 48 * 4);
        let mut state = 0x12345678u32;
        for _ in 0..64 * 48 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            rgba.extend_from_slice(&[
                (state >> 8) as u8,
                (state >> 16) as u8,
                (state >> 24) as u8,
                [0, 128, 255][(state >> 8) as usize % 3],
            ]);
        }
        let mut config = webp::WebPConfig::new().unwrap();
        config.quality = 95.0;
        config.method = 4;
        config.alpha_compression = 1;
        let input = webp::Encoder::from_rgba(&rgba, 64, 48).encode_advanced(&config).unwrap().to_vec();
        assert!(metadata::webp_chunks(&input).iter().any(|c| &c.kind == b"ALPH"));
        let out = compress(&input, &Options::default()).unwrap();
        let chunks = metadata::webp_chunks(&out.bytes);
        assert!(chunks.iter().any(|c| &c.kind == b"ALPH"), "ALPH chunk survives");
        let vp8x = chunks.iter().find(|c| &c.kind == b"VP8X").expect("extended container");
        assert_eq!(vp8x.payload[0] & 0x10, 0x10, "alpha flag is set");
    }
}
