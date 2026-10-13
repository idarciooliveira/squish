//! squish-server: the HTTP API and the embedded web UI behind `squish serve` (ADR-0018).
//!
//! Stateless. Each request carries one image and gets the compressed image back, and
//! nothing is written to disk (ADR-0007).

mod api;
mod assets;
mod config;
mod error;

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use tokio::sync::Semaphore;

pub use config::{Config, DEFAULT_MAX_BODY, DEFAULT_TIMEOUT};
pub use error::ApiError;

use crate::api::AppState;

/// Every route from ADR-0018. `/shrink` and `/output/` stay free for ADR-0021.
pub fn router(config: Config) -> Router {
    let state = Arc::new(AppState { slots: Arc::new(Semaphore::new(config.jobs)), config });
    Router::new()
        .route("/api/compress", post(api::compress))
        .route("/api/info", get(api::info))
        .route("/api/health", get(api::health))
        .fallback(assets::serve)
        .with_state(state)
}

/// Binds, calls `ready` with the real address (the port may have been 0), and serves until
/// Ctrl-C. Running requests finish before it returns.
pub fn run(config: Config, ready: impl FnOnce(SocketAddr)) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind((config.host.as_str(), config.port)).await?;
        ready(listener.local_addr()?);
        axum::serve(listener, router(config))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
    })
}
