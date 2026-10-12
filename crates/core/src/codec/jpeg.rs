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

/// fast-ssim2 score at about 1 MP that the search aims for (ADR-0026).
pub(crate) const TARGET: f64 = 89.3;
const QUALITY_MIN: i32 = 30;
const QUALITY_MAX: i32 = 95;
const QUALITY_START: i32 = 66;
/// Typical fast-ssim2 points per quality step near the target, from the Phase 0 grid.
const SLOPE: f64 = 0.4;
/// `--effort fast` uses this quality and skips the search (ADR-0024).
const FAST_QUALITY: u8 = 66;

fn max_probes(effort: Effort) -> usize {
    match effort {
        Effort::Max => 8,
        _ => 5,
    }
}

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

pub(crate) fn compress(bytes: &[u8], options: &Options) -> Result<Encoded, Error> {
    if options.quality == Quality::Lossless {
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
    let params = Params {
        lo: QUALITY_MIN,
        hi: QUALITY_MAX,
        start: QUALITY_START,
        target: TARGET,
        max_probes: max_probes(options.effort),
        slope: Some(SLOPE),
    };
    let outcome = search::find_lowest(&params, |q| {
        let encoded = encode(&raw, q as u8, profile, &meta)?;
        let decoded = decode(&encoded)?;
        Ok((reference.score(&decoded.pixels)?, encoded))
    })?;

    let quality = outcome.best.setting as u8;
    let mut best = outcome.best.value;
    if options.effort == Effort::Max {
        // The scan search changes the file, not the pixels, so the score still holds.
        let tuned = encode(&raw, quality, Profile::Max, &meta)?;
        if tuned.len() < best.len() {
            best = tuned;
        }
    }
    Ok(Encoded {
        bytes: best,
        report: Report {
            setting: Some(u32::from(quality)),
            score: Some(outcome.best.score),
            target: Some(TARGET),
            probes: outcome.probes as u32,
        },
    })
}
