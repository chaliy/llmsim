//! LLMSim CLI - LLM Traffic Simulator
//!
//! Usage:
//!   llmsim serve [OPTIONS]    Start the HTTP server
//!
//! Examples:
//!   llmsim serve --port 8080
//!   llmsim serve --config config.toml
//!   llmsim serve --generator echo --target-tokens 50
//!   llmsim serve --tui              # Start with real-time stats dashboard

use clap::{Parser, Subcommand};
use llmsim::cli::{Config, ConfigError};
#[cfg(feature = "tui")]
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
        /// Configuration file path (TOML)
        #[arg(short, long)]
        config: Option<String>,

        /// Port to listen on
        #[arg(short, long)]
        port: Option<u16>,

        /// Host to bind to
        #[arg(long, env = "LLMSIM_HOST")]
        host: Option<String>,

        /// Response generator (lorem, echo, random, fixed:text)
        ///
        /// Overrides the config file when set; otherwise the config value
        /// (or the "lorem" default) is used.
        #[arg(long)]
        generator: Option<String>,

        /// Target number of tokens in responses
        ///
        /// Overrides the config file when set; otherwise the config value
        /// (or the default of 100) is used.
        #[arg(long)]
        target_tokens: Option<usize>,

        /// Show real-time stats dashboard (TUI)
        ///
        /// Requires building with `--features tui`.
        #[arg(long)]
        tui: bool,
    },
}

fn build_config(
    config_file: Option<String>,
    port: Option<u16>,
    host: Option<String>,
    generator: Option<String>,
    target_tokens: Option<usize>,
) -> Result<Config, ConfigError> {
    let mut config = if let Some(path) = config_file {
        Config::from_file(&path)?
    } else {
        Config::default()
    };

    // Override with CLI arguments only when explicitly provided, so values from
    // the config file are respected (previously the CLI defaults silently
    // clobbered port/generator/target_tokens from --config; see the host fix
    // in #59 for the same pattern applied to --host).
    if let Some(port) = port {
        config.server.port = port;
    }
    if let Some(host) = host {
        config.server.host = host;
    }
    if let Some(generator) = generator {
        config.response.generator = generator;
    }
    if let Some(target_tokens) = target_tokens {
        config.response.target_tokens = target_tokens;
    }

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
                #[cfg(not(feature = "tui"))]
                {
                    return Err(
                        "the --tui flag requires building llmsim with --features tui".into(),
                    );
                }

                #[cfg(feature = "tui")]
                {
                    // Run server and TUI concurrently. Use the resolved
                    // config port so the dashboard targets the same port the
                    // server binds (config.toml value when --port is absent).
                    let stats = llmsim::new_shared_stats();
                    let server_url = format!("http://127.0.0.1:{}", config.server.port);

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_config(contents: &str, tag: &str) -> String {
        let path = std::env::temp_dir().join(format!(
            "llmsim_build_config_{}_{}.toml",
            std::process::id(),
            tag
        ));
        let mut f = std::fs::File::create(&path).expect("create temp config");
        f.write_all(contents.as_bytes()).expect("write temp config");
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn cli_args_none_preserve_config_file_values() {
        // Regression: the CLI defaults for --port/--generator/--target-tokens
        // used to clobber values from --config. With Option args left as None,
        // the config file must win.
        let path = write_temp_config(
            "[server]\nport = 9123\nhost = \"127.0.0.1\"\n[response]\ngenerator = \"echo\"\ntarget_tokens = 7\n",
            "preserve",
        );

        let config = build_config(Some(path.clone()), None, None, None, None).unwrap();
        assert_eq!(config.server.port, 9123);
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.response.generator, "echo");
        assert_eq!(config.response.target_tokens, 7);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cli_args_some_override_config_file_values() {
        let path = write_temp_config(
            "[server]\nport = 9123\n[response]\ngenerator = \"echo\"\ntarget_tokens = 7\n",
            "override",
        );

        let config = build_config(
            Some(path.clone()),
            Some(9555),
            None,
            Some("lorem".to_string()),
            Some(50),
        )
        .unwrap();
        assert_eq!(config.server.port, 9555);
        assert_eq!(config.response.generator, "lorem");
        assert_eq!(config.response.target_tokens, 50);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn no_config_file_uses_defaults() {
        let config = build_config(None, None, None, None, None).unwrap();
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.response.generator, "lorem");
        assert_eq!(config.response.target_tokens, 100);
    }
}
