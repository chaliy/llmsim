//! LLMSim CLI - LLM Traffic Simulator
//!
//! Usage:
//!   llmsim serve [OPTIONS]    Start the HTTP server
//!
//! Examples:
//!   llmsim serve --port 8080
//!   llmsim serve --config config.yaml
//!   llmsim serve --generator echo --target-tokens 50

use clap::{Parser, Subcommand};
use llmsim::cli::{Config, ConfigError};

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
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("llmsim=info".parse().unwrap())
                .add_directive("tower_http=debug".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve {
            config,
            port,
            host,
            generator,
            target_tokens,
        } => {
            let config = build_config(config, port, host, generator, target_tokens)?;
            llmsim::cli::run_server(config).await?;
        }
    }

    Ok(())
}
