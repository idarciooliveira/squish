//! PNG pipeline (PRD 4.1): decode to RGBA, search the palette size, quantize, encode
//! indexed, then run oxipng on the final palette only.

use std::io::Cursor;

use oxipng::StripChunks;

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

/// What oxipng keeps from a PNG that carries `original`'s chunks under `policy`. Both
/// paths use it, so oxipng's color-type reductions see the same chunks either way.
fn strip_for(original: &[u8], policy: MetaPolicy) -> StripChunks {
    match policy {
        MetaPolicy::StripAll => StripChunks::All,
        MetaPolicy::KeepAll => StripChunks::None,
        MetaPolicy::StripKeepIcc => StripChunks::Keep(metadata::png_color_kinds(original).into_iter().collect()),
    }
}

/// Only the lossless steps: oxipng on the original, with the policy's chunks kept.
fn lossless(bytes: &[u8], options: &Options) -> Result<Encoded, Error> {
    let strip = strip_for(bytes, options.metadata);
    Ok(Encoded { bytes: optimize(bytes, options.effort, strip)?, report: Report::default() })
}

pub(crate) fn compress(bytes: &[u8], options: &Options) -> Result<Encoded, Error> {
    compress_with(bytes, options, quant::default_quantizer())
}

/// `compress` with the quantizer passed in, so tests can run without one or with one that fails.
fn compress_with(bytes: &[u8], options: &Options, quantizer: Option<&dyn Quantizer>) -> Result<Encoded, Error> {
    // An 8-bit palette can't hold PQ or HLG levels, and the cICP tag would stay on the
    // banded result, so HDR files only get the lossless steps.
    if options.quality == Quality::Lossless || metadata::png_is_hdr(bytes) {
        return lossless(bytes, options);
    }
    let Some(quantizer) = quantizer else {
        return lossless(bytes, options);
    };
    match lossy(bytes, options, quantizer) {
        // `lossy` has dropped its pixel buffers by now, so the second oxipng pass on the
        // original doesn't stack on top of them.
        Ok((encoded, true)) => match lossless(bytes, options) {
            Ok(exact) if exact.bytes.len() <= encoded.bytes.len() => {
                // Every pixel survives, which fs2 scores as 100.
                let report = Report {
                    score: Some(100.0),
                    target: encoded.report.target,
                    probes: encoded.report.probes,
                    setting: None,
                };
                Ok(Encoded { bytes: exact.bytes, report })
            }
            _ => Ok(encoded),
        },
        Ok((encoded, false)) => Ok(encoded),
        // PRD 4.1 step 5: when quantizing fails, the original goes through losslessly.
        Err(Error::Encode(_)) => lossless(bytes, options),
        Err(e) => Err(e),
    }
}

/// Size of `raw` as a PNG from the `png` crate at its middle level, a stand-in for what
/// the lossless steps could reach that doesn't depend on how the input was compressed.
/// The fast level overshoots smooth images up to 4x, which hides the case it's meant for.
fn quick_lossless_size(raw: &Raw) -> Option<usize> {
    let opaque = raw.rgba.chunks_exact(4).all(|p| p[3] == 255);
    let rgb: Vec<u8>;
    let (color, pixels) = if opaque {
        rgb = raw.rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        (::png::ColorType::Rgb, &rgb[..])
    } else {
        (::png::ColorType::Rgba, &raw.rgba[..])
    };
    let mut out = Vec::new();
    let mut encoder = ::png::Encoder::new(&mut out, raw.width, raw.height);
    encoder.set_color(color);
    encoder.set_compression(::png::Compression::Balanced);
    encoder.set_filter(::png::Filter::Adaptive);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(pixels).ok()?;
    writer.finish().ok()?;
    Some(out.len())
}

/// The quantized file, and whether the lossless steps on the original are worth a try.
fn lossy(bytes: &[u8], options: &Options, quantizer: &dyn Quantizer) -> Result<(Encoded, bool), Error> {
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

    // Fast and fixed-quality runs never score, so they count as possible misses.
    let missed = report.score.zip(report.target).is_none_or(|(score, target)| score < target);
    let indexed = encode_indexed(raw.width, raw.height, &quantized)?;
    drop(quantized);
    // The chunks go in before oxipng, so it won't reduce to a color type they don't fit.
    let kept = metadata::png_chunks_to_keep(bytes, options.metadata);
    let indexed = metadata::png_insert_after_ihdr(indexed, &kept);
    let bytes_out = optimize(&indexed, options.effort, strip_for(bytes, options.metadata))?;

    // The palette fell short and barely shrank the file: dithering is fighting the image,
    // as on smooth gradients, and the exact pixels may come out smaller. "Barely" is
    // measured against the input and against a quick lossless encode, so a poorly
    // compressed input doesn't hide the case. On bench/ every miss lands at 20% to 49%
    // of both, where lossless never wins, and the extra oxipng pass would cost up to
    // 1.5x a plain encode. The quick encode costs at most 0.45 s there.
    let try_lossless = missed
        && (bytes_out.len() * 2 > bytes.len()
            || quick_lossless_size(&raw).is_some_and(|quick| bytes_out.len() * 2 > quick));
    Ok((Encoded { bytes: bytes_out, report }, try_lossless))
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

    struct Failing;

    impl Quantizer for Failing {
        fn quantize(&self, _: &[u8], _: u32, _: u32, _: u16, _: f32) -> Option<Quantized> {
            None
        }
    }

    #[test]
    fn without_a_working_quantizer_the_original_goes_through_losslessly() {
        let input = include_bytes!("../../tests/fixtures/photo-rgb.png");
        let options = Options::default();
        let exact = lossless(input, &options).unwrap().bytes;
        for quantizer in [None, Some(&Failing as &dyn Quantizer)] {
            let out = compress_with(input, &options, quantizer).unwrap();
            assert_eq!(out.bytes, exact, "with a quantizer: {}", quantizer.is_some());
            assert_eq!(out.report, Report::default());
        }
    }
}
