//! CLI module for LLMSim server functionality.
//!
//! This module provides the `llmsim serve` command implementation.

mod anthropic_handlers;
mod config;
mod decisions_handlers;
mod embeddings_handlers;
mod gemini_handlers;
mod handlers;
mod jobs;
pub mod limits;
pub mod prompt_cache;
mod scenario_handlers;
mod server;
mod state;
mod typesafe_handlers;
pub mod validation;
mod ws_handler;

pub use config::{Config, ConfigError};
pub use server::{build_state, start_server, ServerError, ServerHandle};
pub use state::AppState;
pub use ws_handler::ws_responses;

use crate::stats::{new_shared_stats, SharedStats};
use axum::{
    routing::{get, post},
    Router,
};
use std::{net::SocketAddr, sync::Arc};
use tokio::signal;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

/// Build the Axum router with all endpoints.
/// Exposed for integration testing.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(handlers::health))
        .route("/llmsim/stats", get(handlers::get_stats))
        // OpenAI API routes
        .route(
            "/openai/v1/chat/completions",
            post(handlers::chat_completions),
        )
        .route("/openai/v1/models", get(handlers::list_models))
        .route("/openai/v1/models/{model_id}", get(handlers::get_model))
        .route(
            "/openai/v1/responses",
            post(handlers::create_response).get(ws_handler::ws_responses),
        )
        .route(
            "/openai/v1/responses/{response_id}",
            get(jobs::get_response).delete(jobs::delete_response),
        )
        .route(
            "/openai/v1/responses/{response_id}/cancel",
            post(jobs::cancel_response),
        )
        .route("/openai/v1/files", post(jobs::upload_file))
        .route(
            "/openai/v1/files/{file_id}",
            get(jobs::get_file).delete(jobs::delete_file),
        )
        .route(
            "/openai/v1/files/{file_id}/content",
            get(jobs::get_file_content),
        )
        .route(
            "/openai/v1/batches",
            post(jobs::create_openai_batch).get(jobs::list_openai_batches),
        )
        .route("/openai/v1/batches/{batch_id}", get(jobs::get_openai_batch))
        .route(
            "/openai/v1/batches/{batch_id}/cancel",
            post(jobs::cancel_openai_batch),
        )
        .route(
            "/openai/v1/embeddings",
            post(embeddings_handlers::create_embeddings),
        )
        .route(
            "/openai/v1/images/generations",
            post(handlers::create_image),
        )
        // OpenResponses API routes (https://www.openresponses.org)
        .route(
            "/openresponses/v1/responses",
            post(handlers::create_openresponses_response),
        )
        // Anthropic API routes
        .route(
            "/anthropic/v1/messages",
            post(anthropic_handlers::create_message),
        )
        .route(
            "/anthropic/v1/messages/count_tokens",
            post(anthropic_handlers::count_tokens),
        )
        .route(
            "/anthropic/v1/messages/batches",
            post(jobs::create_anthropic_batch).get(jobs::list_anthropic_batches),
        )
        .route(
            "/anthropic/v1/messages/batches/{batch_id}",
            get(jobs::get_anthropic_batch).delete(jobs::delete_anthropic_batch),
        )
        .route(
            "/anthropic/v1/messages/batches/{batch_id}/cancel",
            post(jobs::cancel_anthropic_batch),
        )
        .route(
            "/anthropic/v1/messages/batches/{batch_id}/results",
            get(jobs::anthropic_batch_results),
        )
        .route("/anthropic/v1/models", get(anthropic_handlers::list_models))
        .route(
            "/anthropic/v1/models/{model_id}",
            get(anthropic_handlers::get_model),
        )
        // TypeSafe API routes (System One)
        .route("/typesafe/v1/systemone", post(typesafe_handlers::systemone))
        .route("/typesafe/v1/models", get(typesafe_handlers::list_models))
        // Decisions APIs (typed judgments)
        .route(
            "/openai/v1/decisions",
            post(decisions_handlers::openai_decisions),
        )
        .route("/gemini/v1beta/models", get(gemini_handlers::list_models))
        .route(
            "/gemini/v1beta/models/{model}",
            get(gemini_handlers::get_model).post(gemini_handlers::model_action),
        )
        .route(
            "/gemini/v1beta/decisions",
            post(decisions_handlers::gemini_decisions),
        )
        .route(
            "/foundry/providers/microsoft/v1/systemone",
            post(typesafe_handlers::systemone),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            limits::middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}

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
        "WebSocket mode: ws://{}:{}/openai/v1/responses",
        config.server.host,
        config.server.port
    );
    tracing::info!(
        "OpenResponses endpoint: /openresponses/v1/responses (https://www.openresponses.org)"
    );
    tracing::info!("Anthropic endpoints: /anthropic/v1/messages, /anthropic/v1/messages/count_tokens, /anthropic/v1/models");
    tracing::info!("TypeSafe endpoints: /typesafe/v1/systemone, /typesafe/v1/models");
    tracing::info!(
        "Decisions endpoints: /openai/v1/decisions, /gemini/v1beta/decisions, /foundry/providers/microsoft/v1/systemone"
    );
    tracing::info!("Stats endpoint: /llmsim/stats");

    let state = build_state(config, stats)?;
    let app = build_router(Arc::new(state));

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
