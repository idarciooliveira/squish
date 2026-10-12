//! One module per format. Each takes the original bytes and returns the candidate output.

use crate::Report;
use crate::error::Error;
use crate::format::Format;
use crate::options::Options;

pub(crate) mod jpeg;
pub(crate) mod png;

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
