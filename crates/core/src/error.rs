//! The one error type shared by the CLI and the HTTP API (ADR-0017).

use std::fmt;

use crate::format::Format;

/// Everything `compress` can fail with. `code()` is part of the public contract.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Unknown bytes (`None`), or a format that isn't compiled in or built yet.
    Unsupported(Option<Format>),
    /// APNG or animated WebP. These pass through untouched, so they are reported, not failed.
    Animated(Format),
    /// The header says the image is above `Options::max_pixels`.
    TooManyPixels {
        width: u32,
        height: u32,
        limit: u64,
    },
    Decode(String),
    Encode(String),
}

impl Error {
    /// Stable, snake_case name used in the CLI JSON report and API error bodies.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Unsupported(_) => "unsupported_format",
            Error::Animated(_) => "animated_image",
            Error::TooManyPixels { .. } => "too_many_pixels",
            Error::Decode(_) => "decode_failed",
            Error::Encode(_) => "encode_failed",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported(None) => f.write_str("not a supported image format"),
            Error::Unsupported(Some(format)) => write!(f, "{format} is not supported yet"),
            Error::Animated(format) => write!(f, "animated {format} is not supported"),
            Error::TooManyPixels { width, height, limit } => {
                write!(f, "{width}x{height} is above the limit of {limit} pixels")
            }
            Error::Decode(message) => write!(f, "could not decode the image: {message}"),
            Error::Encode(message) => write!(f, "could not encode the image: {message}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        let cases = [
            (Error::Unsupported(None), "unsupported_format"),
            (Error::Unsupported(Some(Format::Avif)), "unsupported_format"),
            (Error::Animated(Format::Png), "animated_image"),
            (Error::TooManyPixels { width: 1, height: 1, limit: 0 }, "too_many_pixels"),
            (Error::Decode(String::new()), "decode_failed"),
            (Error::Encode(String::new()), "encode_failed"),
        ];
        for (error, code) in cases {
            assert_eq!(error.code(), code);
        }
    }

    #[test]
    fn messages_name_the_format() {
        assert_eq!(Error::Unsupported(Some(Format::Avif)).to_string(), "AVIF is not supported yet");
        assert_eq!(Error::Animated(Format::WebP).to_string(), "animated WebP is not supported");
    }
}
