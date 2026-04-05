//! Performance benchmarks for `LogsComponent::render_entry`.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use ferro_wg_core::config::LogDisplayConfig;
use ferro_wg_core::ipc::{LogEntry, LogLevel};
use ferro_wg_tui_components::logs::LogsComponent;
use ferro_wg_tui_core::Theme;

fn make_entry(level: LogLevel, connection: Option<&str>, msg: &str) -> LogEntry {
    LogEntry {
        timestamp_ms: 1_712_188_800_000,
        level,
        connection_name: connection.map(ToOwned::to_owned),
        message: msg.to_owned(),
    }
}

fn benchmark_render_entry(c: &mut Criterion) {
    let entries = vec![
        make_entry(LogLevel::Info, Some("mia"), "Connection established"),
        make_entry(LogLevel::Error, Some("mia"), "Failed to bind socket"),
        make_entry(LogLevel::Warn, None, "Handshake timeout"),
        make_entry(LogLevel::Debug, Some("lon"), "Packets: 42 rx, 37 tx"),
        make_entry(LogLevel::Trace, None, "Internal trace event"),
    ];

    let cfg_all = LogDisplayConfig {
        show_timestamps: true,
        color_badges: true,
    };
    let cfg_no_ts = LogDisplayConfig {
        show_timestamps: false,
        color_badges: true,
    };
    let cfg_no_color = LogDisplayConfig {
        show_timestamps: true,
        color_badges: false,
    };

    let theme = Theme::mocha();

    c.bench_function("render_entry_all_options", |b| {
        b.iter(|| {
            for entry in &entries {
                let _ = black_box(LogsComponent::render_entry(entry, &cfg_all, &theme));
            }
        });
    });

    c.bench_function("render_entry_no_timestamps", |b| {
        b.iter(|| {
            for entry in &entries {
                let _ = black_box(LogsComponent::render_entry(entry, &cfg_no_ts, &theme));
            }
        });
    });

    c.bench_function("render_entry_no_colors", |b| {
        b.iter(|| {
            for entry in &entries {
                let _ = black_box(LogsComponent::render_entry(entry, &cfg_no_color, &theme));
            }
        });
    });
}

criterion_group!(benches, benchmark_render_entry);
criterion_main!(benches);
