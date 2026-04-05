# Phase 5: Performance Comparison — Implementation Progress

## Completed Phases

### Commit 1: IPC extension + benchmark types (COMPLETED)
- Added `p50_latency`, `p95_latency`, `p99_latency` to `BenchmarkResult` in `ferro-wg-core/src/stats.rs`
- Added `BenchmarkProgress` struct in `ferro-wg-core/src/ipc.rs`
- Added `DaemonCommand::Benchmark` and `DaemonResponse::BenchmarkProgress/BenchmarkResult` in IPC
- Added `send_streaming_command` in `ferro-wg-core/src/client.rs`
- New `ferro-wg-tui-core/src/benchmark.rs` module with pure functions: `BenchmarkResultMap`, `BenchmarkRun`, `BenchmarkError`, formatters, chart data builders, serialization
- Added benchmark actions in `ferro-wg-tui-core/src/action.rs`: `StartBenchmark`, `StartBenchmarkForBackend`, `BenchmarkProgressUpdate`, `BenchmarkComplete`, `SwitchBenchmarkBackend`, `ToggleCompareView`, `EnterExport`, `ExportKey`, `SubmitExport`, `ExitExport`
- Added `InputMode::Export(String)` in `ferro-wg-tui-core/src/app.rs`
- Added benchmark fields to `AppState` in `ferro-wg-tui-core/src/state.rs`: `benchmark_results`, `benchmark_history`, `benchmark_running`, `benchmark_progress_history`, `compare_view`
- All tests pass, clippy clean, no warnings

### Commit 2: CompareComponent live benchmark UI (COMPLETED)
- Replaced static placeholder table with dynamic Live/Historical layout
- Added key bindings: b (StartBenchmark), Enter (StartBenchmarkForBackend), w (SwitchBenchmarkBackend), h (ToggleCompareView), e (EnterExport)
- Implemented render_live: stacked table, BarChart, Sparkline, Gauge
- Implemented render_historical: scrollable BenchmarkRun list
- Updated status bar hints for Compare tab
- All tests pass, clippy clean, no warnings

### Commit 3: Background benchmark task (COMPLETED)
- Added DaemonMessage::BenchmarkProgress/BenchmarkComplete variants
- Extended handle_daemon_messages for benchmark dispatching
- Implemented spawn_benchmark_task with streaming IPC
- Add spawn_switch_backend_task for backend switching
- Introduced TuiError enum with thiserror for proper error handling
- Extended maybe_spawn_command with benchmark action arms
- All tests pass, clippy clean, no warnings

### Commit 4: Historical storage (COMPLETED)
- New ferro-wg-tui/src/history.rs module with load/save functions
- Thread benchmarks_path through TUI startup and event loop
- Load history at startup, save after BenchmarkComplete
- Cap history at 50 runs, graceful error handling
- All tests pass, clippy clean, no warnings

### Commit 5: Export functionality (COMPLETED)
- Added spawn_export_task for async CSV/JSON export to file
- Extension determines format (.csv → CSV, else JSON)
- Error handling for serialization and I/O failures, propagated via DaemonMessage
- Unit tests covering CSV/JSON export, file I/O success/failure, and error propagation
- All tests pass, clippy clean, no warnings

## Pending Phases

## Verification Status
- Tooling checks: PASSED (fmt, test, clippy, build)
- Adversary reviews: PASSED (reviewer, tester, architect)