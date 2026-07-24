//! TUI rendering: the dashboard as a tree of `tuika` views.
//!
//! Design note: composition (the margin, the row/column splits, the 60/40 and
//! 50/50 panels) is expressed with `tuika`'s flexbox `Flex`/`Dimension` instead
//! of ratatui's `Layout`. The data panels themselves are still drawn with
//! ratatui widgets (`Table`, `Sparkline`, `BarChart`, `Paragraph`) because
//! `tuika` has no charting widgets of its own; each panel is wrapped in a
//! `RatatuiView` so the widget code renders through `tuika`'s clipped surface
//! rather than touching the frame buffer directly. Keeping the widgets means
//! the rendered dashboard is pixel-identical to the previous ratatui version.
//!
//! Every `RatatuiView` closure must be `'static`, so each panel takes owned
//! copies of the values it draws; `dashboard` pulls them out of the snapshot
//! once and hands each panel exactly what it needs.

use super::app::DashboardData;
use ratatui::{
    layout::Constraint,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Bar, BarChart, BarGroup, Block, Borders, Paragraph, Row, Sparkline, Table, Widget},
};
use tuika::{element, Dimension, Element, Flex, Padding, RatatuiView};

/// Build the whole dashboard tree for the current snapshot.
///
/// Layout mirrors the original: a 1-cell margin around a vertical stack of a
/// header (3), the request/token row (9), the latency/error row (8), the charts
/// (flex, min 8), and a one-line footer.
pub fn dashboard(data: &DashboardData) -> Element {
    let stats = data.stats.as_ref();

    let uptime = stats
        .map(|s| format_uptime(s.uptime_secs))
        .unwrap_or_else(|| "N/A".to_string());
    let connected = data.error.is_none();

    element(
        Flex::column()
            .padding(Padding::all(1))
            .child(Dimension::Fixed(3), header(uptime, connected))
            .child(Dimension::Fixed(9), stats_row(data))
            .child(Dimension::Fixed(8), latency_errors_row(data))
            .grow(1, charts(data))
            .child(Dimension::Fixed(1), footer()),
    )
}

fn header(uptime: String, connected: bool) -> Element {
    element(RatatuiView::fill(move |area, buf| {
        let status = if connected {
            Span::styled("● CONNECTED", Style::default().fg(Color::Green).bold())
        } else {
            Span::styled("● DISCONNECTED", Style::default().fg(Color::Red).bold())
        };

        Paragraph::new(Line::from(vec![
            Span::styled(
                "  LLMSim Stats Dashboard  ",
                Style::default().fg(Color::Cyan).bold(),
            ),
            Span::raw(" │ "),
            status,
            Span::raw(" │ Uptime: "),
            Span::styled(uptime.clone(), Style::default().fg(Color::Yellow)),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .render(area, buf);
    }))
}

fn stats_row(data: &DashboardData) -> Element {
    element(
        Flex::row()
            .grow(1, request_stats(data))
            .grow(1, token_stats(data)),
    )
}

fn request_stats(data: &DashboardData) -> Element {
    let stats = data.stats.as_ref();
    let total = stats.map(|s| s.total_requests).unwrap_or(0);
    let active = stats.map(|s| s.active_requests).unwrap_or(0);
    let completions = stats.map(|s| s.completions_requests).unwrap_or(0);
    let responses = stats.map(|s| s.responses_requests).unwrap_or(0);
    let messages = stats.map(|s| s.messages_requests).unwrap_or(0);
    let streaming = stats.map(|s| s.streaming_requests).unwrap_or(0);
    let rps = stats.map(|s| s.requests_per_second).unwrap_or(0.0);

    element(RatatuiView::fill(move |area, buf| {
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
                Span::raw("Messages API"),
                Span::styled(
                    format_number(messages),
                    Style::default().fg(Color::LightRed),
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

        Table::new(
            rows,
            [Constraint::Percentage(60), Constraint::Percentage(40)],
        )
        .block(
            Block::default()
                .title(" Requests ")
                .title_style(Style::default().fg(Color::Green).bold())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        )
        .render(area, buf);
    }))
}

fn token_stats(data: &DashboardData) -> Element {
    let stats = data.stats.as_ref();
    let prompt = stats.map(|s| s.prompt_tokens).unwrap_or(0);
    let completion = stats.map(|s| s.completion_tokens).unwrap_or(0);
    let total = stats.map(|s| s.total_tokens).unwrap_or(0);
    let token_rate = data.tokens_history.last().copied().unwrap_or(0.0);

    element(RatatuiView::fill(move |area, buf| {
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

        Table::new(
            rows,
            [Constraint::Percentage(60), Constraint::Percentage(40)],
        )
        .block(
            Block::default()
                .title(" Tokens ")
                .title_style(Style::default().fg(Color::Cyan).bold())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .render(area, buf);
    }))
}

fn latency_errors_row(data: &DashboardData) -> Element {
    element(
        Flex::row()
            .grow(1, latency_stats(data))
            .grow(1, error_stats(data)),
    )
}

fn latency_stats(data: &DashboardData) -> Element {
    let stats = data.stats.as_ref();
    let avg = stats.map(|s| s.avg_latency_ms).unwrap_or(0.0);
    let min = stats.and_then(|s| s.min_latency_ms).unwrap_or(0.0);
    let max = stats.and_then(|s| s.max_latency_ms).unwrap_or(0.0);

    element(RatatuiView::fill(move |area, buf| {
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

        Table::new(
            rows,
            [Constraint::Percentage(50), Constraint::Percentage(50)],
        )
        .block(
            Block::default()
                .title(" Latency ")
                .title_style(Style::default().fg(Color::Yellow).bold())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .render(area, buf);
    }))
}

fn error_stats(data: &DashboardData) -> Element {
    let stats = data.stats.as_ref();
    let total = stats.map(|s| s.total_errors).unwrap_or(0);
    let rate_limit = stats.map(|s| s.rate_limit_errors).unwrap_or(0);
    let server = stats.map(|s| s.server_errors).unwrap_or(0);
    let timeout = stats.map(|s| s.timeout_errors).unwrap_or(0);

    let total_requests = stats.map(|s| s.total_requests).unwrap_or(0);
    let error_rate = if total_requests > 0 {
        (total as f64 / total_requests as f64) * 100.0
    } else {
        0.0
    };

    element(RatatuiView::fill(move |area, buf| {
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

        Table::new(
            rows,
            [Constraint::Percentage(60), Constraint::Percentage(40)],
        )
        .block(
            Block::default()
                .title(" Errors ")
                .title_style(Style::default().fg(Color::Red).bold())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red)),
        )
        .render(area, buf);
    }))
}

fn charts(data: &DashboardData) -> Element {
    element(
        Flex::row()
            .grow(3, sparklines(data))
            .grow(2, model_chart(data)),
    )
}

fn sparklines(data: &DashboardData) -> Element {
    element(
        Flex::column()
            .grow(1, rps_sparkline(data))
            .grow(1, tokens_sparkline(data)),
    )
}

fn rps_sparkline(data: &DashboardData) -> Element {
    let rps_data: Vec<u64> = data
        .rps_history
        .iter()
        .map(|v| (*v * 10.0) as u64)
        .collect();
    let current = data.rps_history.last().copied().unwrap_or(0.0);
    let max = data.rps_history.iter().copied().fold(0.0_f64, f64::max);

    element(RatatuiView::fill(move |area, buf| {
        Sparkline::default()
            .block(
                Block::default()
                    .title(format!(" RPS (current: {:.2}, max: {:.2}) ", current, max))
                    .title_style(Style::default().fg(Color::Green))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green)),
            )
            .data(&rps_data)
            .style(Style::default().fg(Color::Green))
            .render(area, buf);
    }))
}

fn tokens_sparkline(data: &DashboardData) -> Element {
    let token_data: Vec<u64> = data.tokens_history.iter().map(|v| *v as u64).collect();
    let current = data.tokens_history.last().copied().unwrap_or(0.0);
    let max = data.tokens_history.iter().copied().fold(0.0_f64, f64::max);

    element(RatatuiView::fill(move |area, buf| {
        Sparkline::default()
            .block(
                Block::default()
                    .title(format!(
                        " Tokens/sec (current: {:.0}, max: {:.0}) ",
                        current, max
                    ))
                    .title_style(Style::default().fg(Color::Cyan))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .data(&token_data)
            .style(Style::default().fg(Color::Cyan))
            .render(area, buf);
    }))
}

fn model_chart(data: &DashboardData) -> Element {
    let model_requests = data
        .stats
        .as_ref()
        .map(|s| s.model_requests.clone())
        .unwrap_or_default();

    element(RatatuiView::fill(move |area, buf| {
        if model_requests.is_empty() {
            Paragraph::new("No requests yet")
                .style(Style::default().fg(Color::Gray))
                .block(
                    Block::default()
                        .title(" Models ")
                        .title_style(Style::default().fg(Color::Magenta).bold())
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Magenta)),
                )
                .render(area, buf);
            return;
        }

        // Sort by count and take the top models.
        let mut model_vec: Vec<_> = model_requests.clone().into_iter().collect();
        model_vec.sort_by_key(|m| std::cmp::Reverse(m.1));
        model_vec.truncate(8);

        let bars: Vec<Bar> = model_vec
            .iter()
            .map(|(model, count)| {
                // Shorten model name if too long.
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

        BarChart::default()
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
            )
            .render(area, buf);
    }))
}

fn footer() -> Element {
    element(RatatuiView::fill(move |area, buf| {
        Paragraph::new(Line::from(vec![
            Span::styled(" q ", Style::default().fg(Color::Black).bg(Color::White)),
            Span::raw(" Quit  "),
            Span::styled(" r ", Style::default().fg(Color::Black).bg(Color::White)),
            Span::raw(" Refresh  "),
        ]))
        .style(Style::default().fg(Color::Gray))
        .render(area, buf);
    }))
}

/// Format uptime in human-readable format.
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

/// Format large numbers with K/M/B suffixes.
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
