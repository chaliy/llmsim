//! TUI application logic: state, stats fetching, and the tuika host loop.
//!
//! Design note: the dashboard is driven by `tuika`'s [`AsyncRunner`], which ties
//! the alternate-screen lifecycle, crossterm's async event stream, and a tick
//! timer into one `tokio::select!` loop on the caller's runtime. The dashboard
//! state is a plain local [`DashboardData`] the loop owns: `view` reads it to
//! build each frame and `update` mutates it (and may `.await` a stats fetch) in
//! response to a [`Signal`] — a tick or a key. No `spawn_blocking`, shared
//! `RwLock`, `Notify`, or stop flag; `run_dashboard` stays an `async fn` the
//! caller races against the server with `tokio::select!`.

use super::ui;
use crate::stats::StatsSnapshot;
use std::io;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tuika::{
    async_from_fn, AsyncRunner, Event, KeyCode, RunnerConfig, Signal, Theme, UpdateResult,
};

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

/// Fetch a fresh snapshot and fold it into `data`, recording any failure as the
/// disconnected state. Shared by the periodic tick and the manual `r` refresh.
async fn poll(url: &str, data: &mut DashboardData) {
    match fetch_stats(url).await {
        Ok(snapshot) => data.ingest(snapshot),
        Err(error) => data.set_error(error),
    }
}

/// Map one runner signal onto the dashboard state.
///
/// Split out of the runner closure so the key/tick contract is testable without
/// a terminal: the runner only forwards signals here and acts on the returned
/// [`UpdateResult`].
async fn update(url: &str, data: &mut DashboardData, signal: Signal) -> UpdateResult {
    match signal {
        Signal::Tick => {
            poll(url, data).await;
            // A tick always folds a fresh sample (or an error) into the state,
            // so the frame is repainted on every tick as it was before
            // `UpdateResult` started gating redraws.
            UpdateResult::Dirty
        }
        Signal::Event(Event::Key(key)) if key.plain() => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => UpdateResult::Exit,
            KeyCode::Char('r') => {
                // Force an immediate refresh out of the tick cadence.
                poll(url, data).await;
                UpdateResult::Dirty
            }
            // Unhandled: `Clean` lets the runner apply its own default
            // interactions (such as drag-to-select) to the input.
            _ => UpdateResult::Clean,
        },
        _ => UpdateResult::Clean,
    }
}

/// Run the TUI dashboard until the user quits with `q`/`Esc`.
pub async fn run_dashboard(config: DashboardConfig) -> io::Result<()> {
    let refresh = Duration::from_millis(config.refresh_ms.max(1));
    let runner = AsyncRunner::new(RunnerConfig {
        tick_rate: refresh,
        ..RunnerConfig::default()
    });

    // Match the previous look: keep the terminal's own background instead of
    // tuika's themed fill, so only the widgets paint color.
    let theme = Theme {
        background: tuika::prelude::Color::Reset,
        ..Theme::default()
    };

    // The runner owns this state for the duration of the run. `view` reads it
    // each frame; `update` mutates it on every signal. The first tick fires
    // right after the initial paint, so the dashboard loads without a special
    // startup fetch.
    let url = config.server_url;
    let mut data = DashboardData::new();

    runner
        .run(
            &theme,
            async_from_fn(
                &mut data,
                |data, _frame| ui::dashboard(data),
                async |data, signal| update(&url, data, signal).await,
            ),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::{update, DashboardData, StatsEndpoint, StatsSnapshot};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tuika::{Event, Key, KeyCode, Signal, UpdateResult};

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

    /// A snapshot with the two fields `ingest` reads (`total_tokens`,
    /// `requests_per_second`) set and everything else zeroed.
    fn snapshot(total_tokens: u64, requests_per_second: f64) -> StatsSnapshot {
        StatsSnapshot {
            uptime_secs: 0,
            total_requests: 0,
            active_requests: 0,
            streaming_requests: 0,
            non_streaming_requests: 0,
            completions_requests: 0,
            responses_requests: 0,
            websocket_requests: 0,
            messages_requests: 0,
            image_requests: 0,
            active_websocket_connections: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens,
            total_errors: 0,
            rate_limit_errors: 0,
            server_errors: 0,
            timeout_errors: 0,
            requests_per_second,
            avg_latency_ms: 0.0,
            min_latency_ms: None,
            max_latency_ms: None,
            model_requests: Default::default(),
        }
    }

    #[test]
    fn ingest_derives_token_rate_after_the_first_snapshot() {
        let mut data = DashboardData::new();

        // First snapshot seeds the token baseline; no rate can be derived yet.
        data.ingest(snapshot(100, 1.0));
        assert_eq!(data.rps_history, vec![1.0]);
        assert!(
            data.tokens_history.is_empty(),
            "no token rate on the first sample"
        );

        // Second snapshot has a baseline to diff against, so a rate is recorded.
        data.ingest(snapshot(300, 2.0));
        assert_eq!(data.rps_history, vec![1.0, 2.0]);
        assert_eq!(data.tokens_history.len(), 1);
        assert!(
            data.tokens_history[0] > 0.0,
            "200 new tokens over a positive interval is a positive rate"
        );
        assert_eq!(data.stats.as_ref().map(|s| s.total_tokens), Some(300));
    }

    #[test]
    fn ingest_caps_rolling_history_at_60_samples() {
        let mut data = DashboardData::new();
        for i in 0..70 {
            data.ingest(snapshot((i + 1) * 10, i as f64));
        }
        assert_eq!(data.rps_history.len(), 60, "rps history is bounded");
        assert_eq!(data.tokens_history.len(), 60, "token history is bounded");
    }

    /// Serve the `/llmsim/stats` JSON to any number of connections, closing
    /// each one so the fetch's `read_to_end` completes. Returns the base URL to
    /// point the dashboard at.
    async fn stub_stats_server(snapshot: StatsSnapshot) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = serde_json::to_string(&snapshot).unwrap();

        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = socket.read(&mut buf).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });

        format!("http://{}", addr)
    }

    /// An address nothing is listening on: bind a port, then drop the listener.
    async fn unbound_url() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{}", addr)
    }

    fn key(code: KeyCode) -> Signal {
        Signal::Event(Event::Key(Key::new(code)))
    }

    #[tokio::test]
    async fn tick_polls_stats_and_repaints() {
        let url = stub_stats_server(snapshot(500, 3.0)).await;
        let mut data = DashboardData::new();

        let result = update(&url, &mut data, Signal::Tick).await;

        assert_eq!(result, UpdateResult::Dirty, "a fresh sample must repaint");
        assert_eq!(data.stats.as_ref().map(|s| s.total_tokens), Some(500));
        assert_eq!(data.rps_history, vec![3.0]);
        assert!(data.error.is_none());
    }

    #[tokio::test]
    async fn tick_against_a_dead_server_records_the_error_and_still_repaints() {
        let url = unbound_url().await;
        let mut data = DashboardData::new();

        let result = update(&url, &mut data, Signal::Tick).await;

        assert_eq!(
            result,
            UpdateResult::Dirty,
            "flipping to DISCONNECTED is a visible change"
        );
        assert!(data.error.is_some(), "the failed fetch is recorded");
        assert!(data.stats.is_none());
    }

    #[tokio::test]
    async fn r_refreshes_out_of_the_tick_cadence() {
        let url = stub_stats_server(snapshot(42, 1.0)).await;
        let mut data = DashboardData::new();

        let result = update(&url, &mut data, key(KeyCode::Char('r'))).await;

        assert_eq!(result, UpdateResult::Dirty);
        assert_eq!(data.stats.as_ref().map(|s| s.total_tokens), Some(42));
    }

    #[tokio::test]
    async fn q_and_esc_quit() {
        let url = unbound_url().await;
        let mut data = DashboardData::new();

        for code in [KeyCode::Char('q'), KeyCode::Esc] {
            assert_eq!(
                update(&url, &mut data, key(code)).await,
                UpdateResult::Exit,
                "{:?} quits the dashboard",
                code
            );
        }
    }

    #[tokio::test]
    async fn unhandled_input_stays_clean() {
        let url = unbound_url().await;
        let mut data = DashboardData::new();

        // An unbound key: no state change, and the runner keeps its default
        // handling for the input.
        assert_eq!(
            update(&url, &mut data, key(KeyCode::Char('x'))).await,
            UpdateResult::Clean
        );

        // A modified chord is not the bare `q` binding, so it must not quit.
        let ctrl_q = Signal::Event(Event::Key(Key {
            code: KeyCode::Char('q'),
            ctrl: true,
            alt: false,
            shift: false,
        }));
        assert_eq!(update(&url, &mut data, ctrl_q).await, UpdateResult::Clean);

        assert!(data.stats.is_none(), "no fetch was triggered");
        assert!(data.error.is_none());
    }

    #[test]
    fn set_error_marks_disconnected_then_ingest_clears_it() {
        let mut data = DashboardData::new();

        data.set_error("connection refused".to_string());
        assert_eq!(data.error.as_deref(), Some("connection refused"));

        // A successful fetch clears the disconnected state.
        data.ingest(snapshot(10, 1.0));
        assert!(data.error.is_none());
    }
}
