// Embeddable server handle.
//
// Decisions:
// - `start_server` binds before returning, so callers (tests, the Python and
//   Node bindings) learn the real port when they ask for port 0 and never race
//   the listener.
// - Shutdown is explicit via `ServerHandle::shutdown` (graceful, drains open
//   connections). Dropping the handle also signals shutdown so an embedded
//   server never outlives its owner; it does not wait for the drain.
// - No OS signal handling here: that belongs to the CLI (`run_server`), not to
//   a library embedded in someone else's process.

use super::config::{Config, ConfigError};
use super::state::AppState;
use crate::scenario::ScenarioSet;
use crate::script::Script;
use crate::stats::{new_shared_stats, SharedStats, StatsSnapshot};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Errors from starting or stopping an embedded server.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("Invalid listen address {0}")]
    Address(String),
    #[error("Failed to bind {addr}: {source}")]
    Bind {
        addr: String,
        source: std::io::Error,
    },
    #[error("Server error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Server task failed: {0}")]
    Join(String),
}

/// Build the application state for `config`, loading the script and
/// scenario files it names.
pub fn build_state(config: Config, stats: SharedStats) -> Result<AppState, ConfigError> {
    let mut state = AppState::new(config, stats);
    if let Some(path) = state.config.response.script_path.clone() {
        let script = Script::from_file(&path).map_err(|e| {
            ConfigError::Validation(format!("Failed to load script from {}: {}", path, e))
        })?;
        tracing::info!(
            "Scripted mode enabled: {} turns from {} (on_exhausted={:?})",
            script.len(),
            path,
            script.on_exhausted()
        );
        state = state.with_script(Arc::new(script));
    }
    if let Some(path) = state.config.response.scenarios_path.clone() {
        let scenarios = ScenarioSet::from_path(&path).map_err(|e| {
            ConfigError::Validation(format!("Failed to load scenarios from {}: {}", path, e))
        })?;
        tracing::info!(
            "Scenarios enabled: {} from {} ({})",
            scenarios.len(),
            path,
            scenarios.names().join(", ")
        );
        state = state.with_scenarios(Arc::new(scenarios));
    }
    Ok(state)
}

/// A running, embedded LLMSim server.
pub struct ServerHandle {
    addr: SocketAddr,
    stats: SharedStats,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<std::io::Result<()>>>,
}

impl ServerHandle {
    /// The address the server is listening on (with the real port when the
    /// config asked for port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Base URL, e.g. `http://127.0.0.1:41234`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Live statistics for this server.
    pub fn stats(&self) -> StatsSnapshot {
        self.stats.snapshot()
    }

    /// Whether the server task is still running.
    pub fn is_running(&self) -> bool {
        self.task.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Stop accepting connections, drain in-flight requests, and wait for the
    /// server task to finish. Idempotent.
    pub async fn shutdown(&mut self) -> Result<(), ServerError> {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        match self.task.take() {
            Some(task) => match task.await {
                Ok(result) => result.map_err(ServerError::from),
                Err(e) => Err(ServerError::Join(e.to_string())),
            },
            None => Ok(()),
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// Bind and start an LLMSim server in the background on the current Tokio
/// runtime. Returns once the listener is bound.
pub async fn start_server(config: Config) -> Result<ServerHandle, ServerError> {
    let host = config.server.host.clone();
    let port = config.server.port;
    // `host:port` does not parse for bare IPv6 hosts; bracket them.
    let addr_str = if host.contains(':') && !host.starts_with('[') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    };
    let addr: SocketAddr = addr_str
        .parse()
        .map_err(|_| ServerError::Address(addr_str.clone()))?;

    let stats = new_shared_stats();
    let state = build_state(config, stats.clone())?;
    let app = super::build_router(Arc::new(state));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| ServerError::Bind {
            addr: addr_str,
            source,
        })?;
    let addr = listener.local_addr()?;

    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await
    });

    Ok(ServerHandle {
        addr,
        stats,
        shutdown: Some(tx),
        task: Some(task),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_config() -> Config {
        Config::default()
            .merged_with_json(&serde_json::json!({
                "server": {"host": "127.0.0.1", "port": 0},
                "latency": {"profile": "instant"}
            }))
            .unwrap()
    }

    #[tokio::test]
    async fn start_binds_ephemeral_port_and_shuts_down() {
        let mut server = start_server(local_config()).await.unwrap();
        assert_ne!(server.local_addr().port(), 0);
        assert!(server.is_running());

        let stream = tokio::net::TcpStream::connect(server.local_addr()).await;
        assert!(stream.is_ok());
        drop(stream);

        server.shutdown().await.unwrap();
        assert!(!server.is_running());
        // Second shutdown is a no-op.
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn port_in_use_is_a_bind_error() {
        let first = start_server(local_config()).await.unwrap();
        let port = first.local_addr().port();
        let config = local_config()
            .merged_with_json(&serde_json::json!({"server": {"port": port}}))
            .unwrap();
        let err = start_server(config).await.err().unwrap();
        assert!(matches!(err, ServerError::Bind { .. }), "{err}");
    }

    #[tokio::test]
    async fn invalid_host_and_missing_script_fail() {
        let bad_host = local_config()
            .merged_with_json(&serde_json::json!({"server": {"host": "not a host"}}))
            .unwrap();
        assert!(matches!(
            start_server(bad_host).await.err().unwrap(),
            ServerError::Address(_)
        ));

        let missing_script = local_config()
            .merged_with_json(
                &serde_json::json!({"response": {"script_path": "/nonexistent/llmsim.yaml"}}),
            )
            .unwrap();
        assert!(matches!(
            start_server(missing_script).await.err().unwrap(),
            ServerError::Config(_)
        ));
    }
}
