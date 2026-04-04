# Preferred Rust Crates

Use this guide when selecting dependencies for a Rust project.

| Purpose | Crate(s) |
|---|---|
| Build / deps | `cargo` |
| Progress bars | `indicatif` |
| Serialization | `serde`, `serde_json` |
| TUI | `ratatui`, `crossterm` |
| HTTP API | `axum`, `tower` |
| Async runtime | `tokio` |
| CPU parallelism | `rayon` |
| Error types | `thiserror` |
| Secrets | `secrecy` |
| Env vars | `dotenvy` |
| Tabular data | `polars` |
| Logging | `tracing` |
| Human-readable durations | `humantime` |

Use `tracing::error!` / `log::error!` — never `println!` — for error reporting.
