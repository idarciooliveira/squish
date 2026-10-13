//! `/api/compress`, `/api/info` and `/api/health` (ADR-0018).

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use squish_core::{Effort, MetaPolicy, Options, Quality, SUPPORTED_FORMATS};
use tokio::sync::Semaphore;

use crate::config::Config;
use crate::error::ApiError;

pub(crate) struct AppState {
    pub config: Config,
    /// One permit per job slot. Handlers only `try_acquire`, so a full server answers `429`
    /// at once instead of queueing.
    pub slots: Arc<Semaphore>,
    /// Bodies being read at once. Each one may buffer up to `max_body`, so this bounds the
    /// memory that slow or unfinished uploads can hold.
    pub uploads: Semaphore,
}

/// Upload slots per job slot. The page sends 4 files at once, so a few more uploads than
/// jobs keeps the next file ready while the current ones compress.
pub(crate) const UPLOADS_PER_JOB: usize = 2;

pub(crate) async fn health() -> &'static str {
    "ok"
}

pub(crate) async fn info(State(state): State<Arc<AppState>>) -> Response {
    let config = &state.config;
    let defaults = Options::default();
    let formats: Vec<&str> = SUPPORTED_FORMATS.iter().map(|f| f.name()).collect();
    let body = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "formats": formats,
        "limits": {
            "max_body_bytes": config.max_body,
            "max_pixels": config.max_pixels,
            "timeout_ms": config.timeout.as_millis() as u64,
            "jobs": config.jobs,
        },
        "defaults": {
            "quality": defaults.quality.to_string(),
            "effort": defaults.effort.to_string(),
            "metadata": defaults.metadata.to_string(),
        },
    });
    ([(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

pub(crate) async fn compress(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let config = &state.config;
    let options = parse_options(&params, config.max_pixels)?;

    if let Some(length) = content_length(&headers)
        && length > config.max_body as u64
    {
        return Err(too_large(config.max_body));
    }
    // Saves uploading a body that would only get `429` once read.
    if state.slots.available_permits() == 0 {
        return Err(ApiError::busy());
    }
    let bytes = {
        let _upload = state.uploads.try_acquire().map_err(|_| ApiError::busy())?;
        // The compression timeout only starts once the body is in, so the upload gets its own.
        match tokio::time::timeout(config.timeout, read_body(body, config.max_body, content_length(&headers))).await {
            Ok(read) => read?,
            Err(_) => {
                let secs = config.timeout.as_secs_f64();
                return Err(ApiError::new(
                    StatusCode::REQUEST_TIMEOUT,
                    "upload_timeout",
                    format!("the upload took longer than {secs} s"),
                ));
            }
        }
    };
    if bytes.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "empty_body", "the request body is empty"));
    }
    // Header-only checks, so an unsupported or oversized file never takes a slot.
    squish_core::preflight(&bytes, &options)?;

    let permit = Arc::clone(&state.slots).try_acquire_owned().map_err(|_| ApiError::busy())?;
    // The permit moves into the blocking task and is released only when the work ends,
    // even after a `504`, so timeouts can't start more jobs than there are slots.
    let job = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let started = Instant::now();
        let result = catch_unwind(AssertUnwindSafe(|| squish_core::compress(&bytes, &options)));
        (result, started.elapsed())
    });
    let (result, elapsed) = match tokio::time::timeout(config.timeout, job).await {
        Err(_) => {
            let secs = config.timeout.as_secs_f64();
            return Err(ApiError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "timeout",
                format!("compression took longer than {secs} s"),
            ));
        }
        Ok(Err(join)) => return Err(crashed(&join.to_string())),
        Ok(Ok(done)) => done,
    };
    let output = match result {
        Ok(compressed) => compressed?,
        Err(panic) => {
            let message = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            return Err(crashed(&message));
        }
    };

    let compressed_size = output.bytes.len();
    let mut response = Body::from(output.bytes).into_response();
    let h = response.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(output.format.mime_type()));
    h.insert("x-original-size", HeaderValue::from(output.original_size));
    h.insert("x-compressed-size", HeaderValue::from(compressed_size));
    h.insert("x-kept-original", HeaderValue::from_static(if output.kept_original { "true" } else { "false" }));
    let timing = format!("compress;dur={:.1}", elapsed.as_secs_f64() * 1000.0);
    h.insert("server-timing", HeaderValue::from_str(&timing).expect("ascii"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// Options from the query, with the grammar from ADR-0017. Unknown parameters are ignored.
fn parse_options(params: &HashMap<String, String>, max_pixels: u64) -> Result<Options, ApiError> {
    fn get<T: std::str::FromStr<Err = squish_core::ParseError>>(
        params: &HashMap<String, String>,
        name: &str,
    ) -> Result<Option<T>, ApiError> {
        params
            .get(name)
            .map(|v| v.parse::<T>())
            .transpose()
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_option", e.to_string()))
    }
    let defaults = Options::default();
    Ok(Options {
        quality: get::<Quality>(params, "quality")?.unwrap_or(defaults.quality),
        effort: get::<Effort>(params, "effort")?.unwrap_or(defaults.effort),
        metadata: get::<MetaPolicy>(params, "metadata")?.unwrap_or(defaults.metadata),
        max_pixels,
        ..defaults
    })
}

fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers.get(header::CONTENT_LENGTH)?.to_str().ok()?.parse().ok()
}

fn too_large(limit: usize) -> ApiError {
    let mb = limit as f64 / (1024.0 * 1024.0);
    ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large", format!("the body is above the {mb} MB limit"))
}

fn crashed(message: &str) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "encode_failed", format!("squish crashed on this file: {message}"))
}

/// Reads the whole body, stopping as soon as it passes `limit`. That covers chunked bodies,
/// which have no `Content-Length` to check up front.
async fn read_body(mut body: Body, limit: usize, length: Option<u64>) -> Result<Vec<u8>, ApiError> {
    let mut bytes = Vec::with_capacity(length.map_or(0, |n| n.min(limit as u64) as usize));
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| {
            ApiError::new(StatusCode::BAD_REQUEST, "read_failed", format!("could not read the body: {e}"))
        })?;
        if let Ok(data) = frame.into_data() {
            if bytes.len() + data.len() > limit {
                return Err(too_large(limit));
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn options_use_the_core_grammar() {
        let o = parse_options(&params(&[("quality", "80"), ("effort", "fast"), ("metadata", "keep-all")]), 7).unwrap();
        assert_eq!(
            (o.quality, o.effort, o.metadata, o.max_pixels),
            (Quality::Fixed(80), Effort::Fast, MetaPolicy::KeepAll, 7)
        );
        assert_eq!(parse_options(&params(&[("cache", "1")]), 7).unwrap().quality, Quality::Auto);
        let e = parse_options(&params(&[("quality", "loud")]), 7).unwrap_err();
        assert_eq!((e.status, e.code), (StatusCode::BAD_REQUEST, "invalid_option"));
        assert!(e.message.contains("quality"), "{}", e.message);
    }
}
