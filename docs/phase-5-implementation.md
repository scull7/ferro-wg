# Phase 5: Performance Comparison — Implementation Plan

## Context

Phase 5 transforms the Compare tab from a static backend availability check into a full performance benchmarking and visualization tool. The daemon already supports switching backends mid-connection (via `DaemonCommand::SwitchBackend`), and the IPC includes a `Benchmark` command for throughput/latency tests. This phase adds the TUI layer: running benchmarks from the UI, displaying results as charts, and enabling side-by-side comparison across all three backends (boringtun, neptun, gotatun).

**Done when:** the Compare tab shows live-updating charts during benchmark runs, allows switching backends directly from the UI, stores historical results, and exports data as JSON/CSV — all without blocking the TUI.

---

## User Stories

| ID | User story | Acceptance criteria |
|----|------------|---------------------|
| US-1 | As a user I want to run throughput benchmarks from the TUI | Pressing `b` on the Compare tab starts a benchmark against the active connection; progress shown in real time |
| US-2 | As a user I want to see benchmark results as charts | Throughput, latency, and handshake times displayed as `BarChart` / `Sparkline` widgets updating live during the run |
| US-3 | As a user I want to compare backends side-by-side | Charts show all three backends simultaneously; current backend highlighted |
| US-4 | As a user I want to switch backends directly from the Compare tab | Pressing `1`/`2`/`3` switches to boringtun/neptun/gotatun and re-runs the benchmark if one is active |
| US-5 | As a user I want to view historical benchmark results | Results stored locally; press `h` to toggle between live/current and historical view |
| US-6 | As a user I want to export benchmark data | Pressing `e` opens export dialog; saves as JSON or CSV to user-specified path |

---

## Architecture

### Benchmark IPC (existing)

Reuse the `DaemonCommand::Benchmark` (already in IPC) which runs a configurable test (duration, packet size, concurrency) and returns `DaemonResponse::BenchmarkResult` with metrics.

### TUI State Extensions

- `AppState::benchmark_results: HashMap<BackendKind, BenchmarkResult>` — latest results per backend
- `AppState::benchmark_history: Vec<BenchmarkRun>` — historical runs with timestamp
- `AppState::benchmark_running: bool` — prevents multiple concurrent benchmarks
- `BenchmarkRun { timestamp: DateTime<Utc>, results: HashMap<BackendKind, BenchmarkResult> }`

### Async Benchmark Task

Spawn `tokio::spawn` on `Action::StartBenchmark`. Task sends `Benchmark` command, waits for result, dispatches `Action::BenchmarkResult(backend, result)`. If benchmark fails, dispatch `Action::BenchmarkError(e)`.

### Charts Using ratatui

- **Throughput:** `BarChart` showing packets/s and bytes/s for each backend
- **Latency:** `Sparkline` for p50/p95/p99 over time during the run
- **Handshake:** `BarChart` for initial/rekey times

Charts update live via `Component::update()` on `BenchmarkResult` actions.

### Backend Switching

`Action::SwitchBackend(backend)` dispatches to daemon, then optionally restarts benchmark if one was running.

### Historical Storage

Results saved to `~/.config/ferro-wg/benchmarks.json` as JSON. Loaded on TUI startup.

### Export

`Action::ExportBenchmarks(format, path)` spawns task to serialize and write file. Success/failure via `DaemonMessage`.

---

## Design Rationale

### Why Charts in TUI

Ratatui's `BarChart`/`Sparkline` widgets provide terminal-native visualization without external deps. For rich graphs, defer to Phase 7 (Sixel support).

### Benchmark Duration

Default 10s test (configurable in future Phase 7). Long enough for stable metrics but short enough for interactive feel.

### Concurrent Backends

Daemon does not support running multiple backends on the same connection simultaneously. Phase 5 switches backends sequentially during comparison — acceptable for "apples-to-apples" as long as test conditions are identical.

### Historical Data Scope

Store last 10 runs per connection. Simple JSON append; no database.

### Export Formats

JSON for full fidelity (retains all fields); CSV for spreadsheet import.

---

## Implementation Steps

### Step 1: Benchmark IPC Client and State

**Files:** `ferro-wg-tui-core/src/state.rs`, `ferro-wg-tui-core/src/action.rs`

- Add `BenchmarkResult`, `BenchmarkRun` structs
- Extend `AppState` with benchmark fields
- New actions: `StartBenchmark`, `BenchmarkResult`, `BenchmarkError`, `SwitchBackend`

### Step 2: Async Benchmark Task

**Files:** `ferro-wg-tui/src/lib.rs`

- Spawn benchmark task on `StartBenchmark`
- Handle result/error dispatch via `DaemonMessage`

### Step 3: Compare Component Charts

**Files:** `ferro-wg-tui-components/src/compare.rs`

- Replace static table with `BarChart`/`Sparkline` widgets
- Update on `BenchmarkResult` actions
- Keybindings: `b` start, `1`/`2`/`3` switch backend

### Step 4: Historical Storage

**Files:** `ferro-wg-tui/src/lib.rs`, `ferro-wg-tui-core/src/state.rs`

- Load/save `benchmarks.json` on startup/shutdown
- `h` key to toggle history view

### Step 5: Export Functionality

**Files:** `ferro-wg-tui/src/lib.rs`, `ferro-wg-tui-components/src/compare.rs`

- `e` key opens export dialog (reuse import path input pattern)
- Spawn export task; handle success/error feedback

---

## Testing Strategy

- Unit: Benchmark result parsing, chart rendering with mock data
- Integration: Full benchmark cycle against mock daemon; verify charts update and history saves

---

## Verification

Manual test: Run benchmark, switch backends, view charts, export CSV, verify history persists across restarts.

---

## Files Modified

| File | Changes |
|------|---------|
| `ferro-wg-tui-core/src/state.rs` | Benchmark structs, state fields |
| `ferro-wg-tui-core/src/action.rs` | New actions |
| `ferro-wg-tui/src/lib.rs` | Benchmark task, export task |
| `ferro-wg-tui-components/src/compare.rs` | Charts, keybindings |