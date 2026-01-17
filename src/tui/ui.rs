//! TUI rendering logic using Ratatui.

use super::app::App;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Bar, BarChart, BarGroup, Block, Borders, Paragraph, Row, Sparkline, Table},
    Frame,
};

/// Main draw function
pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(8), // Request stats + Token stats
            Constraint::Length(8), // Latency + Errors
            Constraint::Min(8),    // Charts
            Constraint::Length(1), // Footer
        ])
        .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_stats_row(f, app, chunks[1]);
    draw_latency_errors_row(f, app, chunks[2]);
    draw_charts(f, app, chunks[3]);
    draw_footer(f, chunks[4]);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let uptime = app
        .stats
        .as_ref()
        .map(|s| format_uptime(s.uptime_secs))
        .unwrap_or_else(|| "N/A".to_string());

    let status = if app.error.is_some() {
        Span::styled("● DISCONNECTED", Style::default().fg(Color::Red).bold())
    } else {
        Span::styled("● CONNECTED", Style::default().fg(Color::Green).bold())
    };

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "  LLMSim Stats Dashboard  ",
            Style::default().fg(Color::Cyan).bold(),
        ),
        Span::raw(" │ "),
        status,
        Span::raw(" │ Uptime: "),
        Span::styled(uptime, Style::default().fg(Color::Yellow)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );

    f.render_widget(header, area);
}

fn draw_stats_row(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    draw_request_stats(f, app, chunks[0]);
    draw_token_stats(f, app, chunks[1]);
}

fn draw_request_stats(f: &mut Frame, app: &App, area: Rect) {
    let stats = app.stats.as_ref();

    let total = stats.map(|s| s.total_requests).unwrap_or(0);
    let active = stats.map(|s| s.active_requests).unwrap_or(0);
    let completions = stats.map(|s| s.completions_requests).unwrap_or(0);
    let responses = stats.map(|s| s.responses_requests).unwrap_or(0);
    let streaming = stats.map(|s| s.streaming_requests).unwrap_or(0);
    let rps = stats.map(|s| s.requests_per_second).unwrap_or(0.0);

    let rows = vec![
        Row::new(vec![
            Span::raw("Total Requests"),
            Span::styled(
                format_number(total),
                Style::default().fg(Color::Green).bold(),
            ),
        ]),
        Row::new(vec![
            Span::raw("Active Requests"),
            Span::styled(
                format!("{}", active),
                if active > 0 {
                    Style::default().fg(Color::Yellow).bold()
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
        ]),
        Row::new(vec![
            Span::raw("Completions API"),
            Span::styled(format_number(completions), Style::default().fg(Color::Cyan)),
        ]),
        Row::new(vec![
            Span::raw("Responses API"),
            Span::styled(
                format_number(responses),
                Style::default().fg(Color::Magenta),
            ),
        ]),
        Row::new(vec![
            Span::raw("Streaming"),
            Span::styled(format_number(streaming), Style::default().fg(Color::Blue)),
        ]),
        Row::new(vec![
            Span::raw("Requests/sec"),
            Span::styled(
                format!("{:.2}", rps),
                Style::default().fg(Color::Green).bold(),
            ),
        ]),
    ];

    let table = Table::new(
        rows,
        [Constraint::Percentage(60), Constraint::Percentage(40)],
    )
    .block(
        Block::default()
            .title(" Requests ")
            .title_style(Style::default().fg(Color::Green).bold())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green)),
    );

    f.render_widget(table, area);
}

fn draw_token_stats(f: &mut Frame, app: &App, area: Rect) {
    let stats = app.stats.as_ref();

    let prompt = stats.map(|s| s.prompt_tokens).unwrap_or(0);
    let completion = stats.map(|s| s.completion_tokens).unwrap_or(0);
    let total = stats.map(|s| s.total_tokens).unwrap_or(0);

    // Calculate token rate
    let token_rate = if !app.tokens_history.is_empty() {
        *app.tokens_history.last().unwrap_or(&0.0)
    } else {
        0.0
    };

    let rows = vec![
        Row::new(vec![
            Span::raw("Prompt Tokens"),
            Span::styled(format_number(prompt), Style::default().fg(Color::Blue)),
        ]),
        Row::new(vec![
            Span::raw("Completion Tokens"),
            Span::styled(
                format_number(completion),
                Style::default().fg(Color::Magenta),
            ),
        ]),
        Row::new(vec![
            Span::raw("Total Tokens"),
            Span::styled(
                format_number(total),
                Style::default().fg(Color::Cyan).bold(),
            ),
        ]),
        Row::new(vec![
            Span::raw("Tokens/sec"),
            Span::styled(
                format!("{:.1}", token_rate),
                Style::default().fg(Color::Green),
            ),
        ]),
    ];

    let table = Table::new(
        rows,
        [Constraint::Percentage(60), Constraint::Percentage(40)],
    )
    .block(
        Block::default()
            .title(" Tokens ")
            .title_style(Style::default().fg(Color::Cyan).bold())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );

    f.render_widget(table, area);
}

fn draw_latency_errors_row(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    draw_latency_stats(f, app, chunks[0]);
    draw_error_stats(f, app, chunks[1]);
}

fn draw_latency_stats(f: &mut Frame, app: &App, area: Rect) {
    let stats = app.stats.as_ref();

    let avg = stats.map(|s| s.avg_latency_ms).unwrap_or(0.0);
    let min = stats.and_then(|s| s.min_latency_ms).unwrap_or(0.0);
    let max = stats.and_then(|s| s.max_latency_ms).unwrap_or(0.0);

    let rows = vec![
        Row::new(vec![
            Span::raw("Average"),
            Span::styled(format!("{:.2} ms", avg), Style::default().fg(Color::Yellow)),
        ]),
        Row::new(vec![
            Span::raw("Minimum"),
            Span::styled(format!("{:.2} ms", min), Style::default().fg(Color::Green)),
        ]),
        Row::new(vec![
            Span::raw("Maximum"),
            Span::styled(
                format!("{:.2} ms", max),
                if max > 1000.0 {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::Yellow)
                },
            ),
        ]),
    ];

    let table = Table::new(
        rows,
        [Constraint::Percentage(50), Constraint::Percentage(50)],
    )
    .block(
        Block::default()
            .title(" Latency ")
            .title_style(Style::default().fg(Color::Yellow).bold())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow)),
    );

    f.render_widget(table, area);
}

fn draw_error_stats(f: &mut Frame, app: &App, area: Rect) {
    let stats = app.stats.as_ref();

    let total = stats.map(|s| s.total_errors).unwrap_or(0);
    let rate_limit = stats.map(|s| s.rate_limit_errors).unwrap_or(0);
    let server = stats.map(|s| s.server_errors).unwrap_or(0);
    let timeout = stats.map(|s| s.timeout_errors).unwrap_or(0);

    // Calculate error rate
    let total_requests = stats.map(|s| s.total_requests).unwrap_or(0);
    let error_rate = if total_requests > 0 {
        (total as f64 / total_requests as f64) * 100.0
    } else {
        0.0
    };

    let error_style = if total > 0 {
        Style::default().fg(Color::Red).bold()
    } else {
        Style::default().fg(Color::Gray)
    };

    let rows = vec![
        Row::new(vec![
            Span::raw("Total Errors"),
            Span::styled(format!("{} ({:.1}%)", total, error_rate), error_style),
        ]),
        Row::new(vec![
            Span::raw("Rate Limit (429)"),
            Span::styled(
                format!("{}", rate_limit),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Row::new(vec![
            Span::raw("Server (5xx)"),
            Span::styled(format!("{}", server), Style::default().fg(Color::Red)),
        ]),
        Row::new(vec![
            Span::raw("Timeout (504)"),
            Span::styled(format!("{}", timeout), Style::default().fg(Color::Magenta)),
        ]),
    ];

    let table = Table::new(
        rows,
        [Constraint::Percentage(60), Constraint::Percentage(40)],
    )
    .block(
        Block::default()
            .title(" Errors ")
            .title_style(Style::default().fg(Color::Red).bold())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red)),
    );

    f.render_widget(table, area);
}

fn draw_charts(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    draw_sparklines(f, app, chunks[0]);
    draw_model_chart(f, app, chunks[1]);
}

fn draw_sparklines(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // RPS Sparkline
    let rps_data: Vec<u64> = app.rps_history.iter().map(|v| (*v * 10.0) as u64).collect();
    let current_rps = app.rps_history.last().copied().unwrap_or(0.0);
    let max_rps = app.rps_history.iter().copied().fold(0.0_f64, f64::max);

    let rps_sparkline = Sparkline::default()
        .block(
            Block::default()
                .title(format!(
                    " RPS (current: {:.2}, max: {:.2}) ",
                    current_rps, max_rps
                ))
                .title_style(Style::default().fg(Color::Green))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        )
        .data(&rps_data)
        .style(Style::default().fg(Color::Green));

    f.render_widget(rps_sparkline, chunks[0]);

    // Token rate Sparkline
    let token_data: Vec<u64> = app.tokens_history.iter().map(|v| *v as u64).collect();
    let current_tokens = app.tokens_history.last().copied().unwrap_or(0.0);
    let max_tokens = app.tokens_history.iter().copied().fold(0.0_f64, f64::max);

    let token_sparkline = Sparkline::default()
        .block(
            Block::default()
                .title(format!(
                    " Tokens/sec (current: {:.0}, max: {:.0}) ",
                    current_tokens, max_tokens
                ))
                .title_style(Style::default().fg(Color::Cyan))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .data(&token_data)
        .style(Style::default().fg(Color::Cyan));

    f.render_widget(token_sparkline, chunks[1]);
}

fn draw_model_chart(f: &mut Frame, app: &App, area: Rect) {
    let stats = app.stats.as_ref();

    let model_requests = stats.map(|s| s.model_requests.clone()).unwrap_or_default();

    if model_requests.is_empty() {
        let empty = Paragraph::new("No requests yet")
            .style(Style::default().fg(Color::Gray))
            .block(
                Block::default()
                    .title(" Models ")
                    .title_style(Style::default().fg(Color::Magenta).bold())
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Magenta)),
            );
        f.render_widget(empty, area);
        return;
    }

    // Sort by count and take top models
    let mut model_vec: Vec<_> = model_requests.into_iter().collect();
    model_vec.sort_by(|a, b| b.1.cmp(&a.1));
    model_vec.truncate(8);

    // Create bars
    let bars: Vec<Bar> = model_vec
        .iter()
        .map(|(model, count)| {
            // Shorten model name if too long
            let short_name = if model.len() > 12 {
                format!("{}...", &model[..9])
            } else {
                model.clone()
            };
            Bar::default()
                .value(*count)
                .label(Line::from(short_name))
                .style(Style::default().fg(Color::Magenta))
        })
        .collect();

    let bar_chart = BarChart::default()
        .block(
            Block::default()
                .title(" Models (top 8) ")
                .title_style(Style::default().fg(Color::Magenta).bold())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta)),
        )
        .data(BarGroup::default().bars(&bars))
        .bar_width(3)
        .bar_gap(1)
        .bar_style(Style::default().fg(Color::Magenta))
        .value_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );

    f.render_widget(bar_chart, area);
}

fn draw_footer(f: &mut Frame, area: Rect) {
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" q ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw(" Quit  "),
        Span::styled(" r ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw(" Refresh  "),
    ]))
    .style(Style::default().fg(Color::Gray));

    f.render_widget(footer, area);
}

/// Format uptime in human-readable format
fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;

    if days > 0 {
        format!("{}d {}h {}m {}s", days, hours, minutes, seconds)
    } else if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}

/// Format large numbers with K/M/B suffixes
fn format_number(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2}B", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.2}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}
