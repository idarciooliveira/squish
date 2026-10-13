//! Bind address and limits for `squish serve` (ADR-0014, ADR-0018).

use std::time::Duration;

use squish_core::DEFAULT_MAX_PIXELS;

/// Body limit when the caller doesn't set one: 25 MB (PRD FR-15).
pub const DEFAULT_MAX_BODY: usize = 25 * 1024 * 1024;
/// Per-request timeout when the caller doesn't set one (PRD FR-15).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Localhost by default. Exposing the server takes an explicit `0.0.0.0` (ADR-0014).
    pub host: String,
    /// 0 lets the OS pick a free port.
    pub port: u16,
    /// Compressions allowed at once. Requests above it get `429`. With 0 every compress
    /// request gets `429`, which the route tests use.
    pub jobs: usize,
    /// Largest accepted body, in bytes.
    pub max_body: usize,
    /// Largest accepted image, from the header dimensions.
    pub max_pixels: u64,
    /// How long one compression may run before the request gets `504`.
    pub timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            host: "127.0.0.1".into(),
            port: 8080,
            jobs: std::thread::available_parallelism().map_or(1, |n| n.get()),
            max_body: DEFAULT_MAX_BODY,
            max_pixels: DEFAULT_MAX_PIXELS,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}
