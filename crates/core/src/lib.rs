//! squish-core: bytes in, bytes out.
//!
//! No file I/O, no network, no printing (PRD FR-5). The CLI and the HTTP server are
//! thin shells around [`compress`].

mod codec;
mod error;
mod format;
mod guard;
mod metadata;
mod metric;
mod options;
pub mod quant;
mod search;

pub use error::Error;
pub use format::{Format, Info, detect, dimensions, inspect, is_animated};
pub use options::{DEFAULT_MAX_PIXELS, Effort, MetaPolicy, Options, ParseError, Quality};
pub use quant::{Quantized, Quantizer};

/// What the compressor measured and chose for one file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    /// JPEG quality, or the number of palette colors. `None` for lossless runs.
    pub setting: Option<u32>,
    /// fast-ssim2 score of the output at about 1 MP (ADR-0026). `None` when nothing was scored.
    pub score: Option<f64>,
    /// The target the search aimed for. A score below it means the best setting missed.
    pub target: Option<f64>,
    /// Encode-and-score rounds the search ran.
    pub probes: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub bytes: Vec<u8>,
    pub format: Format,
    pub original_size: usize,
    /// True when the size guard returned the input unchanged (ADR-0011).
    pub kept_original: bool,
    pub report: Report,
}

/// Compresses one image, keeping its format.
pub fn compress(bytes: &[u8], options: &Options) -> Result<Output, Error> {
    let info = inspect(bytes).ok_or(Error::Unsupported(None))?;
    if info.animated {
        return Err(Error::Animated(info.format));
    }
    if !matches!(info.format, Format::Png | Format::Jpeg) {
        return Err(Error::Unsupported(Some(info.format)));
    }
    let (width, height) =
        info.dimensions.ok_or_else(|| Error::Decode("could not read the image size from the header".into()))?;
    if u64::from(width) * u64::from(height) > options.max_pixels {
        return Err(Error::TooManyPixels { width, height, limit: options.max_pixels });
    }

    let encoded = codec::compress(info.format, bytes, options)?;
    let (out, kept_original) = guard::apply(bytes, encoded.bytes);
    Ok(Output { bytes: out, format: info.format, original_size: bytes.len(), kept_original, report: encoded.report })
}

/// Scores `candidate` against `original` with the metric the search uses (ADR-0026).
///
/// Both must be PNG or JPEG files of the same size. 100 means identical.
pub fn score(original: &[u8], candidate: &[u8]) -> Result<f64, Error> {
    let decode = |bytes: &[u8]| {
        let format = detect(bytes).ok_or(Error::Unsupported(None))?;
        codec::decode_rgba(format, bytes, u64::MAX / 8)
    };
    let (reference, width, height) = decode(original)?;
    let (pixels, w, h) = decode(candidate)?;
    if (width, height) != (w, h) {
        return Err(Error::Decode(format!("candidate is {w}x{h}, original is {width}x{height}")));
    }
    metric::Reference::new(&reference, width, height, metric::Layout::Rgba)?.score(&pixels)
}
