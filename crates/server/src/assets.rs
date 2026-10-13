//! The web UI from `web/`, embedded at build time (ADR-0013).

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/"]
struct Web;

/// The page only loads its own files. Thumbnails and downloads are `blob:` URLs.
const CSP: &str = "default-src 'self'; img-src 'self' blob: data:; object-src 'none'; base-uri 'none'; \
                   form-action 'none'; frame-ancestors 'none'";

/// `GET /` and `GET /{file}`. Anything that isn't an embedded file is `404`.
pub(crate) async fn serve(method: Method, uri: Uri, headers: HeaderMap) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = match uri.path().trim_start_matches('/') {
        "" => "index.html",
        p => p,
    };
    // rust-embed reads from disk in debug builds. Never let a path climb out of `web/`.
    if path.split('/').any(|s| s == ".." || s.is_empty()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(file) = Web::get(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let etag = format!("\"{}\"", hex(&file.metadata.sha256_hash()));
    let fresh = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"));
    let mut response =
        if fresh { StatusCode::NOT_MODIFIED.into_response() } else { Body::from(file.data).into_response() };
    let h = response.headers_mut();
    h.insert(header::ETAG, HeaderValue::from_str(&etag).expect("hex"));
    // Revalidate every time, so a new binary never serves stale JS (ADR-0018).
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    if !fresh {
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type(path)));
    }
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    if path.ends_with(".html") {
        h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    }
    response
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
