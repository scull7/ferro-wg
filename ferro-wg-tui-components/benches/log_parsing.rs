# Performance benchmark for log parsing

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ferro_wg_tui_components::logs::LogsComponent;

fn benchmark_log_parsing(c: &mut Criterion) {
    let test_lines = vec![
        "12:34:56 INFO ferro_wg_core::tunnel::mod: Connection established",
        "12:34:57 ERROR ferro_wg_core::error: Failed to bind socket",
        "12:34:58 WARN ferro_wg_core::tunnel: Handshake timeout",
        "12:34:59 DEBUG ferro_wg_core::stats: Packets: 42 rx, 37 tx",
        "INFO ferro_wg_core::legacy: Old format log", // Legacy format
        "some malformed log message", // Malformed
    ];

    c.bench_function("parse_log_lines", |b| {
        b.iter(|| {
            for line in &test_lines {
                black_box(LogsComponent::parse_log_line(line, true, true));
            }
        })
    });

    c.bench_function("parse_log_lines_no_timestamps", |b| {
        b.iter(|| {
            for line in &test_lines {
                black_box(LogsComponent::parse_log_line(line, false, true));
            }
        })
    });

    c.bench_function("parse_log_lines_no_colors", |b| {
        b.iter(|| {
            for line in &test_lines {
                black_box(LogsComponent::parse_log_line(line, true, false));
            }
        })
    });
}

criterion_group!(benches, benchmark_log_parsing);
criterion_main!(benches);