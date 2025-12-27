//! LLMSim CLI - LLM Traffic Simulator
//!
//! Usage:
//!   llmsim serve [OPTIONS]    Start the HTTP server
//!
//! Examples:
//!   llmsim serve --port 8080
//!   llmsim serve --config config.yaml
//!   llmsim serve --generator echo --target-tokens 50
//!   llmsim serve --tui              # Start with real-time stats dashboard

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

        /// Show real-time stats dashboard (TUI)
        #[arg(long)]
        tui: bool,
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
            tui,
        } => {
            let config = build_config(config, port, host.clone(), generator, target_tokens)?;

            if tui {
                // Run server and TUI concurrently
                let stats = llmsim::new_shared_stats();
                let server_url = format!("http://127.0.0.1:{}", port);

                let dashboard_config = DashboardConfig {
                    server_url,
                    refresh_ms: 200,
                };

                // Run both concurrently - TUI exit will shut down the app
                tokio::select! {
                    result = llmsim::cli::run_server_with_stats(config, stats) => {
                        result?;
                    }
                    result = run_dashboard(dashboard_config) => {
                        result?;
                    }
                }
            } else {
                // Initialize tracing for server-only mode
                tracing_subscriber::fmt()
                    .with_env_filter(
                        tracing_subscriber::EnvFilter::from_default_env()
                            .add_directive("llmsim=info".parse().unwrap())
                            .add_directive("tower_http=debug".parse().unwrap()),
                    )
                    .init();

                llmsim::cli::run_server(config).await?;
            }
        }
    }

    Ok(())
}
