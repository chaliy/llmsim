//! TUI Application logic and event handling.

use super::ui;
use crate::stats::StatsSnapshot;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Configuration for the dashboard
#[derive(Debug, Clone)]
pub struct DashboardConfig {
    /// Server URL to fetch stats from
    pub server_url: String,
    /// Refresh interval in milliseconds
    pub refresh_ms: u64,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            server_url: "http://127.0.0.1:8080".to_string(),
            refresh_ms: 200,
        }
    }
}

/// Application state for the dashboard
pub struct App {
    /// Current stats snapshot
    pub stats: Option<StatsSnapshot>,
    /// Last error message
    pub error: Option<String>,
    /// Historical RPS values for sparkline (last 60 values)
    pub rps_history: Vec<f64>,
    /// Historical token rate values for sparkline
    pub tokens_history: Vec<f64>,
    /// Last fetch time
    pub last_fetch: Instant,
    /// Whether to exit
    pub should_quit: bool,
    /// Server URL
    pub server_url: String,
    /// Total tokens from last snapshot (for rate calculation)
    pub last_total_tokens: u64,
}

impl App {
    pub fn new(server_url: String) -> Self {
        Self {
            stats: None,
            error: None,
            rps_history: Vec::with_capacity(60),
            tokens_history: Vec::with_capacity(60),
            last_fetch: Instant::now(),
            should_quit: false,
            server_url,
            last_total_tokens: 0,
        }
    }

    /// Update the stats by fetching from the server
    pub async fn update_stats(&mut self) {
        match fetch_stats(&self.server_url).await {
            Ok(snapshot) => {
                // Calculate token rate
                let elapsed = self.last_fetch.elapsed().as_secs_f64();
                if elapsed > 0.0 && self.last_total_tokens > 0 {
                    let token_diff = snapshot.total_tokens.saturating_sub(self.last_total_tokens);
                    let token_rate = token_diff as f64 / elapsed;
                    self.tokens_history.push(token_rate);
                    if self.tokens_history.len() > 60 {
                        self.tokens_history.remove(0);
                    }
                }
                self.last_total_tokens = snapshot.total_tokens;

                // Update RPS history
                self.rps_history.push(snapshot.requests_per_second);
                if self.rps_history.len() > 60 {
                    self.rps_history.remove(0);
                }

                self.stats = Some(snapshot);
                self.error = None;
                self.last_fetch = Instant::now();
            }
            Err(e) => {
                self.error = Some(e);
            }
        }
    }
}

async fn fetch_stats(server_url: &str) -> Result<StatsSnapshot, String> {
    let endpoint = StatsEndpoint::parse(server_url)?;
    let mut stream = TcpStream::connect(&endpoint.connect_addr)
        .await
        .map_err(|e| format!("Failed to connect: {}", e))?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        endpoint.path, endpoint.host_header
    );

    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("Failed to request stats: {}", e))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .map_err(|e| format!("Failed to read stats: {}", e))?;

    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "Failed to parse stats response: missing headers".to_string())?;
    let headers = std::str::from_utf8(&response[..header_end])
        .map_err(|e| format!("Failed to parse stats response headers: {}", e))?;
    let status_line = headers
        .lines()
        .next()
        .ok_or_else(|| "Failed to parse stats response: missing status".to_string())?;

    if !status_line.contains(" 200 ") {
        return Err(format!("Stats endpoint returned {}", status_line));
    }

    serde_json::from_slice(&response[header_end + 4..])
        .map_err(|e| format!("Failed to parse stats: {}", e))
}

struct StatsEndpoint {
    connect_addr: String,
    host_header: String,
    path: String,
}

impl StatsEndpoint {
    fn parse(server_url: &str) -> Result<Self, String> {
        let server_url = server_url.trim().trim_end_matches('/');
        let rest = server_url
            .strip_prefix("http://")
            .ok_or_else(|| "TUI stats fetching supports http:// server URLs".to_string())?;
        let (authority, path_prefix) = rest.split_once('/').unwrap_or((rest, ""));

        if authority.is_empty() {
            return Err("TUI server URL is missing a host".to_string());
        }

        if path_prefix.contains('?') || path_prefix.contains('#') {
            return Err("TUI server URL must not include query or fragment components".to_string());
        }

        let connect_addr = if authority.starts_with('[') {
            if authority.contains("]:") {
                authority.to_string()
            } else if authority.ends_with(']') {
                format!("{}:80", authority)
            } else {
                return Err("TUI server URL has an invalid IPv6 host".to_string());
            }
        } else if authority.contains(':') {
            authority.to_string()
        } else {
            format!("{}:80", authority)
        };

        let path = if path_prefix.is_empty() {
            "/llmsim/stats".to_string()
        } else {
            format!("/{}/llmsim/stats", path_prefix.trim_end_matches('/'))
        };

        Ok(Self {
            connect_addr,
            host_header: authority.to_string(),
            path,
        })
    }
}

/// Run the TUI dashboard
pub async fn run_dashboard(config: DashboardConfig) -> io::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new(config.server_url);
    let tick_rate = Duration::from_millis(config.refresh_ms);
    let mut last_tick = Instant::now();

    // Initial fetch
    app.update_stats().await;

    loop {
        // Draw UI
        terminal.draw(|f| ui::draw(f, &app))?;

        // Handle events with timeout
        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            app.should_quit = true;
                        }
                        KeyCode::Char('r') => {
                            // Force refresh
                            app.update_stats().await;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Check if we should quit
        if app.should_quit {
            break;
        }

        // Update stats on tick
        if last_tick.elapsed() >= tick_rate {
            app.update_stats().await;
            last_tick = Instant::now();
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
