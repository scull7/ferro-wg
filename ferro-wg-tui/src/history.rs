//! Historical benchmark persistence.
//!
//! This module provides async functions for loading and saving benchmark
//! history to/from a JSON file. It follows the calculation layer principles:
//! pure functions where possible, isolating I/O to async boundaries.

use std::path::Path;

use ferro_wg_tui_core::benchmark::{
    BENCHMARK_HISTORY_CAP, BenchmarkError, BenchmarkRun, cap_history,
};

/// Load benchmark history from `path`.
///
/// **Error semantics:**
/// - File does not exist → `Ok(vec![])` (first run, not an error).
/// - File exists but is empty or contains invalid JSON →
///   `Err(BenchmarkError::Serialize(_))` so the UI can warn the user.
/// - Any other I/O error → `Err(BenchmarkError::Io(_))`.
///
/// The file is read entirely into memory via `tokio::fs::read_to_string`.
/// Benchmark history files are small (50 × ~500 bytes = ~25 KB), so full
/// in-memory loading is safe and appropriate.
pub async fn load_benchmark_history(path: &Path) -> Result<Vec<BenchmarkRun>, BenchmarkError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // File doesn't exist: not an error, return empty history.
            return Ok(Vec::new());
        }
        Err(e) => return Err(BenchmarkError::Io(e)),
    };
    if content.trim().is_empty() {
        // Empty file: treat as invalid JSON to warn the user.
        return Err(BenchmarkError::Serialize(serde_json::Error::io(
            std::io::Error::new(std::io::ErrorKind::InvalidData, "file is empty"),
        )));
    }
    let runs: Vec<BenchmarkRun> = serde_json::from_str(&content)?;
    Ok(runs)
}

/// Persist `runs` (capped at `BENCHMARK_HISTORY_CAP`) to `path` as
/// pretty-printed JSON.
///
/// Parsing and serialization are synchronous calculations; only the
/// `fs::write` call is async. Calls `cap_history` internally before
/// serializing so the persisted file never exceeds the cap.
pub async fn save_benchmark_history(
    path: &Path,
    runs: Vec<BenchmarkRun>,
) -> Result<(), BenchmarkError> {
    let capped_runs = cap_history(runs, BENCHMARK_HISTORY_CAP);
    let json = serde_json::to_string_pretty(&capped_runs)?;
    tokio::fs::write(path, json).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_wg_tui_core::benchmark::BenchmarkResultMap;
    use std::time::Duration;
    use tempfile::tempdir;

    #[tokio::test]
    async fn load_benchmark_history_nonexistent_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        let result = load_benchmark_history(&path).await;
        assert_eq!(result.unwrap(), Vec::<BenchmarkRun>::new());
    }

    #[tokio::test]
    async fn load_benchmark_history_valid_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");
        let run = BenchmarkRun {
            timestamp_ms: 1_000_000,
            connection_name: "test".to_string(),
            results: BenchmarkResultMap::new(),
        };
        let json = serde_json::to_string_pretty(&vec![run.clone()]).unwrap();
        tokio::fs::write(&path, json).await.unwrap();

        let loaded = load_benchmark_history(&path).await.unwrap();
        assert_eq!(loaded, vec![run]);
    }

    #[tokio::test]
    async fn load_benchmark_history_empty_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.json");
        tokio::fs::write(&path, "").await.unwrap();

        let result = load_benchmark_history(&path).await;
        assert!(matches!(result, Err(BenchmarkError::Serialize(_))));
    }

    #[tokio::test]
    async fn save_benchmark_history_capped() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");

        let runs: Vec<_> = (0..55)
            .map(|i| BenchmarkRun {
                timestamp_ms: i64::from(i),
                connection_name: format!("test{i}"),
                results: BenchmarkResultMap::new(),
            })
            .collect();

        save_benchmark_history(&path, runs).await.unwrap();

        let loaded = load_benchmark_history(&path).await.unwrap();
        assert_eq!(loaded.len(), 50);
        assert_eq!(loaded[0].timestamp_ms, 5); // oldest evicted, starts from 5
        assert_eq!(loaded[49].timestamp_ms, 54); // newest
    }

    #[tokio::test]
    async fn roundtrip_with_latencies() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");

        let mut results = BenchmarkResultMap::new();
        results.insert(
            "boringtun".to_string(),
            ferro_wg_core::stats::BenchmarkResult {
                backend: "boringtun".to_string(),
                packets_processed: 1000,
                bytes_encapsulated: 100_000,
                elapsed: Duration::from_secs(1),
                throughput_bps: 100_000.0,
                avg_latency: Duration::from_micros(100),
                p50_latency: Duration::from_micros(90),
                p95_latency: Duration::from_micros(150),
                p99_latency: Duration::from_micros(200),
            },
        );

        let run = BenchmarkRun {
            timestamp_ms: 1_000_000,
            connection_name: "test".to_string(),
            results,
        };

        save_benchmark_history(&path, vec![run.clone()])
            .await
            .unwrap();
        let loaded = load_benchmark_history(&path).await.unwrap();

        assert_eq!(loaded.len(), 1);
        let loaded_run = &loaded[0];
        assert_eq!(loaded_run, &run);
        let result = loaded_run.results.get("boringtun").unwrap();
        assert_eq!(result.p50_latency, Duration::from_micros(90));
        assert_eq!(result.p95_latency, Duration::from_micros(150));
        assert_eq!(result.p99_latency, Duration::from_micros(200));
    }
}
