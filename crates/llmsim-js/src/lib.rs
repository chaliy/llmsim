//! Native half of the `llmsim` npm package.
//!
//! Decisions:
//! - Servers run on napi's shared Tokio runtime (`tokio_rt` feature), so they
//!   never block the Node event loop; `startServer`/`stop` return Promises.
//! - Config crosses the boundary as TOML text and/or a JSON string of
//!   overrides; merging lives in the core crate (`Config::merged_with_json`),
//!   shared with the Python binding.
//! - The ergonomic API (`Server`, `Symbol.asyncDispose`) lives in
//!   `wrapper.ts`; this module stays a thin, typed shim.

use llmsim::cli::{start_server, Config, ServerHandle};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

fn build_config(toml: Option<String>, overrides_json: Option<String>) -> Result<Config> {
    let base = match toml {
        Some(text) => {
            Config::from_toml(&text).map_err(|e| Error::new(Status::InvalidArg, e.to_string()))?
        }
        None => Config::default(),
    };
    match overrides_json {
        Some(json) => {
            let value: serde_json::Value = serde_json::from_str(&json)
                .map_err(|e| Error::new(Status::InvalidArg, format!("invalid config JSON: {e}")))?;
            base.merged_with_json(&value)
                .map_err(|e| Error::new(Status::InvalidArg, e.to_string()))
        }
        None => Ok(base),
    }
}

/// A running llmsim server. Create with `startServer()`.
#[napi]
pub struct NativeServer {
    handle: Arc<Mutex<Option<ServerHandle>>>,
    url: String,
    host: String,
    port: u32,
}

#[napi]
impl NativeServer {
    /// Base URL, e.g. `http://127.0.0.1:41234`.
    #[napi(getter)]
    pub fn url(&self) -> String {
        self.url.clone()
    }

    #[napi(getter)]
    pub fn host(&self) -> String {
        self.host.clone()
    }

    /// The bound port (the real one when port 0 was requested).
    #[napi(getter)]
    pub fn port(&self) -> u32 {
        self.port
    }

    #[napi(getter)]
    pub fn is_running(&self) -> bool {
        self.handle
            .try_lock()
            .map(|g| g.as_ref().is_some_and(|h| h.is_running()))
            // Locked means a stop() is in flight.
            .unwrap_or(false)
    }

    /// Server statistics as a JSON string (same shape as `GET /llmsim/stats`).
    #[napi]
    pub fn stats_json(&self) -> Result<String> {
        let guard = self
            .handle
            .try_lock()
            .map_err(|_| Error::from_reason("server is stopping"))?;
        let handle = guard
            .as_ref()
            .ok_or_else(|| Error::from_reason("server is stopped"))?;
        serde_json::to_string(&handle.stats()).map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Gracefully stop the server and wait for it to finish. Idempotent.
    #[napi]
    pub async fn stop(&self) -> Result<()> {
        let taken = self.handle.lock().await.take();
        let Some(mut handle) = taken else {
            return Ok(());
        };
        match tokio::time::timeout(SHUTDOWN_TIMEOUT, handle.shutdown()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(Error::from_reason(e.to_string())),
            Err(_) => Err(Error::from_reason("timed out waiting for server shutdown")),
        }
    }
}

/// Start a server. `toml` is a full config file's text; `overridesJson` is a
/// JSON object deep-merged on top. Resolves once the port is bound.
#[napi]
pub async fn start_server_native(
    toml: Option<String>,
    overrides_json: Option<String>,
) -> Result<NativeServer> {
    let config = build_config(toml, overrides_json)?;
    let host = config.server.host.clone();
    let handle = start_server(config)
        .await
        .map_err(|e| Error::from_reason(e.to_string()))?;
    let addr = handle.local_addr();
    Ok(NativeServer {
        url: format!("http://{}", addr),
        host,
        port: u32::from(addr.port()),
        handle: Arc::new(Mutex::new(Some(handle))),
    })
}

/// Parse and validate a config (TOML text plus JSON overrides) without
/// starting a server. Returns the effective config as JSON.
#[napi]
pub fn resolve_config_native(
    toml: Option<String>,
    overrides_json: Option<String>,
) -> Result<String> {
    let config = build_config(toml, overrides_json)?;
    serde_json::to_string(&config).map_err(|e| Error::from_reason(e.to_string()))
}

/// Version of the native addon (matches the npm package version).
#[napi]
pub fn native_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
