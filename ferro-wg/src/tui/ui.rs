//! Pure rendering functions for each TUI view.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Tabs};

use super::app::{App, InputMode, Tab};

/// Draw the entire TUI frame.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Tab bar
            Constraint::Min(0),    // Main content
            Constraint::Length(3), // Status bar / search
        ])
        .split(frame.area());

    draw_tabs(frame, app, chunks[0]);
    draw_content(frame, app, chunks[1]);
    draw_status_bar(frame, app, chunks[2]);
}

/// Draw the tab bar at the top.
fn draw_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line<'_>> = Tab::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let num = i + 1;
            Line::from(format!(" {num}:{} ", t.title()))
        })
        .collect();

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title(" ferro-wg "))
        .select(app.active_tab.index())
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(tabs, area);
}

/// Dispatch to the active tab's content renderer.
fn draw_content(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.active_tab {
        Tab::Status => draw_status_view(frame, app, area),
        Tab::Peers => draw_peers_view(frame, app, area),
        Tab::Compare => draw_compare_view(frame, app, area),
        Tab::Config => draw_config_view(frame, app, area),
        Tab::Logs => draw_logs_view(frame, app, area),
    }
}

/// Draw the bottom status bar (search input or help text).
fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let content = match app.input_mode {
        InputMode::Search => Line::from(vec![
            Span::styled(" /", Style::default().fg(Color::Yellow)),
            Span::raw(&app.search_query),
            Span::styled("_", Style::default().fg(Color::DarkGray)),
        ]),
        InputMode::Normal => Line::from(vec![
            Span::styled(
                " q",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" quit  "),
            Span::styled(
                "/",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" search  "),
            Span::styled(
                "1-5",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" tabs  "),
            Span::styled(
                "j/k",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" navigate"),
        ]),
    };

    let block = Block::default().borders(Borders::ALL);
    let paragraph = Paragraph::new(content).block(block);
    frame.render_widget(paragraph, area);
}

// -- Tab-specific views --

/// Status tab: active tunnels overview.
fn draw_status_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let header = Row::new(vec!["Peer", "Endpoint", "Status", "Rx", "Tx", "Handshake"]).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row<'static>> = app
        .filtered_peers()
        .map(|p| {
            let status_str: String = if p.connected {
                "connected".into()
            } else {
                "down".into()
            };
            let status_style = if p.connected {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let name = p.config.name.clone();
            let endpoint = p.config.endpoint.clone().unwrap_or_else(|| "-".into());
            let hs = p
                .stats
                .last_handshake
                .map_or_else(|| "-".to_owned(), |d| format!("{}s ago", d.as_secs()));
            let rx = format_bytes(p.stats.rx_bytes);
            let tx = format_bytes(p.stats.tx_bytes);

            Row::new(vec![
                Cell::from(name),
                Cell::from(endpoint),
                Cell::from(status_str).style(status_style),
                Cell::from(rx),
                Cell::from(tx),
                Cell::from(hs),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(20),
            Constraint::Percentage(25),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(19),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(" Status "))
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(table, area, &mut app.table_state);
}

/// Peers tab: all configured peers with connect/disconnect.
fn draw_peers_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let header = Row::new(vec![
        "Peer",
        "Public Key",
        "Endpoint",
        "Allowed IPs",
        "Keepalive",
        "Backend",
    ])
    .style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row<'static>> = app
        .filtered_peers()
        .map(|p| {
            let pk = p.config.public_key.to_base64();
            let short_pk = format!("{}...", &pk[..10]);
            let name = p.config.name.clone();
            let endpoint = p.config.endpoint.clone().unwrap_or_else(|| "-".into());
            let allowed = p.config.allowed_ips.join(", ");
            let keepalive = format!("{}s", p.config.persistent_keepalive);
            let backend = p.backend.to_string();

            Row::new(vec![
                Cell::from(name),
                Cell::from(short_pk),
                Cell::from(endpoint),
                Cell::from(allowed),
                Cell::from(keepalive),
                Cell::from(backend),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(10),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(" Peers "))
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(table, area, &mut app.table_state);
}

/// Compare tab: backend performance comparison.
fn draw_compare_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let header = Row::new(vec![
        "Backend",
        "Available",
        "Encap/s",
        "Throughput",
        "Latency",
    ])
    .style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let backends = [
        ("boringtun", cfg!(feature = "boringtun")),
        ("neptun", cfg!(feature = "neptun")),
        ("gotatun", cfg!(feature = "gotatun")),
    ];

    let rows: Vec<Row<'_>> = backends
        .iter()
        .map(|(name, available)| {
            let avail = if *available { "yes" } else { "no" };
            let avail_style = if *available {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            };
            Row::new(vec![
                Cell::from(*name),
                Cell::from(avail).style(avail_style),
                Cell::from("-"),
                Cell::from("-"),
                Cell::from("-"),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(20),
            Constraint::Percentage(15),
            Constraint::Percentage(20),
            Constraint::Percentage(25),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Compare (run benchmarks to populate) "),
    )
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(table, area, &mut app.table_state);
}

/// Config tab: interface and peer settings.
fn draw_config_view(frame: &mut Frame, app: &App, area: Rect) {
    let public_key = app.wg_config.interface.private_key.public_key().to_base64();
    let listen = app.wg_config.interface.listen_port;
    let addrs = app.wg_config.interface.addresses.join(", ");
    let dns: Vec<String> = app
        .wg_config
        .interface
        .dns
        .iter()
        .map(ToString::to_string)
        .collect();

    let text = vec![
        Line::from(vec![
            Span::styled("Public Key: ", Style::default().fg(Color::Cyan)),
            Span::raw(public_key),
        ]),
        Line::from(vec![
            Span::styled("Listen Port: ", Style::default().fg(Color::Cyan)),
            Span::raw(listen.to_string()),
        ]),
        Line::from(vec![
            Span::styled("Address: ", Style::default().fg(Color::Cyan)),
            Span::raw(if addrs.is_empty() {
                "(none)".to_owned()
            } else {
                addrs
            }),
        ]),
        Line::from(vec![
            Span::styled("DNS: ", Style::default().fg(Color::Cyan)),
            Span::raw(if dns.is_empty() {
                "(none)".to_owned()
            } else {
                dns.join(", ")
            }),
        ]),
        Line::from(vec![
            Span::styled("MTU: ", Style::default().fg(Color::Cyan)),
            Span::raw(app.wg_config.interface.mtu.to_string()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!("{} peer(s) configured", app.wg_config.peers.len()),
            Style::default().fg(Color::Yellow),
        )),
    ];

    let paragraph =
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(" Config "));
    frame.render_widget(paragraph, area);
}

/// Logs tab: scrollable log viewer.
fn draw_logs_view(frame: &mut Frame, app: &App, area: Rect) {
    let lines: Vec<Line<'_>> = if app.log_lines.is_empty() {
        vec![Line::from(Span::styled(
            "(no log entries yet)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.log_lines
            .iter()
            .map(|l| Line::from(l.as_str()))
            .collect()
    };

    let paragraph =
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Logs "));
    frame.render_widget(paragraph, area);
}

/// Format a byte count into a human-readable string.
fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;

    if bytes >= GIB {
        #[allow(clippy::cast_precision_loss)]
        let val = bytes as f64 / GIB as f64;
        format!("{val:.1} GiB")
    } else if bytes >= MIB {
        #[allow(clippy::cast_precision_loss)]
        let val = bytes as f64 / MIB as f64;
        format!("{val:.1} MiB")
    } else if bytes >= KIB {
        #[allow(clippy::cast_precision_loss)]
        let val = bytes as f64 / KIB as f64;
        format!("{val:.1} KiB")
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1_048_576), "1.0 MiB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GiB");
    }
}
