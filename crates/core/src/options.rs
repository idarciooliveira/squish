//! Compression options and their string grammar (ADR-0017).
//!
//! The CLI flags and the HTTP query parameters both go through `FromStr` here, so a
//! value that works in one works in the other.

use std::fmt;
use std::str::FromStr;

/// Pixel limit applied when the caller doesn't set one (PRD FR-4).
pub const DEFAULT_MAX_PIXELS: u64 = 50_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    /// Per-file search against the format's quality target (ADR-0024).
    #[default]
    Auto,
    /// A fixed setting, 0 to 100, with no search.
    Fixed(u8),
    /// No pixel changes. Only the lossless optimization steps run.
    Lossless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Effort {
    /// No search. Fixed settings, cheapest encoder profile.
    Fast,
    #[default]
    Default,
    /// Same target as `Default`, with more probes and the slowest lossless passes.
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MetaPolicy {
    /// Keep the ICC profile and the orientation, drop the rest (ADR-0010).
    #[default]
    StripKeepIcc,
    StripAll,
    KeepAll,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub quality: Quality,
    pub effort: Effort,
    pub metadata: MetaPolicy,
    pub max_pixels: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            quality: Quality::default(),
            effort: Effort::default(),
            metadata: MetaPolicy::default(),
            max_pixels: DEFAULT_MAX_PIXELS,
        }
    }
}

/// A value that didn't match an option's grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    option: &'static str,
    value: String,
    expected: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {} {:?}, expected {}", self.option, self.value, self.expected)
    }
}

impl std::error::Error for ParseError {}

impl FromStr for Quality {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        if t.eq_ignore_ascii_case("auto") {
            return Ok(Quality::Auto);
        }
        if t.eq_ignore_ascii_case("lossless") {
            return Ok(Quality::Lossless);
        }
        // `u8::from_str` also accepts a leading "+", which isn't part of the grammar.
        match t.parse::<u8>() {
            Ok(n) if n <= 100 && t.bytes().all(|b| b.is_ascii_digit()) => Ok(Quality::Fixed(n)),
            _ => Err(ParseError {
                option: "quality",
                value: s.to_string(),
                expected: "auto, lossless or an integer from 0 to 100",
            }),
        }
    }
}

impl fmt::Display for Quality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Quality::Auto => f.write_str("auto"),
            Quality::Lossless => f.write_str("lossless"),
            Quality::Fixed(n) => write!(f, "{n}"),
        }
    }
}

impl FromStr for Effort {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "fast" => Ok(Effort::Fast),
            "default" => Ok(Effort::Default),
            "max" => Ok(Effort::Max),
            _ => Err(ParseError { option: "effort", value: s.to_string(), expected: "fast, default or max" }),
        }
    }
}

impl fmt::Display for Effort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Effort::Fast => "fast",
            Effort::Default => "default",
            Effort::Max => "max",
        })
    }
}

impl FromStr for MetaPolicy {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "strip-keep-icc" => Ok(MetaPolicy::StripKeepIcc),
            "strip-all" => Ok(MetaPolicy::StripAll),
            "keep-all" => Ok(MetaPolicy::KeepAll),
            _ => Err(ParseError {
                option: "metadata",
                value: s.to_string(),
                expected: "strip-keep-icc, strip-all or keep-all",
            }),
        }
    }
}

impl fmt::Display for MetaPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MetaPolicy::StripKeepIcc => "strip-keep-icc",
            MetaPolicy::StripAll => "strip-all",
            MetaPolicy::KeepAll => "keep-all",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_grammar() {
        assert_eq!("auto".parse(), Ok(Quality::Auto));
        assert_eq!("LOSSLESS".parse(), Ok(Quality::Lossless));
        assert_eq!("0".parse(), Ok(Quality::Fixed(0)));
        assert_eq!("100".parse(), Ok(Quality::Fixed(100)));
        assert_eq!(" 80 ".parse(), Ok(Quality::Fixed(80)));
        for bad in ["101", "-1", "+5", "80.5", "", "high", "256", "1e2"] {
            assert!(bad.parse::<Quality>().is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn effort_and_metadata_grammar() {
        assert_eq!("fast".parse(), Ok(Effort::Fast));
        assert_eq!("Max".parse(), Ok(Effort::Max));
        assert!("slow".parse::<Effort>().is_err());
        assert_eq!("strip-keep-icc".parse(), Ok(MetaPolicy::StripKeepIcc));
        assert_eq!("keep-all".parse(), Ok(MetaPolicy::KeepAll));
        assert!("strip".parse::<MetaPolicy>().is_err());
    }

    #[test]
    fn display_round_trips() {
        for q in [Quality::Auto, Quality::Lossless, Quality::Fixed(0), Quality::Fixed(73)] {
            assert_eq!(q.to_string().parse(), Ok(q));
        }
        for e in [Effort::Fast, Effort::Default, Effort::Max] {
            assert_eq!(e.to_string().parse(), Ok(e));
        }
        for m in [MetaPolicy::StripKeepIcc, MetaPolicy::StripAll, MetaPolicy::KeepAll] {
            assert_eq!(m.to_string().parse(), Ok(m));
        }
    }

    #[test]
    fn error_message_names_the_option() {
        let message = "loud".parse::<Quality>().unwrap_err().to_string();
        assert!(message.contains("quality") && message.contains("\"loud\""), "{message}");
    }

    #[test]
    fn defaults_follow_the_prd() {
        let o = Options::default();
        assert_eq!(o.quality, Quality::Auto);
        assert_eq!(o.effort, Effort::Default);
        assert_eq!(o.metadata, MetaPolicy::StripKeepIcc);
        assert_eq!(o.max_pixels, 50_000_000);
    }
}
