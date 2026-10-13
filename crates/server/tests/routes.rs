//! One test per route and per error status (ADR-0018), run against the router in memory.

use std::path::Path;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use squish_server::{Config, router};
use tower::ServiceExt;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures").join(name)).unwrap()
}

fn config() -> Config {
    Config { jobs: 2, ..Config::default() }
}

async fn send(app: Router, request: Request<Body>) -> (Response<Body>, Vec<u8>) {
    let response = app.oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes().to_vec();
    (Response::from_parts(parts, Body::empty()), bytes)
}

async fn get(app: Router, uri: &str) -> (Response<Body>, Vec<u8>) {
    send(app, Request::get(uri).body(Body::empty()).unwrap()).await
}

async fn post(app: Router, uri: &str, body: Vec<u8>) -> (Response<Body>, Vec<u8>) {
    send(app, Request::post(uri).header(header::CONTENT_LENGTH, body.len()).body(Body::from(body)).unwrap()).await
}

/// Checks the status and the JSON error body, and returns the message.
fn assert_error(response: &Response<Body>, body: &[u8], status: StatusCode, code: &str) -> String {
    assert_eq!(response.status(), status, "{}", String::from_utf8_lossy(body));
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    let json: serde_json::Value = serde_json::from_slice(body).unwrap();
    assert_eq!(json["error"], code);
    json["message"].as_str().unwrap().to_string()
}

fn header_str<'a>(response: &'a Response<Body>, name: &str) -> &'a str {
    response.headers()[name].to_str().unwrap()
}

#[tokio::test]
async fn health() {
    let (response, body) = get(router(config()), "/api/health").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body, b"ok");
}

#[tokio::test]
async fn info_reports_version_formats_limits_and_defaults() {
    let config = Config { max_body: 1000, max_pixels: 2000, timeout: Duration::from_millis(3000), jobs: 4, ..config() };
    let (response, body) = get(router(config), "/api/info").await;
    assert_eq!(response.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(json["formats"], serde_json::json!(["png", "jpeg"]));
    assert_eq!(
        json["limits"],
        serde_json::json!({ "max_body_bytes": 1000, "max_pixels": 2000, "timeout_ms": 3000, "jobs": 4 })
    );
    assert_eq!(
        json["defaults"],
        serde_json::json!({ "quality": "auto", "effort": "default", "metadata": "strip-keep-icc" })
    );
}

#[tokio::test]
async fn compress_returns_the_smaller_image_with_size_headers() {
    let input = fixture("photo.jpg");
    let (response, body) = post(router(config()), "/api/compress?effort=fast", input.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header_str(&response, "content-type"), "image/jpeg");
    assert_eq!(header_str(&response, "x-original-size"), input.len().to_string());
    assert_eq!(header_str(&response, "x-compressed-size"), body.len().to_string());
    assert_eq!(header_str(&response, "x-kept-original"), "false");
    assert_eq!(header_str(&response, "cache-control"), "no-store");
    assert!(header_str(&response, "server-timing").starts_with("compress;dur="));
    assert!(body.len() < input.len());
    assert!(body.starts_with(&[0xFF, 0xD8, 0xFF]));
    // No CORS: other sites can't read results from a local squish (ADR-0018).
    assert!(response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
}

#[tokio::test]
async fn compress_returns_the_original_when_it_cannot_shrink() {
    let (first, once) = post(router(config()), "/api/compress?effort=fast", fixture("photo-rgb.png")).await;
    assert_eq!(first.status(), StatusCode::OK);
    let (response, body) = post(router(config()), "/api/compress?quality=lossless&effort=fast", once.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header_str(&response, "content-type"), "image/png");
    assert_eq!(header_str(&response, "x-kept-original"), "true");
    assert_eq!(body, once);
}

#[tokio::test]
async fn empty_body_is_400() {
    let (response, body) = post(router(config()), "/api/compress", Vec::new()).await;
    assert_error(&response, &body, StatusCode::BAD_REQUEST, "empty_body");
}

#[tokio::test]
async fn bad_option_is_400_and_names_it() {
    let (response, body) = post(router(config()), "/api/compress?effort=slow", fixture("photo.jpg")).await;
    let message = assert_error(&response, &body, StatusCode::BAD_REQUEST, "invalid_option");
    assert!(message.contains("effort"), "{message}");
}

#[tokio::test]
async fn body_above_the_limit_is_413() {
    let app = router(Config { max_body: 100, ..config() });
    let (response, body) = post(app, "/api/compress", fixture("photo.jpg")).await;
    assert_error(&response, &body, StatusCode::PAYLOAD_TOO_LARGE, "body_too_large");
}

#[tokio::test]
async fn body_without_length_above_the_limit_is_413() {
    // No Content-Length header, as with a chunked upload, so the limit applies while reading.
    let app = router(Config { max_body: 1000, ..config() });
    let request = Request::post("/api/compress").body(Body::from(fixture("photo.jpg"))).unwrap();
    let (response, body) = send(app, request).await;
    assert_error(&response, &body, StatusCode::PAYLOAD_TOO_LARGE, "body_too_large");
}

#[tokio::test]
async fn too_many_pixels_is_413() {
    let app = router(Config { max_pixels: 100, ..config() });
    let (response, body) = post(app, "/api/compress", fixture("photo.jpg")).await;
    assert_error(&response, &body, StatusCode::PAYLOAD_TOO_LARGE, "too_many_pixels");
}

#[tokio::test]
async fn unknown_bytes_are_415() {
    let (response, body) = post(router(config()), "/api/compress", b"body { color: red }".to_vec()).await;
    assert_error(&response, &body, StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_format");
}

#[tokio::test]
async fn animated_png_is_415() {
    let (response, body) = post(router(config()), "/api/compress", fixture("animated.png")).await;
    assert_error(&response, &body, StatusCode::UNSUPPORTED_MEDIA_TYPE, "animated_image");
}

#[tokio::test]
async fn broken_image_is_422() {
    let mut broken = fixture("photo-rgb.png");
    broken.truncate(200);
    let (response, body) = post(router(config()), "/api/compress", broken).await;
    assert_error(&response, &body, StatusCode::UNPROCESSABLE_ENTITY, "decode_failed");
}

#[tokio::test]
async fn no_free_slot_is_429_with_retry_after() {
    let (response, body) = post(router(Config { jobs: 0, ..config() }), "/api/compress", fixture("photo.jpg")).await;
    assert_error(&response, &body, StatusCode::TOO_MANY_REQUESTS, "busy");
    assert_eq!(header_str(&response, "retry-after"), "1");
}

#[tokio::test]
async fn slow_job_is_504_and_keeps_its_slot() {
    let app = router(Config { jobs: 1, timeout: Duration::from_millis(1), ..config() });
    let (response, body) = post(app.clone(), "/api/compress?effort=max", fixture("photo-rgb.png")).await;
    assert_error(&response, &body, StatusCode::GATEWAY_TIMEOUT, "timeout");
    // The timed-out job is still running and holds the only slot.
    let (response, body) = post(app, "/api/compress", fixture("photo.jpg")).await;
    assert_error(&response, &body, StatusCode::TOO_MANY_REQUESTS, "busy");
}

#[tokio::test]
async fn index_and_assets_are_embedded() {
    let (response, body) = get(router(config()), "/").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header_str(&response, "content-type"), "text/html; charset=utf-8");
    assert_eq!(header_str(&response, "cache-control"), "no-cache");
    assert!(response.headers().contains_key(header::CONTENT_SECURITY_POLICY));
    assert!(String::from_utf8_lossy(&body).contains("<title>squish</title>"));

    for (path, kind) in [
        ("/app.js", "text/javascript; charset=utf-8"),
        ("/style.css", "text/css; charset=utf-8"),
        ("/vendor/client-zip.js", "text/javascript; charset=utf-8"),
        ("/logo.png", "image/png"),
    ] {
        let (response, _) = get(router(config()), path).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(header_str(&response, "content-type"), kind, "{path}");
    }
}

#[tokio::test]
async fn assets_revalidate_with_etag() {
    let (response, _) = get(router(config()), "/app.js").await;
    let etag = header_str(&response, "etag").to_string();
    let request = Request::get("/app.js").header(header::IF_NONE_MATCH, &etag).body(Body::empty()).unwrap();
    let (response, body) = send(router(config()), request).await;
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(header_str(&response, "etag"), etag);
    assert!(body.is_empty());
}

#[tokio::test]
async fn unknown_paths_are_404() {
    for path in ["/nope.html", "/../Cargo.toml", "/vendor/../app.js", "/shrink", "/output/abc"] {
        let (response, _) = get(router(config()), path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn wrong_method_on_an_api_route_is_405() {
    let (response, _) = get(router(config()), "/api/compress").await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}
