//! CLI module for LLMSim server functionality.
//!
//! This module provides the `llmsim serve` command implementation.

mod config;
mod handlers;
mod state;

pub use config::{Config, ConfigError};
pub use state::AppState;

use crate::stats::{new_shared_stats, SharedStats};
use axum::{
    routing::{get, post},
    Router,
};
use std::{net::SocketAddr, sync::Arc};
use tokio::signal;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

/// Run the LLMSim server with the given configuration
pub async fn run_server(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    run_server_with_stats(config, new_shared_stats()).await
}

/// Run the LLMSim server with the given configuration and shared stats
pub async fn run_server_with_stats(
    config: Config,
    stats: SharedStats,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .expect("Invalid address");

    tracing::info!("Starting LLMSim server on {}", addr);
    tracing::info!(
        "Configuration: latency={:?}, generator={}, target_tokens={}",
        config.latency.profile.as_deref().unwrap_or("auto"),
        config.response.generator,
        config.response.target_tokens
    );
    tracing::info!("OpenAI endpoints: /openai/v1/...");
    tracing::info!(
        "OpenResponses endpoint: /openresponses/v1/responses (https://www.openresponses.org)"
    );
    tracing::info!("Stats endpoint: /llmsim/stats");

    let state = Arc::new(AppState::new(config, stats));

    let app = Router::new()
        .route("/health", get(handlers::health))
        .route("/llmsim/stats", get(handlers::get_stats))
        // OpenAI API routes
        .route(
            "/openai/v1/chat/completions",
            post(handlers::chat_completions),
        )
        .route("/openai/v1/models", get(handlers::list_models))
        .route("/openai/v1/models/:model_id", get(handlers::get_model))
        .route("/openai/v1/responses", post(handlers::create_response))
        // OpenResponses API routes (https://www.openresponses.org)
        .route(
            "/openresponses/v1/responses",
            post(handlers::create_response),
        )
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("Server shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received");
}
