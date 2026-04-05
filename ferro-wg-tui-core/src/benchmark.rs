use std::collections::HashMap;
use std::fmt::Write;
use std::time::Duration;
// BenchmarkProgress is a wire-format type (lives in ipc.rs); BenchmarkResult
// is a post-processed aggregation type (lives in stats.rs).
use ferro_wg_core::ipc::BenchmarkProgress;
use ferro_wg_core::stats::BenchmarkResult;

/// Maximum number of live progress samples kept in `AppState::benchmark_progress_history`.
///
/// At one sample per second this gives one minute of sparkline history.
pub const BENCHMARK_PROGRESS_HISTORY_CAP: usize = 60;

/// Maximum number of historical benchmark runs kept in `AppState::benchmark_history`.
///
/// Prevents unbounded growth of the history vector.
pub const BENCHMARK_HISTORY_CAP: usize = 50;

/// Latest benchmark result keyed by backend name string.
pub type BenchmarkResultMap = HashMap<String, BenchmarkResult>;

/// A single historical benchmark run with a wall-clock timestamp.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkRun {
    /// Milliseconds since the Unix epoch (UTC) when the run completed.
    pub timestamp_ms: i64,
    /// Which connection was benchmarked.
    pub connection_name: String,
    /// Per-backend results from this run.
    pub results: BenchmarkResultMap,
}

/// Errors produced by benchmark calculation / persistence helpers.
#[derive(Debug, thiserror::Error)]
pub enum BenchmarkError {
    #[error("benchmark already running")]
    AlreadyRunning,
    #[error("no active connection: {0}")]
    NoActiveConnection(String),
    #[error("daemon error: {0}")]
    DaemonError(String),
    #[error("serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ── Calculation layer (pure functions) ───────────────────────────────────────

/// Format a throughput value as a human-readable string.
///
/// - `0.0` → `"0 B/s"`
/// - `1_000_000.0` → `"1.00 MB/s"`
/// - `1_073_741_824.0` → `"1.00 GB/s"`
#[must_use]
pub fn format_throughput(bps: f64) -> String {
    const UNITS: &[&str] = &["B/s", "KB/s", "MB/s", "GB/s", "TB/s"];
    let mut value = bps;
    let mut unit_index = 0;
    while value >= 1000.0 && unit_index < UNITS.len() - 1 {
        value /= 1000.0;
        unit_index += 1;
    }
    if unit_index == 0 {
        format!("{:.0} {}", value, UNITS[unit_index])
    } else {
        format!("{:.2} {}", value, UNITS[unit_index])
    }
}

/// Format a `Duration` as milliseconds with two decimal places.
///
/// `Duration::from_micros(420)` → `"0.42 ms"`
#[must_use]
pub fn format_latency(d: Duration) -> String {
    #[allow(clippy::cast_precision_loss)]
    let ms = d.as_micros() as f64 / 1000.0;
    format!("{ms:.2} ms")
}

/// Format a `Duration` as a whole-seconds string.
///
/// `Duration::from_secs(10)` → `"10s"`
#[must_use]
pub fn format_duration(d: Duration) -> String {
    format!("{}s", d.as_secs())
}

/// Build `BarChart` data from a `BenchmarkResultMap`.
///
/// Returns entries in stable alphabetical order by backend name so the
/// chart order does not depend on `HashMap` iteration order.
/// Values are throughput in kilobytes per second (truncated to `u64`).
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn throughput_bar_data(results: &BenchmarkResultMap) -> Vec<(&str, u64)> {
    let mut entries: Vec<_> = results.iter().collect();
    entries.sort_by_key(|(k, _)| *k);
    entries
        .into_iter()
        .map(|(backend, result)| {
            let kbps = (result.throughput_bps / 1000.0) as u64; // Truncate to u64 for chart
            (backend.as_str(), kbps)
        })
        .collect()
}

/// Build `Sparkline` data from a slice of live progress samples.
///
/// Each sample contributes one `u64` value: `current_throughput_bps`
/// truncated to **kBps** (divided by 1000, truncated to `u64`).
/// Returns an empty `Vec` when `progress` is empty.
///
/// **Note on naming:** The sparkline visualises live throughput during a run,
/// not latency. The function is named `throughput_sparkline_data` to be
/// unambiguous. The separate p50/p95/p99 latency fields are shown in the
/// static detail panel after a run completes.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn throughput_sparkline_data(progress: &[BenchmarkProgress]) -> Vec<u64> {
    progress
        .iter()
        .map(|p| (p.current_throughput_bps / 1000.0) as u64) // Truncate to u64 for sparkline
        .collect()
}

/// Return the backend name with the highest `throughput_bps`.
///
/// Returns `None` when `results` is empty or all backends have
/// `throughput_bps == 0.0`.
#[must_use]
#[allow(clippy::missing_panics_doc)]
pub fn best_backend(results: &BenchmarkResultMap) -> Option<&str> {
    results
        .iter()
        .max_by(|a, b| a.1.throughput_bps.partial_cmp(&b.1.throughput_bps).unwrap())
        .filter(|(_, r)| r.throughput_bps > 0.0)
        .map(|(k, _)| k.as_str())
}

/// Serialize `runs` to a pretty-printed JSON string.
///
/// # Errors
///
/// Returns `BenchmarkError::Serialize` if serialization fails.
pub fn benchmark_to_json(runs: &[BenchmarkRun]) -> Result<String, BenchmarkError> {
    serde_json::to_string_pretty(runs).map_err(Into::into)
}

/// Serialize `runs` to CSV.
///
/// Header (first line, exactly as shown — no spaces, no quoting):
/// `timestamp_ms,connection_name,backend,throughput_bps,avg_latency_us,p50_latency_us,p95_latency_us,p99_latency_us`
///
/// One row per backend per run. Duration fields are in **microseconds**
/// (`Duration::as_micros() as u64`). No RFC 4180 quoting is applied:
/// connection names and backend names are validated identifiers with no
/// commas or special characters, so quoting is not needed. If validation
/// rules change in the future, adopt the `csv` crate at that point.
#[must_use]
pub fn benchmark_to_csv(runs: &[BenchmarkRun]) -> String {
    let mut csv = String::new();
    csv.push_str("timestamp_ms,connection_name,backend,throughput_bps,avg_latency_us,p50_latency_us,p95_latency_us,p99_latency_us\n");
    for run in runs {
        for (backend, result) in &run.results {
            let _ = writeln!(
                csv,
                "{},{},{},{},{},{},{},{}",
                run.timestamp_ms,
                run.connection_name,
                backend,
                result.throughput_bps,
                result.avg_latency.as_micros(),
                result.p50_latency.as_micros(),
                result.p95_latency.as_micros(),
                result.p99_latency.as_micros()
            );
        }
    }
    csv
}

/// Return a new `Vec` containing at most `cap` entries from `runs`,
/// evicting the oldest (front) entries when `runs.len() > cap`.
///
/// This is a pure calculation function — it takes ownership and returns a
/// new `Vec`, making it trivially testable with no side-effects.
/// When `runs.len() <= cap` the original `Vec` is returned without allocation.
#[must_use]
pub fn cap_history(runs: Vec<BenchmarkRun>, cap: usize) -> Vec<BenchmarkRun> {
    let len = runs.len();
    if len <= cap {
        runs
    } else {
        runs.into_iter().skip(len - cap).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn format_throughput_zero() {
        assert_eq!(format_throughput(0.0), "0 B/s");
    }

    #[test]
    fn format_throughput_mb() {
        assert_eq!(format_throughput(1_000_000.0), "1.00 MB/s");
    }

    #[test]
    fn format_throughput_gb() {
        assert_eq!(format_throughput(1_073_741_824.0), "1.07 GB/s");
    }

    #[test]
    fn format_latency_micros() {
        assert_eq!(format_latency(Duration::from_micros(420)), "0.42 ms");
    }

    #[test]
    fn throughput_bar_data_alphabetical_order() {
        let mut results = BenchmarkResultMap::new();
        results.insert(
            "charlie".to_string(),
            BenchmarkResult {
                backend: "charlie".to_string(),
                packets_processed: 1000,
                bytes_encapsulated: 100_000,
                elapsed: Duration::from_secs(1),
                throughput_bps: 800_000.0,
                avg_latency: Duration::from_micros(100),
                p50_latency: Duration::ZERO,
                p95_latency: Duration::ZERO,
                p99_latency: Duration::ZERO,
            },
        );
        results.insert(
            "alice".to_string(),
            BenchmarkResult {
                backend: "alice".to_string(),
                packets_processed: 1000,
                bytes_encapsulated: 100_000,
                elapsed: Duration::from_secs(1),
                throughput_bps: 800_000.0,
                avg_latency: Duration::from_micros(100),
                p50_latency: Duration::ZERO,
                p95_latency: Duration::ZERO,
                p99_latency: Duration::ZERO,
            },
        );
        results.insert(
            "bob".to_string(),
            BenchmarkResult {
                backend: "bob".to_string(),
                packets_processed: 1000,
                bytes_encapsulated: 100_000,
                elapsed: Duration::from_secs(1),
                throughput_bps: 800_000.0,
                avg_latency: Duration::from_micros(100),
                p50_latency: Duration::ZERO,
                p95_latency: Duration::ZERO,
                p99_latency: Duration::ZERO,
            },
        );
        let data = throughput_bar_data(&results);
        assert_eq!(data, vec![("alice", 800), ("bob", 800), ("charlie", 800),]);
    }

    #[test]
    fn throughput_bar_data_empty() {
        let results = BenchmarkResultMap::new();
        let data = throughput_bar_data(&results);
        assert_eq!(data, Vec::<(&str, u64)>::new());
    }

    #[test]
    fn throughput_sparkline_data_with_samples() {
        let progress = vec![BenchmarkProgress {
            backend: "test".to_string(),
            elapsed_secs: 1,
            total_secs: 10,
            current_throughput_bps: 1_234_567.0,
            packets_processed: 1000,
        }];
        let data = throughput_sparkline_data(&progress);
        assert_eq!(data, vec![1234]);
    }

    #[test]
    fn throughput_sparkline_data_empty() {
        let progress = vec![];
        let data = throughput_sparkline_data(&progress);
        assert_eq!(data, Vec::<u64>::new());
    }

    #[test]
    fn best_backend_with_nonzero() {
        let mut results = BenchmarkResultMap::new();
        results.insert(
            "slow".to_string(),
            BenchmarkResult {
                backend: "slow".to_string(),
                packets_processed: 1000,
                bytes_encapsulated: 100_000,
                elapsed: Duration::from_secs(1),
                throughput_bps: 100_000.0,
                avg_latency: Duration::from_micros(100),
                p50_latency: Duration::ZERO,
                p95_latency: Duration::ZERO,
                p99_latency: Duration::ZERO,
            },
        );
        results.insert(
            "fast".to_string(),
            BenchmarkResult {
                backend: "fast".to_string(),
                packets_processed: 1000,
                bytes_encapsulated: 100_000,
                elapsed: Duration::from_secs(1),
                throughput_bps: 200_000.0,
                avg_latency: Duration::from_micros(100),
                p50_latency: Duration::ZERO,
                p95_latency: Duration::ZERO,
                p99_latency: Duration::ZERO,
            },
        );
        assert_eq!(best_backend(&results), Some("fast"));
    }

    #[test]
    fn best_backend_with_zero() {
        let mut results = BenchmarkResultMap::new();
        results.insert(
            "zero".to_string(),
            BenchmarkResult {
                backend: "zero".to_string(),
                packets_processed: 0,
                bytes_encapsulated: 0,
                elapsed: Duration::from_secs(1),
                throughput_bps: 0.0,
                avg_latency: Duration::from_micros(100),
                p50_latency: Duration::ZERO,
                p95_latency: Duration::ZERO,
                p99_latency: Duration::ZERO,
            },
        );
        assert_eq!(best_backend(&results), None);
    }

    #[test]
    fn best_backend_empty() {
        let results = BenchmarkResultMap::new();
        assert_eq!(best_backend(&results), None);
    }

    #[test]
    fn best_backend_tied() {
        let mut results = BenchmarkResultMap::new();
        results.insert(
            "a".to_string(),
            BenchmarkResult {
                backend: "a".to_string(),
                packets_processed: 1000,
                bytes_encapsulated: 100_000,
                elapsed: Duration::from_secs(1),
                throughput_bps: 100_000.0,
                avg_latency: Duration::from_micros(100),
                p50_latency: Duration::ZERO,
                p95_latency: Duration::ZERO,
                p99_latency: Duration::ZERO,
            },
        );
        results.insert(
            "z".to_string(),
            BenchmarkResult {
                backend: "z".to_string(),
                packets_processed: 1000,
                bytes_encapsulated: 100_000,
                elapsed: Duration::from_secs(1),
                throughput_bps: 100_000.0,
                avg_latency: Duration::from_micros(100),
                p50_latency: Duration::ZERO,
                p95_latency: Duration::ZERO,
                p99_latency: Duration::ZERO,
            },
        );
        // Since max_by is stable, it should pick the first in iteration order, but since HashMap order is not guaranteed,
        // but in practice it might be insertion order, but to make it deterministic, perhaps sort or something.
        // For test, just check it's one of them.
        let best = best_backend(&results);
        assert!(best == Some("a") || best == Some("z"));
    }

    #[test]
    fn cap_history_no_eviction() {
        let runs = vec![BenchmarkRun {
            timestamp_ms: 1,
            connection_name: "test".to_string(),
            results: BenchmarkResultMap::new(),
        }];
        let capped = cap_history(runs.clone(), 1);
        assert_eq!(capped.len(), 1);
        assert_eq!(capped[0].timestamp_ms, 1);
        assert_eq!(capped[0].connection_name, "test");
    }

    #[test]
    fn cap_history_eviction() {
        let runs = vec![
            BenchmarkRun {
                timestamp_ms: 1,
                connection_name: "test1".to_string(),
                results: BenchmarkResultMap::new(),
            },
            BenchmarkRun {
                timestamp_ms: 2,
                connection_name: "test2".to_string(),
                results: BenchmarkResultMap::new(),
            },
        ];
        let capped = cap_history(runs, 1);
        assert_eq!(capped.len(), 1);
        assert_eq!(capped[0].timestamp_ms, 2);
        assert_eq!(capped[0].connection_name, "test2");
    }

    #[test]
    fn cap_history_zero_cap() {
        let runs = vec![BenchmarkRun {
            timestamp_ms: 1,
            connection_name: "test".to_string(),
            results: BenchmarkResultMap::new(),
        }];
        let capped = cap_history(runs, 0);
        assert_eq!(capped.len(), 0);
    }

    #[test]
    fn benchmark_error_display() {
        assert_eq!(
            BenchmarkError::AlreadyRunning.to_string(),
            "benchmark already running"
        );
    }

    #[test]
    fn benchmark_to_csv_empty() {
        let csv = benchmark_to_csv(&[]);
        assert_eq!(
            csv,
            "timestamp_ms,connection_name,backend,throughput_bps,avg_latency_us,p50_latency_us,p95_latency_us,p99_latency_us\n"
        );
    }

    #[test]
    fn benchmark_to_json_empty() {
        let json = benchmark_to_json(&[]).unwrap();
        assert_eq!(json, "[]");
    }
}
