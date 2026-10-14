//! One module per format. Each takes the original bytes and returns the candidate output.

use crate::Report;
use crate::error::Error;
use crate::format::Format;
use crate::options::Options;

pub(crate) mod jpeg;
pub(crate) mod png;
pub(crate) mod webp;

pub(crate) struct Encoded {
    pub bytes: Vec<u8>,
    pub report: Report,
}

/// Runs the pipeline for a format. The size guard is applied by the caller.
pub(crate) fn compress(format: Format, bytes: &[u8], options: &Options) -> Result<Encoded, Error> {
    match format {
        Format::Png => png::compress(bytes, options),
        Format::Jpeg => jpeg::compress(bytes, options),
        Format::WebP | Format::Avif => Err(Error::Unsupported(Some(format))),
    }
}

/// Decodes either supported format to RGBA8, with its width and height.
pub(crate) fn decode_rgba(format: Format, bytes: &[u8], max_pixels: u64) -> Result<(Vec<u8>, u32, u32), Error> {
    match format {
        Format::Png => png::decode_rgba(bytes, max_pixels),
        Format::Jpeg => jpeg::decode_rgba(bytes),
        Format::WebP => webp::decode_rgba(bytes, max_pixels),
        Format::Avif => Err(Error::Unsupported(Some(format))),
    }
}
