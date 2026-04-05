//! Tunnel and peer statistics for monitoring and backend comparison.

use std::time::Duration;

/// Per-tunnel statistics snapshot.
///
/// Captured at a point in time from a running [`WgBackend`](crate::backend::WgBackend).
/// All counters are cumulative since tunnel creation.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TunnelStats {
    /// Bytes sent through the tunnel (encrypted, on the wire).
    pub tx_bytes: u64,
    /// Bytes received through the tunnel (encrypted, on the wire).
    pub rx_bytes: u64,
    /// Time since the last successful handshake, if any.
    pub last_handshake: Option<Duration>,
    /// Estimated packet loss ratio (0.0 = none, 1.0 = total loss).
    pub packet_loss: f32,
    /// Most recent session index, if a session is active.
    pub session_index: Option<u32>,
}

/// Aggregated comparison across multiple backend runs.
///
/// Used by the Compare TUI tab to display side-by-side benchmarks.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkResult {
    /// Which backend produced this result.
    pub backend: String,
    /// Packets processed during the benchmark window.
    pub packets_processed: u64,
    /// Total bytes encapsulated.
    pub bytes_encapsulated: u64,
    /// Wall-clock duration of the benchmark run.
    pub elapsed: Duration,
    /// Derived throughput in bytes per second.
    pub throughput_bps: f64,
    /// Average encapsulation latency per packet.
    pub avg_latency: Duration,
    /// Median (50th percentile) encapsulation latency.
    #[serde(default)]
    pub p50_latency: Duration,
    /// 95th-percentile encapsulation latency.
    #[serde(default)]
    pub p95_latency: Duration,
    /// 99th-percentile encapsulation latency.
    #[serde(default)]
    pub p99_latency: Duration,
}

impl BenchmarkResult {
    /// Compute throughput from `bytes_encapsulated` and `elapsed`.
    pub fn compute_throughput(&mut self) {
        let secs = self.elapsed.as_secs_f64();
        self.throughput_bps = if secs > 0.0 {
            #[allow(clippy::cast_precision_loss)]
            {
                self.bytes_encapsulated as f64 / secs
            }
        } else {
            0.0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_stats_default() {
        let stats = TunnelStats::default();
        assert_eq!(stats.tx_bytes, 0);
        assert_eq!(stats.rx_bytes, 0);
        assert!(stats.last_handshake.is_none());
        assert_eq!(stats.packet_loss.to_bits(), 0.0_f32.to_bits());
        assert!(stats.session_index.is_none());
    }

    #[test]
    fn benchmark_compute_throughput() {
        let mut result = BenchmarkResult {
            backend: "boringtun".into(),
            packets_processed: 1000,
            bytes_encapsulated: 1_000_000,
            elapsed: Duration::from_secs(1),
            throughput_bps: 0.0,
            avg_latency: Duration::from_micros(10),
            ..BenchmarkResult::default()
        };
        result.compute_throughput();
        assert!((result.throughput_bps - 1_000_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn benchmark_zero_elapsed() {
        let mut result = BenchmarkResult {
            elapsed: Duration::ZERO,
            bytes_encapsulated: 500,
            ..BenchmarkResult::default()
        };
        result.compute_throughput();
        assert_eq!(result.throughput_bps.to_bits(), 0.0_f64.to_bits());
    }

    #[test]
    fn tunnel_stats_serde_roundtrip() {
        let stats = TunnelStats {
            tx_bytes: 42_000,
            rx_bytes: 84_000,
            last_handshake: Some(Duration::from_secs(5)),
            packet_loss: 0.01,
            session_index: Some(7),
        };
        let json = serde_json::to_string(&stats).expect("serialize");
        let back: TunnelStats = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, stats);
    }
}
