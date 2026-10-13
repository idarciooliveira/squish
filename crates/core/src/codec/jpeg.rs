//! JPEG pipeline (PRD 4.2): decode once, search mozjpeg quality per file, re-encode.
//!
//! mozjpeg reports errors by panicking, so every call into it runs under `catch_unwind`.

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};

use mozjpeg::{ColorSpace, Compress, Decompress, Marker};

use super::Encoded;
use crate::Report;
use crate::error::Error;
use crate::metadata::{self, JpegMeta};
use crate::metric::{Layout, Reference};
use crate::options::{Effort, Options, Quality};
use crate::search::{self, Params};

/// fast-ssim2 score on the ~1 MP copy that the search aims for. Calibrated against
/// what TinyPNG's own JPEGs score on the bench corpus under the same metric
/// (median 89.4): 88.7 buys NFR-1 (a) at −1.3% bytes and the (c) worst file at 1.83,
/// leaving the gates judgement in ADR-0026 with headroom on both sides.
pub(crate) const TARGET: f64 = 88.7;
const QUALITY_MIN: i32 = 30;
const QUALITY_MAX: i32 = 95;
/// `--effort fast` uses this quality and skips the search (ADR-0024).
const FAST_QUALITY: u8 = 66;

// The search has two stages. Trellis encodes cost about 15x a plain libjpeg encode and
// dominate the budget, so stage 1 finds the neighborhood with cheap `Profile::Fast`
// probes, and stage 2 spends one or two trellis probes to land and verify the result.
/// Where stage 1 starts: the median of the Phase 1 grid for the fast profile.
const FAST_START: i32 = 74;
/// fast-ssim2 points per quality step near the target, medians from the Phase 1 grid.
const FAST_SLOPE: f64 = 0.45;
const TRELLIS_SLOPE: f64 = 0.4;
/// Quality points the trellis profile needs above the fast one for the same score (median).
const TRELLIS_OFFSET: i32 = 1;

/// Encoder settings that don't change which pixels come out, except `Fast`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Profile {
    /// libjpeg defaults: baseline, no trellis. Different pixels from the others.
    Fast,
    /// Trellis quantization, progressive with the simple scan script, optimized Huffman tables.
    Trellis,
    /// `Trellis` plus mozjpeg's scan search. Same pixels, smaller file, slower.
    Max,
}

struct Raw {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    gray: bool,
}

impl Raw {
    fn layout(&self) -> Layout {
        if self.gray { Layout::Gray } else { Layout::Rgb }
    }
}

/// Runs `f`, turning a panic from the C library into an error message.
fn guarded<T>(f: impl FnOnce() -> io::Result<T>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(e.to_string()),
        Err(payload) => Err(payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "the JPEG library failed".to_string())),
    }
}

fn decode(bytes: &[u8]) -> Result<Raw, Error> {
    guarded(|| {
        let decompress = Decompress::new_mem(bytes)?;
        let (width, height) = decompress.size();
        let gray = decompress.color_space() == ColorSpace::JCS_GRAYSCALE;
        let mut started = if gray { decompress.grayscale()? } else { decompress.rgb()? };
        let pixels = started.read_scanlines::<u8>()?;
        started.finish()?;
        Ok(Raw { pixels, width: width as u32, height: height as u32, gray })
    })
    .map_err(Error::Decode)
}

fn marker(code: u8) -> Marker {
    match code {
        metadata::COM => Marker::COM,
        c => Marker::APP(c - metadata::APP0),
    }
}

fn encode(raw: &Raw, quality: u8, profile: Profile, meta: &JpegMeta) -> Result<Vec<u8>, Error> {
    guarded(|| {
        let mut compress = Compress::new(if raw.gray { ColorSpace::JCS_GRAYSCALE } else { ColorSpace::JCS_RGB });
        compress.set_size(raw.width as usize, raw.height as usize);
        // set_fastest_defaults resets the quality tables, so it goes before set_quality.
        match profile {
            Profile::Fast => compress.set_fastest_defaults(),
            Profile::Trellis => {
                compress.set_optimize_scans(false);
                compress.set_progressive_mode();
            }
            Profile::Max => {}
        }
        compress.set_quality(f32::from(quality.max(1)));
        let mut started = compress.start_compress(Vec::new())?;
        // Not `write_icc_profile`: the crate numbers the chunks from 0, the spec says 1.
        if let Some(icc) = &meta.icc {
            for payload in metadata::icc_payloads(icc) {
                started.write_marker(marker(metadata::APP2), &payload);
            }
        }
        for (code, payload) in &meta.segments {
            started.write_marker(marker(*code), payload);
        }
        started.write_scanlines(&raw.pixels)?;
        started.finish()
    })
    .map_err(Error::Encode)
}

/// Decodes to RGBA8 for scoring.
pub(crate) fn decode_rgba(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), Error> {
    let raw = decode(bytes)?;
    let rgba = if raw.gray {
        raw.pixels.iter().flat_map(|&g| [g, g, g, 255]).collect()
    } else {
        raw.pixels.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
    };
    Ok((rgba, raw.width, raw.height))
}

/// True for CMYK and YCCK files. The search decodes to RGB, and re-encoding as RGB would
/// leave a CMYK ICC profile on RGB pixels, so these only get the lossless steps.
fn is_cmyk(bytes: &[u8]) -> bool {
    guarded(|| Ok(Decompress::new_mem(bytes)?.color_space()))
        .is_ok_and(|c| matches!(c, ColorSpace::JCS_CMYK | ColorSpace::JCS_YCCK))
}

pub(crate) fn compress(bytes: &[u8], options: &Options) -> Result<Encoded, Error> {
    if options.quality == Quality::Lossless || is_cmyk(bytes) {
        let stripped = metadata::jpeg_strip(bytes, options.metadata);
        return Ok(Encoded { bytes: stripped, report: Report::default() });
    }

    let raw = decode(bytes)?;
    let meta = metadata::jpeg_meta(bytes, options.metadata);
    let profile = match options.effort {
        Effort::Fast => Profile::Fast,
        _ => Profile::Trellis,
    };

    let fixed = match (options.quality, options.effort) {
        (Quality::Fixed(q), _) => Some(q),
        (_, Effort::Fast) => Some(FAST_QUALITY),
        _ => None,
    };
    if let Some(q) = fixed {
        let bytes = encode(&raw, q, profile, &meta)?;
        return Ok(Encoded { bytes, report: Report { setting: Some(u32::from(q)), ..Report::default() } });
    }

    let reference = Reference::new(&raw.pixels, raw.width, raw.height, raw.layout())?;
    let target = options.target.unwrap_or(TARGET);
    let score_of = |encoded: &[u8]| -> Result<f64, Error> { reference.score(&decode(encoded)?.pixels) };

    // Stage 1: cheap probes, no metadata, to find where the target sits.
    let probe_meta = JpegMeta::default();
    let fast = search::find_lowest(
        &Params {
            lo: QUALITY_MIN,
            hi: QUALITY_MAX,
            start: FAST_START,
            target,
            max_probes: 3,
            slope: Some(FAST_SLOPE),
            stop_within: 1.0,
            near_miss: 0.0,
            probe_top_on_miss: true,
        },
        |q| {
            let encoded = encode(&raw, q as u8, Profile::Fast, &probe_meta)?;
            let sc = score_of(&encoded)?;
            if std::env::var_os("SQUISH_TRACE").is_some() {
                eprintln!("  fast q{q}: {sc:.2} ({} bytes)", encoded.len());
            }
            Ok((sc, ()))
        },
    )?;
    let fast_met = fast.best.score >= target;

    // Stage 2: trellis probes around the stage 1 answer. The here-and-now winner can
    // still miss the size guard (the top encode usually does), so every fitting probe
    // is kept and the strongest one is the fallback when the winner is too big.
    let landed = if fast_met { fast.best.setting } else { QUALITY_MAX - TRELLIS_OFFSET };
    let start = (landed + TRELLIS_OFFSET).clamp(QUALITY_MIN, QUALITY_MAX);
    let (max_probes, stop_within) = match (fast_met, options.effort) {
        (false, _) => (2, 0.5),
        (true, Effort::Max) => (3, 0.5),
        (true, _) => (2, 0.7),
    };
    let mut best_fit: Option<(i32, f64, Vec<u8>)> = None;
    let outcome = search::find_lowest(
        &Params {
            lo: (landed - 3).clamp(QUALITY_MIN, start),
            hi: QUALITY_MAX,
            start,
            target,
            max_probes,
            slope: Some(TRELLIS_SLOPE),
            stop_within,
            near_miss: 0.4,
            probe_top_on_miss: false,
        },
        |q| {
            let encoded = encode(&raw, q as u8, profile, &meta)?;
            let sc = score_of(&encoded)?;
            if std::env::var_os("SQUISH_TRACE").is_some() {
                eprintln!(
                    "  trellis q{q}: {sc:.2} ({} bytes){}",
                    encoded.len(),
                    if encoded.len() < bytes.len() { ", fits" } else { ", over the input" }
                );
            }
            if encoded.len() < bytes.len() && best_fit.as_ref().is_none_or(|&(_, fs, _)| sc > fs) {
                best_fit = Some((q, sc, encoded.clone()));
            }
            Ok((sc, encoded))
        },
    )?;

    // The winner only counts when it fits the guard; otherwise the strongest probe
    // that fits takes over. Without either, the guard keeps the input for us.
    let (quality, score, mut best) = if outcome.best.value.len() < bytes.len() || best_fit.is_none() {
        (outcome.best.setting, outcome.best.score, outcome.best.value)
    } else {
        let mut fit = best_fit.take().unwrap();
        (fit.0, fit.1, std::mem::take(&mut fit.2))
    };
    if options.effort == Effort::Max {
        // The scan search changes the file, not the pixels, so the score still holds.
        let tuned = encode(&raw, quality as u8, Profile::Max, &meta)?;
        if tuned.len() < best.len() {
            best = tuned;
        }
    }
    Ok(Encoded {
        bytes: best,
        report: Report {
            setting: Some(u32::from(quality as u8)),
            score: Some(score),
            target: Some(target),
            probes: (fast.probes + outcome.probes) as u32,
        },
    })
}
