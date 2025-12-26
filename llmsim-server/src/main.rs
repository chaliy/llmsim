// LLMSim Server - LLM Traffic Simulator
// A lightweight, high-performance LLM API simulator.

mod config;
mod handlers;
mod state;

use axum::{
    routing::{get, post},
    Router,
};
use clap::Parser;
use config::Config;
use state::AppState;
use std::{net::SocketAddr, sync::Arc};
use tokio::signal;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// LLMSim Server - LLM Traffic Simulator
#[derive(Parser, Debug)]
#[command(name = "llmsim-server")]
#[command(about = "A lightweight, high-performance LLM API simulator")]
#[command(version)]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "8080", env = "LLMSIM_PORT")]
    port: u16,

    /// Host to bind to
    #[arg(long, default_value = "0.0.0.0", env = "LLMSIM_HOST")]
    host: String,

    /// Path to configuration file (YAML)
    #[arg(short, long, env = "LLMSIM_CONFIG")]
    config: Option<String>,

    /// Latency profile (gpt4, gpt35, claude-opus, claude-sonnet, instant, fast)
    #[arg(long, env = "LLMSIM_LATENCY_PROFILE")]
    latency_profile: Option<String>,

    /// Response generator (lorem, echo, random, sequence)
    #[arg(long, env = "LLMSIM_GENERATOR")]
    generator: Option<String>,

    /// Target number of tokens in responses
    #[arg(long, env = "LLMSIM_TARGET_TOKENS")]
    target_tokens: Option<usize>,

    /// Enable JSON logging
    #[arg(long, env = "LLMSIM_JSON_LOGS")]
    json_logs: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Initialize tracing
    if args.json_logs {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "llmsim_server=info,tower_http=info".into()),
            )
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "llmsim_server=info,tower_http=info".into()),
            )
            .with(tracing_subscriber::fmt::layer())
            .init();
    }

    // Load configuration
    let mut config = if let Some(config_path) = &args.config {
        tracing::info!("Loading configuration from {}", config_path);
        Config::from_file(config_path)?
    } else {
        Config::default()
    };

    // Override with CLI arguments
    config.server.port = args.port;
    config.server.host = args.host.clone();

    if let Some(profile) = args.latency_profile {
        config.latency.profile = Some(profile);
    }

    if let Some(generator) = args.generator {
        config.response.generator = generator;
    }

    if let Some(target_tokens) = args.target_tokens {
        config.response.target_tokens = target_tokens;
    }

    let state = Arc::new(AppState::new(config.clone()));

    // Build router
    let app = Router::new()
        .route("/health", get(handlers::health))
        .route("/v1/chat/completions", post(handlers::chat_completions))
        .route("/v1/models", get(handlers::list_models))
        .route("/v1/models/:model_id", get(handlers::get_model))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

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
