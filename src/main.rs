//! LLMSim CLI - LLM Traffic Simulator
//!
//! Usage:
//!   llmsim serve [OPTIONS]    Start the HTTP server
//!   llmsim stats [OPTIONS]    Show real-time stats dashboard
//!
//! Examples:
//!   llmsim serve --port 8080
//!   llmsim serve --config config.yaml
//!   llmsim serve --generator echo --target-tokens 50
//!   llmsim stats --url http://localhost:8080

use clap::{Parser, Subcommand};
use llmsim::cli::{Config, ConfigError};
use llmsim::tui::{run_dashboard, DashboardConfig};

#[derive(Parser)]
#[command(name = "llmsim")]
#[command(author, version, about = "LLM Traffic Simulator", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the LLMSim HTTP server
    Serve {
        /// Configuration file path (YAML)
        #[arg(short, long)]
        config: Option<String>,

        /// Port to listen on
        #[arg(short, long, default_value = "8080")]
        port: u16,

        /// Host to bind to
        #[arg(long, default_value = "0.0.0.0")]
        host: String,

        /// Response generator (lorem, echo, random, fixed:text)
        #[arg(long, default_value = "lorem")]
        generator: String,

        /// Target number of tokens in responses
        #[arg(long, default_value = "100")]
        target_tokens: usize,
    },

    /// Show real-time stats dashboard (TUI)
    Stats {
        /// LLMSim server URL
        #[arg(short, long, default_value = "http://127.0.0.1:8080")]
        url: String,

        /// Refresh interval in milliseconds
        #[arg(short, long, default_value = "200")]
        refresh: u64,
    },
}

fn build_config(
    config_file: Option<String>,
    port: u16,
    host: String,
    generator: String,
    target_tokens: usize,
) -> Result<Config, ConfigError> {
    let mut config = if let Some(path) = config_file {
        Config::from_file(&path)?
    } else {
        Config::default()
    };

    // Override with CLI arguments
    config.server.port = port;
    config.server.host = host;
    config.response.generator = generator;
    config.response.target_tokens = target_tokens;

    Ok(config)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve {
            config,
            port,
            host,
            generator,
            target_tokens,
        } => {
            // Initialize tracing for server mode
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::from_default_env()
                        .add_directive("llmsim=info".parse().unwrap())
                        .add_directive("tower_http=debug".parse().unwrap()),
                )
                .init();

            let config = build_config(config, port, host, generator, target_tokens)?;
            llmsim::cli::run_server(config).await?;
        }
        Commands::Stats { url, refresh } => {
            // Run the TUI dashboard
            let config = DashboardConfig {
                server_url: url,
                refresh_ms: refresh,
            };
            run_dashboard(config).await?;
        }
    }

    Ok(())
}
