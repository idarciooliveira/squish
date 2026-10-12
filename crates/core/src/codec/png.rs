//! PNG pipeline (PRD 4.1): decode to RGBA, search the palette size, quantize, encode
//! indexed, then run oxipng on the final palette only.

use std::io::Cursor;

use oxipng::{IndexSet, StripChunks, indexset};

use super::Encoded;
use crate::Report;
use crate::error::Error;
use crate::metadata;
use crate::metric::{Layout, Reference};
use crate::options::{Effort, MetaPolicy, Options, Quality};
use crate::quant::{self, Quantized, Quantizer};
use crate::search::{self, Params};

/// fast-ssim2 score at about 1 MP that the search aims for (ADR-0026).
pub(crate) const TARGET: f64 = 90.0;
/// Floyd-Steinberg strength used for the calibration grid.
const DITHERING: f32 = 1.0;

fn max_probes(effort: Effort) -> usize {
    match effort {
        Effort::Max => 16,
        _ => 12,
    }
}

fn oxipng_preset(effort: Effort) -> u8 {
    match effort {
        Effort::Fast => 1,
        Effort::Default => 2,
        Effort::Max => 5,
    }
}

/// Palette sizes the search steps through: 2 to 256 in steps of about 8%.
fn palette_sizes() -> Vec<u16> {
    let mut sizes = vec![2u16];
    let mut size = 2.0f64;
    while size < 256.0 {
        size *= 1.08;
        let n = (size.round() as u16).min(256);
        if sizes.last() != Some(&n) {
            sizes.push(n);
        }
    }
    if sizes.last() != Some(&256) {
        sizes.push(256);
    }
    sizes
}

/// Palette size for a fixed `--quality N`: 0 is 2 colors and 100 is 256.
fn colors_for_quality(q: u8) -> u16 {
    (256 * u32::from(q.min(100)) / 100).clamp(2, 256) as u16
}

struct Raw {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

fn decode(bytes: &[u8], max_pixels: u64) -> Result<Raw, Error> {
    let mut decoder = ::png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(::png::Transformations::EXPAND | ::png::Transformations::STRIP_16);
    // 4 bytes per pixel at 8 bits, plus headroom for the decoder's own row buffers.
    decoder.set_limits(::png::Limits { bytes: (max_pixels as usize).saturating_mul(8).saturating_add(1 << 20) });
    let mut reader = decoder.read_info().map_err(|e| Error::Decode(e.to_string()))?;
    let size = reader.output_buffer_size().ok_or_else(|| Error::Decode("image is too large".into()))?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).map_err(|e| Error::Decode(e.to_string()))?;
    buf.truncate(info.buffer_size());

    let rgba = match info.color_type {
        ::png::ColorType::Rgba => buf,
        ::png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        ::png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        ::png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        ::png::ColorType::Indexed => return Err(Error::Decode("palette was not expanded".into())),
    };
    Ok(Raw { rgba, width: info.width, height: info.height })
}

fn encode_indexed(width: u32, height: u32, q: &Quantized) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let fail = |e: ::png::EncodingError| Error::Encode(e.to_string());
    {
        let mut encoder = ::png::Encoder::new(&mut out, width, height);
        encoder.set_color(::png::ColorType::Indexed);
        encoder.set_depth(::png::BitDepth::Eight);
        // oxipng recompresses, so the first pass only needs to be quick.
        encoder.set_compression(::png::Compression::Fast);
        encoder.set_palette(q.palette.iter().flat_map(|c| [c[0], c[1], c[2]]).collect::<Vec<u8>>());
        if q.palette.iter().any(|c| c[3] != 255) {
            encoder.set_trns(q.palette.iter().map(|c| c[3]).collect::<Vec<u8>>());
        }
        let mut writer = encoder.write_header().map_err(fail)?;
        writer.write_image_data(&q.indices).map_err(fail)?;
        writer.finish().map_err(fail)?;
    }
    Ok(out)
}

fn optimize(png: &[u8], effort: Effort, strip: StripChunks) -> Result<Vec<u8>, Error> {
    let mut options = oxipng::Options::from_preset(oxipng_preset(effort));
    options.strip = strip;
    oxipng::optimize_from_memory(png, &options).map_err(|e| Error::Encode(e.to_string()))
}

/// Pixels of a quantized image, for scoring.
fn expand(q: &Quantized) -> Vec<u8> {
    q.indices.iter().flat_map(|&i| q.palette[usize::from(i)]).collect()
}

/// Decodes to RGBA8 for scoring.
pub(crate) fn decode_rgba(bytes: &[u8], max_pixels: u64) -> Result<(Vec<u8>, u32, u32), Error> {
    let raw = decode(bytes, max_pixels)?;
    Ok((raw.rgba, raw.width, raw.height))
}

/// Only the lossless steps: oxipng on the original, with the policy's chunks kept.
fn lossless(bytes: &[u8], options: &Options) -> Result<Encoded, Error> {
    let strip = match options.metadata {
        MetaPolicy::StripAll => StripChunks::All,
        MetaPolicy::KeepAll => StripChunks::None,
        MetaPolicy::StripKeepIcc => {
            let keep: IndexSet<[u8; 4]> = indexset! { *b"iCCP" };
            StripChunks::Keep(keep)
        }
    };
    Ok(Encoded { bytes: optimize(bytes, options.effort, strip)?, report: Report::default() })
}

pub(crate) fn compress(bytes: &[u8], options: &Options) -> Result<Encoded, Error> {
    if options.quality == Quality::Lossless {
        return lossless(bytes, options);
    }
    let Some(quantizer) = quant::default_quantizer() else {
        return lossless(bytes, options);
    };
    match lossy(bytes, options, quantizer) {
        // PRD 4.1 step 5: when quantizing fails, the original goes through losslessly.
        Err(Error::Encode(_)) => lossless(bytes, options),
        other => other,
    }
}

fn lossy(bytes: &[u8], options: &Options, quantizer: &dyn Quantizer) -> Result<Encoded, Error> {
    let raw = decode(bytes, options.max_pixels)?;
    let quantize = |colors: u16| -> Result<Quantized, Error> {
        quantizer
            .quantize(&raw.rgba, raw.width, raw.height, colors, DITHERING)
            .ok_or_else(|| Error::Encode("the quantizer failed".into()))
    };

    let fixed = match (options.quality, options.effort) {
        (Quality::Fixed(q), _) => Some(colors_for_quality(q)),
        (_, Effort::Fast) => Some(256),
        _ => None,
    };

    let (quantized, report) = if let Some(colors) = fixed {
        let q = quantize(colors)?;
        let report = Report { setting: Some(q.palette.len() as u32), ..Report::default() };
        (q, report)
    } else {
        let sizes = palette_sizes();
        let reference = Reference::new(&raw.rgba, raw.width, raw.height, Layout::Rgba)?;
        let top = sizes.len() as i32 - 1;
        // Most photos miss the target even at 256 colors (ADR-0022), so the top goes first:
        // a miss there ends the search after one probe.
        let target = options.target.unwrap_or(TARGET);
        let params = Params {
            lo: 0,
            hi: top,
            start: top,
            target,
            max_probes: max_probes(options.effort),
            slope: None,
            stop_within: 0.0,
            near_miss: 0.0,
            probe_top_on_miss: true,
        };
        let outcome = search::find_lowest(&params, |i| {
            let q = quantize(sizes[i as usize])?;
            Ok((reference.score(&expand(&q))?, q))
        })?;
        let report = Report {
            setting: Some(outcome.best.value.palette.len() as u32),
            score: Some(outcome.best.score),
            target: Some(target),
            probes: outcome.probes as u32,
        };
        (outcome.best.value, report)
    };

    let indexed = encode_indexed(raw.width, raw.height, &quantized)?;
    let optimized = optimize(&indexed, options.effort, StripChunks::All)?;
    let kept = metadata::png_chunks_to_keep(bytes, options.metadata);
    Ok(Encoded { bytes: metadata::png_insert_after_ihdr(optimized, &kept), report })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_sizes_span_the_range_in_order() {
        let sizes = palette_sizes();
        assert_eq!((sizes[0], *sizes.last().unwrap()), (2, 256));
        assert!(sizes.windows(2).all(|w| w[0] < w[1]));
        assert!(sizes.len() < 70, "{} steps would make the search slow", sizes.len());
    }

    #[test]
    fn fixed_quality_maps_to_palette_size() {
        assert_eq!(colors_for_quality(0), 2);
        assert_eq!(colors_for_quality(50), 128);
        assert_eq!(colors_for_quality(100), 256);
    }
}
