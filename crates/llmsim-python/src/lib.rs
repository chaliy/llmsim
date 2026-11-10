//! Native half of the `llmsim` Python package.
//!
//! Decisions:
//! - Each server owns a private multi-thread Tokio runtime, so it keeps
//!   serving while Python code blocks (sync SDK clients in the same thread,
//!   pytest fixtures). Every blocking call releases the GIL.
//! - Config crosses the boundary as TOML text and/or a JSON object of
//!   overrides; merging lives in the core crate (`Config::merged_with_json`)
//!   so Python and Node share one code path.
//! - The ergonomic API (`llmsim.Server`, context managers) lives in Python
//!   (`llmsim/__init__.py`); this module stays a thin, typed shim.

use llmsim::cli::{start_server, Config, ServerHandle};
use pyo3::create_exception;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::sync::Mutex;
use std::time::Duration;
use tokio::runtime::Runtime;

create_exception!(
    _llmsim,
    LlmSimError,
    PyRuntimeError,
    "Error raised by the llmsim server."
);

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

struct Running {
    runtime: Runtime,
    handle: ServerHandle,
}

/// A running llmsim server. Created by `start()`; stop it with `stop()`.
#[pyclass(module = "llmsim._llmsim")]
struct NativeServer {
    inner: Mutex<Option<Running>>,
    url: String,
    host: String,
    port: u16,
}

#[pymethods]
impl NativeServer {
    /// Base URL, e.g. ``http://127.0.0.1:41234``.
    #[getter]
    fn url(&self) -> &str {
        &self.url
    }

    #[getter]
    fn host(&self) -> &str {
        &self.host
    }

    /// The bound port (the real one when port 0 was requested).
    #[getter]
    fn port(&self) -> u16 {
        self.port
    }

    #[getter]
    fn is_running(&self) -> bool {
        self.inner
            .lock()
            .map(|g| g.as_ref().is_some_and(|r| r.handle.is_running()))
            .unwrap_or(false)
    }

    /// Server statistics as a JSON string (same shape as ``GET /llmsim/stats``).
    fn stats_json(&self) -> PyResult<String> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| LlmSimError::new_err("server state poisoned"))?;
        let running = guard
            .as_ref()
            .ok_or_else(|| LlmSimError::new_err("server is stopped"))?;
        serde_json::to_string(&running.handle.stats())
            .map_err(|e| LlmSimError::new_err(e.to_string()))
    }

    /// Gracefully stop the server and wait for it to finish. Idempotent.
    fn stop(&self, py: Python<'_>) -> PyResult<()> {
        let running = self
            .inner
            .lock()
            .map_err(|_| LlmSimError::new_err("server state poisoned"))?
            .take();
        let Some(Running {
            runtime,
            mut handle,
        }) = running
        else {
            return Ok(());
        };
        py.detach(move || {
            let result = runtime.block_on(async {
                tokio::time::timeout(SHUTDOWN_TIMEOUT, handle.shutdown()).await
            });
            runtime.shutdown_timeout(Duration::from_secs(1));
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(LlmSimError::new_err(e.to_string())),
                Err(_) => Err(LlmSimError::new_err(
                    "timed out waiting for server shutdown",
                )),
            }
        })
    }

    fn __repr__(&self) -> String {
        format!("NativeServer(url={:?})", self.url)
    }
}

impl Drop for NativeServer {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.inner.lock() {
            if let Some(Running { runtime, handle }) = guard.take() {
                // Dropping the handle signals graceful shutdown; don't block
                // the (possibly GIL-holding) finalizer waiting for the drain.
                drop(handle);
                runtime.shutdown_background();
            }
        }
    }
}

fn build_config(toml: Option<&str>, overrides_json: Option<&str>) -> PyResult<Config> {
    let base = match toml {
        Some(text) => Config::from_toml(text).map_err(|e| PyValueError::new_err(e.to_string()))?,
        None => Config::default(),
    };
    match overrides_json {
        Some(json) => {
            let value: serde_json::Value = serde_json::from_str(json)
                .map_err(|e| PyValueError::new_err(format!("invalid config JSON: {e}")))?;
            base.merged_with_json(&value)
                .map_err(|e| PyValueError::new_err(e.to_string()))
        }
        None => Ok(base),
    }
}

/// Start a server. ``toml`` is a full config file's text; ``overrides_json``
/// is a JSON object deep-merged on top. Returns once the port is bound.
#[pyfunction]
#[pyo3(signature = (toml=None, overrides_json=None))]
fn start(
    py: Python<'_>,
    toml: Option<&str>,
    overrides_json: Option<&str>,
) -> PyResult<NativeServer> {
    let config = build_config(toml, overrides_json)?;
    let host = config.server.host.clone();
    py.detach(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("llmsim")
            .enable_all()
            .build()
            .map_err(|e| LlmSimError::new_err(format!("failed to start runtime: {e}")))?;
        let handle = runtime
            .block_on(start_server(config))
            .map_err(|e| LlmSimError::new_err(e.to_string()))?;
        let addr = handle.local_addr();
        Ok(NativeServer {
            url: format!("http://{}", addr),
            host,
            port: addr.port(),
            inner: Mutex::new(Some(Running { runtime, handle })),
        })
    })
}

/// Parse and validate a config (TOML text plus JSON overrides) without
/// starting a server. Returns the effective config as JSON.
#[pyfunction]
#[pyo3(signature = (toml=None, overrides_json=None))]
fn resolve_config(toml: Option<&str>, overrides_json: Option<&str>) -> PyResult<String> {
    let config = build_config(toml, overrides_json)?;
    serde_json::to_string(&config).map_err(|e| LlmSimError::new_err(e.to_string()))
}

#[pymodule]
fn _llmsim(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("LlmSimError", m.py().get_type::<LlmSimError>())?;
    m.add_class::<NativeServer>()?;
    m.add_function(wrap_pyfunction!(start, m)?)?;
    m.add_function(wrap_pyfunction!(resolve_config, m)?)?;
    Ok(())
}
