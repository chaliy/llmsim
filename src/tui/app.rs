//! TUI application logic: state, stats fetching, and the tuika host loop.
//!
//! Design note: the dashboard is driven by `tuika`'s synchronous [`Runner`],
//! which owns the alternate screen, translates crossterm input, and schedules
//! redraws. Because the runner blocks its thread on terminal I/O, it runs on a
//! `spawn_blocking` thread while the stats poller stays on the async runtime;
//! the two communicate through a [`Live`] value (redraw-on-write) and a `Notify`
//! for the manual `r` refresh. This keeps `run_dashboard` an `async fn` so the
//! caller can still race it against the server with `tokio::select!`.

use super::ui;
use crate::stats::StatsSnapshot;
use ratatui::style::Color;
use std::io;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tuika::{element, Event, KeyCode, Live, LiveView, Runner, RunnerConfig, Theme};

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

/// Live dashboard state, shared between the stats poller and the renderer.
///
/// The renderer reads it each frame ([`ui::dashboard`]); the poller mutates it
/// through [`Live::update`], which requests a redraw from the runner.
pub struct DashboardData {
    /// Current stats snapshot
    pub stats: Option<StatsSnapshot>,
    /// Last error message
    pub error: Option<String>,
    /// Historical RPS values for the sparkline (last 60 values)
    pub rps_history: Vec<f64>,
    /// Historical token-rate values for the sparkline
    pub tokens_history: Vec<f64>,
    /// Last fetch time, used to derive the token rate
    last_fetch: Instant,
    /// Total tokens from the last snapshot (for rate calculation)
    last_total_tokens: u64,
}

impl DashboardData {
    fn new() -> Self {
        Self {
            stats: None,
            error: None,
            rps_history: Vec::with_capacity(60),
            tokens_history: Vec::with_capacity(60),
            last_fetch: Instant::now(),
            last_total_tokens: 0,
        }
    }

    /// Fold a freshly fetched snapshot into the rolling history and clear any
    /// previous error.
    fn ingest(&mut self, snapshot: StatsSnapshot) {
        // Token rate over the interval since the previous successful fetch.
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

        self.rps_history.push(snapshot.requests_per_second);
        if self.rps_history.len() > 60 {
            self.rps_history.remove(0);
        }

        self.stats = Some(snapshot);
        self.error = None;
        self.last_fetch = Instant::now();
    }

    /// Record a failed fetch; the header flips to DISCONNECTED.
    fn set_error(&mut self, error: String) {
        self.error = Some(error);
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

        if contains_invalid_request_chars(authority) {
            return Err("TUI server URL contains invalid host characters".to_string());
        }

        if path_prefix.contains('?') || path_prefix.contains('#') {
            return Err("TUI server URL must not include query or fragment components".to_string());
        }

        if contains_invalid_request_chars(path_prefix) {
            return Err("TUI server URL contains invalid path characters".to_string());
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

fn contains_invalid_request_chars(value: &str) -> bool {
    value
        .chars()
        .any(|c| c.is_ascii_control() || c == ' ' || !c.is_ascii())
}

/// Run the TUI dashboard until the user quits with `q`/`Esc`.
pub async fn run_dashboard(config: DashboardConfig) -> io::Result<()> {
    let refresh = Duration::from_millis(config.refresh_ms.max(1));

    // The runner owns redraw scheduling; hand its redraw handle to the Live
    // value so poller writes wake the render loop.
    let runner = Runner::new(RunnerConfig { tick_rate: refresh });
    let data = Live::with_redraw(DashboardData::new(), runner.redraw_handle());

    // `notify` lets the `r` key trigger an out-of-band refresh; `stop` unwinds
    // the poller once the runner exits.
    let notify = Arc::new(Notify::new());
    let stop = Arc::new(AtomicBool::new(false));

    // Producer: poll the stats endpoint on the async runtime.
    let poller = {
        let data = data.clone();
        let notify = Arc::clone(&notify);
        let stop = Arc::clone(&stop);
        let url = config.server_url.clone();
        tokio::spawn(async move {
            loop {
                match fetch_stats(&url).await {
                    Ok(snapshot) => data.update(|d| d.ingest(snapshot)),
                    Err(error) => data.update(|d| d.set_error(error)),
                }
                if stop.load(Ordering::Acquire) {
                    break;
                }
                // Wake on the next tick or on a manual refresh, whichever first.
                tokio::select! {
                    _ = tokio::time::sleep(refresh) => {}
                    _ = notify.notified() => {}
                }
                if stop.load(Ordering::Acquire) {
                    break;
                }
            }
        })
    };

    // Consumer: the synchronous tuika runner drives the terminal. It blocks its
    // thread on input, so it belongs on a blocking worker rather than the async
    // reactor.
    let render_data = data.clone();
    let render_notify = Arc::clone(&notify);
    let run_result = tokio::task::spawn_blocking(move || {
        // Match the previous look: keep the terminal's own background instead of
        // tuika's themed fill, so only the widgets paint color.
        let theme = Theme {
            background: Color::Reset,
            ..Theme::default()
        };
        runner.run(
            &theme,
            move |_frame| element(LiveView::new(render_data.clone(), ui::dashboard)),
            move |event| match event {
                Event::Key(key)
                    if key.plain() && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) =>
                {
                    ControlFlow::Break(())
                }
                Event::Key(key) if key.plain() && matches!(key.code, KeyCode::Char('r')) => {
                    // Force an immediate refresh out of the poll cadence.
                    render_notify.notify_one();
                    ControlFlow::Continue(())
                }
                _ => ControlFlow::Continue(()),
            },
        )
    })
    .await;

    // Tear down the poller once the UI has exited.
    stop.store(true, Ordering::Release);
    notify.notify_one();
    poller.abort();
    let _ = poller.await;

    match run_result {
        Ok(result) => result,
        Err(join_error) => Err(io::Error::other(join_error)),
    }
}

#[cfg(test)]
mod tests {
    use super::StatsEndpoint;

    #[test]
    fn parse_rejects_crlf_in_host() {
        let err = match StatsEndpoint::parse("http://localhost\r\nX-Test: 1") {
            Ok(_) => panic!("host containing CRLF should be rejected"),
            Err(err) => err,
        };
        assert!(err.contains("invalid host characters"));
    }

    #[test]
    fn parse_rejects_crlf_in_path_prefix() {
        let err = match StatsEndpoint::parse("http://localhost/base\r\nX-Test: 1") {
            Ok(_) => panic!("path containing CRLF should be rejected"),
            Err(err) => err,
        };
        assert!(err.contains("invalid path characters"));
    }
}
