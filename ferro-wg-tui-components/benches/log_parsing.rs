//! Performance benchmarks for `LogsComponent::parse_log_line`.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use ferro_wg_core::config::LogDisplayConfig;
use ferro_wg_tui_components::logs::LogsComponent;

fn benchmark_log_parsing(c: &mut Criterion) {
    let test_lines = vec![
        "12:34:56 INFO ferro_wg_core::tunnel::mod: Connection established",
        "12:34:57 ERROR ferro_wg_core::error: Failed to bind socket",
        "12:34:58 WARN ferro_wg_core::tunnel: Handshake timeout",
        "12:34:59 DEBUG ferro_wg_core::stats: Packets: 42 rx, 37 tx",
        "INFO ferro_wg_core::legacy: Old format log", // Legacy format
        "some malformed log message",                 // Malformed
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

    c.bench_function("parse_log_lines", |b| {
        b.iter(|| {
            for line in &test_lines {
                let _ = black_box(LogsComponent::parse_log_line(line, &cfg_all));
            }
        });
    });

    c.bench_function("parse_log_lines_no_timestamps", |b| {
        b.iter(|| {
            for line in &test_lines {
                let _ = black_box(LogsComponent::parse_log_line(line, &cfg_no_ts));
            }
        });
    });

    c.bench_function("parse_log_lines_no_colors", |b| {
        b.iter(|| {
            for line in &test_lines {
                let _ = black_box(LogsComponent::parse_log_line(line, &cfg_no_color));
            }
        });
    });
}

criterion_group!(benches, benchmark_log_parsing);
criterion_main!(benches);
