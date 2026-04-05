# Phase 5: Performance Comparison — Implementation Plan

## Pre-implementation checklist

Before writing any Rust code, invoke `/rust-code-writer` (per `CLAUDE.md`).

- Confirm `ferro_wg_core::stats::BenchmarkResult` compiles with the new
  `p50_latency`, `p95_latency`, `p99_latency` fields added — existing tests
  in `stats.rs` use struct-literal syntax and must be updated to supply
  `..BenchmarkResult::default()` for the new fields. All three new fields
  carry `#[serde(default)]` so legacy JSON without these keys deserializes
  cleanly to `Duration::ZERO`.
- Confirm `BenchmarkProgress` is placed in `ferro-wg-core/src/ipc.rs`
  (wire-format type, sits with `DaemonCommand`/`DaemonResponse`) — **not**
  in `stats.rs` (which holds post-processed aggregation results only).
- Confirm `AppState::benchmark_progress_history` is `VecDeque<BenchmarkProgress>`
  (not `Option<BenchmarkProgress>`) — the `Sparkline` widget needs a slice of
  samples, not a single latest value. The `VecDeque` is capped at 60 entries
  in `dispatch(BenchmarkProgressUpdate)`.
- Confirm `DaemonCommand::Benchmark` and `DaemonResponse::BenchmarkResult` /
  `DaemonResponse::BenchmarkProgress` are added **before** any TUI code
  references them — `ferro-wg-core` must compile cleanly first.
- Confirm `benchmarks_path` is threaded as a `&Path` / `PathBuf` parameter
  through `handle_key_event` → `maybe_spawn_command` → background tasks,
  exactly like `config_path` — it is **not** added to `AppState`.
- Confirm `BenchmarkResultMap = HashMap<String, BenchmarkResult>` is a type
  alias in `ferro-wg-tui-core/src/benchmark.rs`, not a newtype — backends
  are keyed by the `backend: String` field already present in `BenchmarkResult`.
- Confirm `1`/`2`/`3` keys are consumed by `handle_global_key` as
  `SelectTab` — these **cannot** be used for backend switching; use `Enter`
  (run benchmark for selected row) and `w` (switch connection backend) instead.
- Confirm `tokio::fs::write` is used in all async export/save tasks — no
  `std::fs` blocking I/O inside `tokio::spawn`.
- Confirm `InputMode::Export(String)` is routed through
  `bundle.status_bar.handle_key` alongside `Import(String)`, matching the
  existing routing guard in `handle_key_event`.

---

## Context

Phase 4 delivered full connection lifecycle management — users can bring
tunnels up/down in bulk, import new configs, and control the daemon — all
from the TUI. The Compare tab exists as a static 3-row table showing backend
availability (yes/no) with placeholder dashes for all performance columns.
The `BenchmarkResult` type exists in `ferro-wg-core/src/stats.rs` and
`BenchmarkResult::compute_throughput` already derives `throughput_bps`; the
daemon has `DaemonCommand::SwitchBackend` for mid-connection backend changes.
What is missing: IPC commands to trigger benchmarks, live progress streaming,
state fields to accumulate results, chart widgets to visualise them, and
persistence for historical runs.

Phase 5 completes the Compare tab end-to-end. The daemon owns the backend and
therefore runs the benchmark: the TUI sends `DaemonCommand::Benchmark` and
receives a stream of `DaemonResponse::BenchmarkProgress` updates followed by
a final `DaemonResponse::BenchmarkResult`. The TUI displays a `BarChart` for
side-by-side throughput comparison and a `Sparkline` for live latency during
the run. Results are persisted to a `benchmarks.json` file (capped at 50
runs) and can be exported on demand as JSON or CSV.

**Design is strictly stratified into three layers.** The _calculation layer_
(`ferro-wg-tui-core/src/benchmark.rs`) contains only pure functions — format
helpers, chart data builders, serialization — with no `AppState` dependency.
The _state layer_ (`ferro-wg-tui-core/src/state.rs`) holds benchmark fields
and dispatch arms for all benchmark actions; it performs no I/O. The _action /
effect layer_ (`ferro-wg-tui/src/lib.rs`) spawns background tasks, owns file
I/O, and sends IPC commands.

**Done when:** the Compare tab shows a live-updating `BarChart` and `Sparkline`
during benchmark runs, allows switching backends directly from the tab,
persists up to 50 historical runs that survive TUI restarts, and exports data
as JSON or CSV to a user-specified path — all without blocking the event loop.

---

## User Stories

| ID   | User story | Acceptance criteria |
|------|------------|---------------------|
| US-1 | As a user I want to start a benchmark from the TUI | Pressing `b` on the Compare tab sends `DaemonCommand::Benchmark` for the active connection; `benchmark_running` flips to `true`; pressing `b` again while running does nothing (shows inline error) |
| US-2 | As a user I want live progress during a benchmark | `BenchmarkProgress` updates arrive from the daemon every second; the `Sparkline` and progress bar re-render each tick; elapsed / total seconds shown in the panel title |
| US-3 | As a user I want to see final results as charts | After the run, the `BarChart` shows throughput for all three backends side-by-side; the detail panel shows p50/p95/p99 latency; current backend row is highlighted |
| US-4 | As a user I want to run a benchmark for a specific backend | Pressing `Enter` on a highlighted row sends `DaemonCommand::Benchmark` scoped to that row's backend name; the same progress / result flow applies |
| US-5 | As a user I want to switch my connection to a backend from the Compare tab | Pressing `w` on a highlighted row sends `DaemonCommand::SwitchBackend` for the active connection; a `DaemonOk` feedback message confirms |
| US-6 | As a user I want to toggle between live and historical views | Pressing `h` cycles `CompareView` between `Live` and `Historical`; the Historical view renders a scrollable list of past `BenchmarkRun` entries with timestamps |
| US-7 | As a user I want historical results to persist across restarts | `benchmarks.json` is loaded at TUI startup; a new run is appended and saved on `BenchmarkComplete`; the file is capped at 50 runs (oldest evicted) |
| US-8 | As a user I want to export benchmark data | Pressing `e` opens an export path prompt (same UI pattern as import); submitting with a `.json` extension writes JSON; `.csv` writes CSV; `Esc` cancels |

---

## Architecture

### Existing infrastructure to reuse

```
ferro_wg_core::stats::BenchmarkResult        ← extend with percentile fields (stats.rs)
ferro_wg_core::ipc::DaemonCommand            ← add Benchmark variant (ipc.rs)
ferro_wg_core::ipc::DaemonResponse           ← add BenchmarkResult + BenchmarkProgress variants (ipc.rs)
DaemonCommand::SwitchBackend                 ← reuse for `w` key backend switch (already in ipc.rs)
AppState::feedback / dispatch()              ← reuse Feedback::error for "already running"
InputMode::Import(String)                    ← model InputMode::Export(String) after this (app.rs)
spawn_import_task pattern                    ← model spawn_benchmark_task + spawn_export_task after this (lib.rs)
DaemonMessage enum (private, lib.rs)         ← add BenchmarkProgress + BenchmarkComplete variants
maybe_spawn_command / dispatch_all           ← existing wiring; add benchmark arms
CompareComponent::table_state: TableState   ← reuse existing row selection for benchmark target
handle_key_event routing chain               ← Export(String) added to the InputMode guard
config_path: &Path threading pattern         ← benchmarks_path threaded identically
```

### Stratified layer design

Three layers — never mix them:

1. **Calculation layer** — `ferro-wg-tui-core/src/benchmark.rs`
   Pure functions operating on immutable data. No `AppState`. No I/O. No
   `tokio`. Trivially unit-testable in isolation.

2. **State layer** — `ferro-wg-tui-core/src/state.rs`
   `AppState` fields that accumulate benchmark data. `dispatch()` arms that
   mutate state in response to actions. No I/O, no IPC, no task spawning.

3. **Action/effect layer** — `ferro-wg-tui/src/lib.rs`
   `spawn_benchmark_task`, `spawn_export_task`. All file I/O (`tokio::fs::write`).
   All IPC calls. Translates `DaemonResponse` to `DaemonMessage`; wires
   `DaemonMessage` back to state dispatch via `handle_daemon_messages`.

### New types

```rust
// ferro-wg-core/src/stats.rs  (additions to BenchmarkResult only)

/// Aggregated comparison across multiple backend runs.
///
/// `backend` is kept as `String` (not `BackendKind`) to avoid breaking
/// previously-serialised JSON files. `compute_throughput` must be called
/// after deserialization if `throughput_bps` is not stored.
///
/// **Migration note:** three new fields are added. Any existing struct-literal
/// construction in tests must append `..BenchmarkResult::default()` or supply
/// explicit values for `p50_latency`, `p95_latency`, `p99_latency`. The
/// `Default` derive supplies `Duration::ZERO` for all three, which is a safe
/// sentinel for legacy data loaded from JSON without these fields (they will
/// deserialize as `Duration::ZERO` via `#[serde(default)]`).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkResult {
    pub backend: String,
    pub packets_processed: u64,
    pub bytes_encapsulated: u64,
    pub elapsed: Duration,
    pub throughput_bps: f64,
    pub avg_latency: Duration,
    // -- NEW in Phase 5 --
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
```

```rust
// ferro-wg-core/src/ipc.rs  (BenchmarkProgress — wire-format type lives here,
// alongside the DaemonCommand/DaemonResponse variants that reference it)

/// Periodic benchmark progress update streamed from the daemon.
///
/// `BenchmarkProgress` is a wire-format type: it travels over the IPC socket
/// from daemon to TUI as a `DaemonResponse::BenchmarkProgress` variant.
/// It lives in `ipc.rs` alongside the other wire types, **not** in `stats.rs`
/// (which holds post-processed aggregation results).
///
/// Sent roughly once per second during an active `DaemonCommand::Benchmark`
/// run. The TUI drives the `Sparkline` and progress bar from these updates.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkProgress {
    /// Backend under test.
    pub backend: String,
    /// Seconds elapsed since the benchmark started.
    pub elapsed_secs: u32,
    /// Configured total duration.
    pub total_secs: u32,
    /// Instantaneous throughput at this sample point.
    pub current_throughput_bps: f64,
    /// Cumulative packets processed so far.
    pub packets_processed: u64,
}
```

```rust
// ferro-wg-tui-core/src/benchmark.rs  (new module)

use std::collections::HashMap;
use std::time::Duration;
// BenchmarkProgress is a wire-format type (lives in ipc.rs); BenchmarkResult
// is a post-processed aggregation type (lives in stats.rs).
use ferro_wg_core::ipc::BenchmarkProgress;
use ferro_wg_core::stats::BenchmarkResult;

/// Maximum number of live progress samples kept in `AppState::benchmark_progress_history`.
///
/// At one sample per second this gives one minute of sparkline history.
pub const BENCHMARK_PROGRESS_HISTORY_CAP: usize = 60;

/// Latest benchmark result keyed by backend name string.
pub type BenchmarkResultMap = HashMap<String, BenchmarkResult>;

/// A single historical benchmark run with a wall-clock timestamp.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
pub fn format_throughput(bps: f64) -> String { ... }

/// Format a `Duration` as milliseconds with two decimal places.
///
/// `Duration::from_micros(420)` → `"0.42 ms"`
pub fn format_latency(d: Duration) -> String { ... }

/// Format a `Duration` as a whole-seconds string.
///
/// `Duration::from_secs(10)` → `"10s"`
pub fn format_duration(d: Duration) -> String { ... }

/// Build `BarChart` data from a `BenchmarkResultMap`.
///
/// Returns entries in stable alphabetical order by backend name so the
/// chart order does not depend on `HashMap` iteration order.
/// Values are throughput in kilobytes per second (truncated to `u64`).
pub fn throughput_bar_data<'a>(results: &'a BenchmarkResultMap) -> Vec<(&'a str, u64)> { ... }

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
pub fn throughput_sparkline_data(progress: &[BenchmarkProgress]) -> Vec<u64> { ... }

/// Return the backend name with the highest `throughput_bps`.
///
/// Returns `None` when `results` is empty or all backends have
/// `throughput_bps == 0.0`.
pub fn best_backend(results: &BenchmarkResultMap) -> Option<&str> { ... }

/// Serialize `runs` to a pretty-printed JSON string.
///
/// # Errors
///
/// Returns `BenchmarkError::Serialize` if serialization fails.
pub fn benchmark_to_json(runs: &[BenchmarkRun]) -> Result<String, BenchmarkError> { ... }

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
pub fn benchmark_to_csv(runs: &[BenchmarkRun]) -> String { ... }

/// Return a new `Vec` containing at most `cap` entries from `runs`,
/// evicting the oldest (front) entries when `runs.len() > cap`.
///
/// This is a pure calculation function — it takes ownership and returns a
/// new `Vec`, making it trivially testable with no side-effects.
/// When `runs.len() <= cap` the original `Vec` is returned without allocation.
pub fn cap_history(runs: Vec<BenchmarkRun>, cap: usize) -> Vec<BenchmarkRun> { ... }
```

```rust
// ferro-wg-tui-core/src/app.rs  (additions)

pub enum InputMode {
    Normal,
    Search,
    /// Typing an import file path.
    Import(String),
    // -- NEW in Phase 5 --
    /// Typing an export file path. Inner `String` is the current buffer.
    Export(String),
}

// ferro-wg-tui-core/src/state.rs  (new free types used by AppState)

/// Which view mode is active on the Compare tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompareView {
    /// Show live benchmark results and the running progress widgets.
    #[default]
    Live,
    /// Show the scrollable list of historical `BenchmarkRun` entries.
    Historical,
}
```

### Error handling

All benchmark-related errors in `ferro-wg-tui-core` go through `BenchmarkError`
(defined above). The action/effect layer in `ferro-wg-tui/src/lib.rs` maps
`BenchmarkError` → `DaemonMessage::CommandError(e.to_string())` at task
boundaries so the `From` impl handles propagation cleanly:

```rust
// In spawn_benchmark_task / spawn_export_task:
if let Err(e) = result {
    let _ = daemon_tx.send(DaemonMessage::CommandError(e.to_string()));
}
```

No `anyhow` anywhere. No `.map_err(|e| ...)` at call sites — `?` propagates
through `From` impls.

### New `DaemonCommand` and `DaemonResponse` variants

```rust
// ferro-wg-core/src/ipc.rs  (additions)

pub enum DaemonCommand {
    // ... existing variants unchanged ...

    /// Run a performance benchmark against the named connection.
    ///
    /// The daemon streams [`DaemonResponse::BenchmarkProgress`] updates
    /// approximately once per second, then sends a final
    /// [`DaemonResponse::BenchmarkResult`] when the run completes.
    Benchmark {
        /// Which connection to benchmark.
        connection_name: String,
        /// Benchmark duration in seconds (default: 10).
        duration_secs: u32,
    },
}

pub enum DaemonResponse {
    // ... existing variants unchanged ...

    /// Periodic progress update during an active benchmark run.
    BenchmarkProgress(BenchmarkProgress),

    /// Final aggregated result after a benchmark run completes.
    BenchmarkResult(BenchmarkResult),
}
```

### New `Action` variants

```rust
// ferro-wg-tui-core/src/action.rs  (additions)

// -- Benchmark actions --

/// Start a benchmark for the active connection (all backends sequentially).
///
/// Blocked when `AppState::benchmark_running` is `true`; emits
/// `DaemonError("benchmark already running")` instead.
StartBenchmark,

/// Start a benchmark scoped to the named backend.
///
/// Emitted when the user presses `Enter` on a specific backend row.
StartBenchmarkForBackend(String),

/// Forward a live progress update from the daemon to `AppState`.
BenchmarkProgressUpdate(BenchmarkProgress),

/// A benchmark run completed; store results and persist history.
BenchmarkComplete(BenchmarkResult),

/// Switch the active connection to the named backend.
///
/// Emitted when the user presses `w` on a backend row.
/// Delegates to `DaemonCommand::SwitchBackend`.
SwitchBenchmarkBackend(String),

/// Toggle `AppState::compare_view` between `Live` and `Historical`.
ToggleCompareView,

// -- Export actions --

/// Enter export path input mode (opens the path prompt in the status bar).
EnterExport,

/// Forward a key event to the export path buffer.
ExportKey(KeyEvent),

/// Submit the current export path for processing.
SubmitExport,

/// Cancel export and return to `InputMode::Normal`.
ExitExport,
```

### New `AppState` fields

```rust
// ferro-wg-tui-core/src/state.rs  (additions to AppState)

/// Latest benchmark result per backend name for the **current active connection**.
///
/// Keyed by `BenchmarkResult::backend` (a `String`).
/// Cleared (set to empty `HashMap`) in `dispatch(StartBenchmark)` and in
/// `dispatch(StartBenchmarkForBackend(_))` so stale results from a previous
/// run never appear next to results from a new run. Each individual
/// `BenchmarkComplete` result is inserted by backend key, so a partial
/// all-backends run accumulates results incrementally.
pub benchmark_results: BenchmarkResultMap,

/// Benchmark history, capped at 50 runs; loaded from `benchmarks.json`
/// at startup and appended to on `BenchmarkComplete`.
pub benchmark_history: Vec<BenchmarkRun>,

/// `true` while a benchmark task is running; prevents concurrent runs.
///
/// Set to `true` in `dispatch(StartBenchmark)` **only when not already
/// running**. Set back to `false` in `dispatch(BenchmarkComplete)`.
/// The action/effect layer's `maybe_spawn_command` calls
/// `spawn_benchmark_task` when it sees `StartBenchmark` AND
/// `state.benchmark_running` is still `false` at that point (checked
/// against pre-dispatch state captured before `dispatch_all`).
pub benchmark_running: bool,

/// Ring buffer of live progress samples from the current benchmark run.
///
/// `VecDeque` is used so the oldest sample can be dropped from the front
/// in O(1) when the buffer is capped (maximum 60 samples — one minute of
/// one-per-second updates). Cleared to empty on `BenchmarkComplete` and on
/// `StartBenchmark`.
///
/// The `Sparkline` and `Gauge` widgets are driven from this field via
/// `benchmark::latency_sparkline_data(&state.benchmark_progress_history)`.
pub benchmark_progress_history: VecDeque<BenchmarkProgress>,

/// Which view mode is active on the Compare tab.
pub compare_view: CompareView,
```

`benchmarks_path: PathBuf` is threaded as a parameter — not added to
`AppState`. Default: `config_dir.join("benchmarks.json")`. This follows the
same pattern as `config_path`.

### Private `DaemonMessage` additions

```rust
// ferro-wg-tui/src/lib.rs  (additions to private enum)

enum DaemonMessage {
    StatusUpdate(Vec<PeerStatus>),
    CommandOk(String),
    CommandError(String),
    Unreachable,
    ReloadConfig(AppConfig, String),
    // -- NEW in Phase 5 --
    /// Live progress update from a running benchmark.
    BenchmarkProgress(BenchmarkProgress),
    /// A benchmark run completed successfully.
    BenchmarkComplete(BenchmarkResult),
}
```

### `CompareComponent` design

`CompareComponent` gains a richer layout split into two vertical panes when
the terminal is wide enough (≥ 80 columns): a **chart pane** (left, 60%) and
a **detail pane** (right, 40%). The existing `table_state: TableState` is
reused for backend row selection — no new selection field is introduced.

```rust
// ferro-wg-tui-components/src/compare.rs

/// Backend performance comparison tab.
///
/// Renders either the Live view (BarChart + Sparkline + progress bar) or
/// the Historical view (scrollable run list) depending on
/// `state.compare_view`.
pub struct CompareComponent {
    /// Existing row selection — reused as benchmark target selector.
    table_state: TableState,
}

impl Component for CompareComponent {
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down  => Some(Action::NextRow),
            KeyCode::Char('k') | KeyCode::Up    => Some(Action::PrevRow),
            // Always emit StartBenchmark; AppState::dispatch guards benchmark_running
            // and sets feedback "benchmark already running" without spawning a task.
            // Using Action::DaemonError here would be wrong — this error originates
            // locally, not from the daemon. Components stay dumb; state enforces policy.
            KeyCode::Char('b')  => Some(Action::StartBenchmark),
            KeyCode::Enter      => self.start_benchmark_for_selected(state),
            KeyCode::Char('w')  => self.switch_backend_for_selected(state),
            KeyCode::Char('h')  => Some(Action::ToggleCompareView),
            KeyCode::Char('e')  => Some(Action::EnterExport),
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, _state: &AppState) {
        // existing NextRow / PrevRow / SelectTab reset logic unchanged
        ...
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, focused: bool, state: &AppState) {
        match state.compare_view {
            CompareView::Live      => self.render_live(frame, area, focused, state),
            CompareView::Historical => self.render_historical(frame, area, state),
        }
    }
}
```

**`render_live`** layout (stacked vertically):
1. Backend table (5 rows + header) — same columns as before but now populated
   from `state.benchmark_results`.
2. `BarChart` (6 rows) — throughput in kBps per backend, data from
   `benchmark::throughput_bar_data(&state.benchmark_results)`.
3. `Sparkline` (3 rows) — live throughput samples, data from
   `benchmark::throughput_sparkline_data(state.benchmark_progress_history.make_contiguous())`
   where `benchmark_progress_history: VecDeque<BenchmarkProgress>` holds up
   to `BENCHMARK_PROGRESS_HISTORY_CAP` (60) one-per-second samples from the
   current run. Values are kBps (each sample: `current_throughput_bps / 1000`
   truncated to `u64`).
4. `Gauge` progress bar (1 row) — visible only when `benchmark_running`;
   ratio derived from `state.benchmark_progress_history.back()` (the most
   recent sample): `elapsed_secs as f64 / total_secs as f64`. Renders as 0%
   until the first progress update arrives.

**`render_historical`** renders a scrollable `List` of `BenchmarkRun` entries
from `state.benchmark_history`, newest first, with timestamp and best-backend
annotation from `benchmark::best_backend`.

### Key routing changes

The routing guard in `handle_key_event` gains one new `InputMode` arm:

```rust
// ferro-wg-tui/src/lib.rs

let action = if state.pending_confirm.is_some() {
    bundle.confirm_dialog.handle_key(key, state)
} else if matches!(
    state.input_mode,
    InputMode::Search | InputMode::Import(_) | InputMode::Export(_)  // ← added Export
) {
    bundle.status_bar.handle_key(key, state)
} else {
    handle_global_key(key)
        .or_else(|| bundle.connection_bar.handle_key(key, state))
        .or_else(|| bundle.tabs[state.active_tab.index()].handle_key(key, state))
};
```

`SubmitExport` capture (parallel to `SubmitImport`):

```rust
let export_path = if matches!(action, Action::SubmitExport) {
    state.export_buffer().map(PathBuf::from)
} else {
    None
};

// ... dispatch_all + maybe_spawn_command ...

if let Some(path) = export_path {
    spawn_export_task(path, &state.benchmark_history.clone(), daemon_tx, tasks);
}
```

### New `client::send_streaming_command` API (Commit 1)

The daemon client (`ferro-wg-core/src/client.rs`) currently has `send_command`
which sends one request and receives one response. Benchmark needs a streaming
variant that can receive multiple responses for a single command.

```rust
// ferro-wg-core/src/client.rs  (new function — add in Commit 1)

/// Send a command to the daemon and return a stream of responses.
///
/// The stream yields `DaemonResponse` items until the connection closes or
/// a terminal response (`BenchmarkResult`, `Error`) is received. The caller
/// is responsible for terminating iteration on terminal variants.
///
/// # Errors
///
/// Returns an error if the socket cannot be reached or the initial command
/// fails to serialize.
pub async fn send_streaming_command(
    cmd: DaemonCommand,
) -> Result<impl Stream<Item = DaemonResponse>, DaemonClientError> { ... }
```

`DaemonClientError` is the existing error type used by `send_command`.
Internally, `send_streaming_command` opens the same Unix socket, serializes
`cmd` once, then loops reading framed responses until the stream closes.

### Background task pseudo-code

```rust
// ferro-wg-tui/src/lib.rs

/// Spawn an async task that sends a `DaemonCommand::Benchmark` IPC request,
/// relays periodic `BenchmarkProgress` updates, and sends `BenchmarkComplete`
/// when the run finishes.
fn spawn_benchmark_task(
    connection_name: String,
    duration_secs: u32,
    daemon_tx: mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    tasks.spawn(async move {
        let cmd = DaemonCommand::Benchmark { connection_name, duration_secs };
        let mut stream = match client::send_streaming_command(cmd).await {
            Ok(s) => s,
            Err(e) => {
                let _ = daemon_tx.send(error_to_message(&e));
                return;
            }
        };
        while let Some(response) = stream.next().await {
            match response {
                DaemonResponse::BenchmarkProgress(p) => {
                    let _ = daemon_tx.send(DaemonMessage::BenchmarkProgress(p));
                }
                DaemonResponse::BenchmarkResult(r) => {
                    let _ = daemon_tx.send(DaemonMessage::BenchmarkComplete(r));
                    return;
                }
                DaemonResponse::Error(e) => {
                    let _ = daemon_tx.send(DaemonMessage::CommandError(e));
                    return;
                }
                _ => {}
            }
        }
    });
}

/// Spawn an async task that serialises `runs` and writes the result to `path`.
///
/// Extension determines format: `.csv` → CSV; anything else → JSON.
fn spawn_export_task(
    path: PathBuf,
    runs: Vec<BenchmarkRun>,
    daemon_tx: mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    tasks.spawn(async move {
        let result: Result<(), BenchmarkError> = async {
            let content = match path.extension().and_then(|e| e.to_str()) {
                Some("csv") => benchmark_to_csv(&runs),
                _ => benchmark_to_json(&runs)?,
            };
            tokio::fs::write(&path, content).await?;
            Ok(())
        }.await;
        let msg = match result {
            Ok(()) => DaemonMessage::CommandOk(
                format!("exported to {}", path.display())
            ),
            Err(e) => DaemonMessage::CommandError(e.to_string()),
        };
        let _ = daemon_tx.send(msg);
    });
}

/// Spawn an async task that saves `runs` (capped at 50) to `benchmarks_path`.
fn spawn_save_history_task(
    benchmarks_path: PathBuf,
    runs: Vec<BenchmarkRun>,
    daemon_tx: mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    tasks.spawn(async move {
        let result: Result<(), BenchmarkError> = async {
            let json = benchmark_to_json(&runs)?;
            tokio::fs::write(&benchmarks_path, json).await?;
            Ok(())
        }.await;
        if let Err(e) = result {
            let _ = daemon_tx.send(DaemonMessage::CommandError(
                format!("failed to save benchmark history: {e}")
            ));
        }
    });
}
```

`handle_daemon_messages` dispatch additions:

```rust
DaemonMessage::BenchmarkProgress(p) => {
    vec![Action::BenchmarkProgressUpdate(p)]
}
DaemonMessage::BenchmarkComplete(r) => {
    vec![Action::BenchmarkComplete(r)]
}
```

After dispatching `Action::BenchmarkComplete`, the event loop clones the
updated `state.benchmark_history` and calls `spawn_save_history_task` to
persist it asynchronously.

### Key `AppState::dispatch` arms (pseudocode)

```rust
// In ferro-wg-tui-core/src/state.rs AppState::dispatch

Action::StartBenchmark | Action::StartBenchmarkForBackend(_) => {
    if self.benchmark_running {
        // Guard: swallow silently and surface feedback. No task spawned.
        self.feedback = Some(Feedback::error("benchmark already running"));
        return vec![];   // action effect layer sees no signal to spawn
    }
    self.benchmark_running = true;
    self.benchmark_results.clear();
    self.benchmark_progress_history.clear();
    vec![]
}

Action::BenchmarkProgressUpdate(p) => {
    // Ring buffer: pop oldest when at cap, then push newest.
    if self.benchmark_progress_history.len() >= BENCHMARK_PROGRESS_HISTORY_CAP {
        self.benchmark_progress_history.pop_front();
    }
    self.benchmark_progress_history.push_back(p);
    vec![]
}

Action::BenchmarkComplete(result) => {
    self.benchmark_running = false;
    self.benchmark_results.insert(result.backend.clone(), result.clone());
    let run = BenchmarkRun {
        timestamp_ms: /* chrono::Utc::now().timestamp_millis() */ ...,
        connection_name: self.active_connection_name().unwrap_or_default(),
        results: self.benchmark_results.clone(),
    };
    self.benchmark_history.push(run);
    // cap_history takes ownership and returns a new Vec (pure function).
    self.benchmark_history = cap_history(
        std::mem::take(&mut self.benchmark_history),
        BENCHMARK_HISTORY_CAP,   // 50
    );
    // Signal the effect layer to persist history + show success feedback.
    // (effect layer checks for BenchmarkComplete in the action list and
    //  calls spawn_save_history_task with the updated history clone.)
    vec![]
}
```

`BENCHMARK_HISTORY_CAP` is a second named constant in `benchmark.rs`:

```rust
/// Maximum historical benchmark runs to keep in `benchmarks.json`.
pub const BENCHMARK_HISTORY_CAP: usize = 50;
```

**Effect layer guard for `maybe_spawn_command`:** the action/effect layer
checks `action == Action::StartBenchmark || action == StartBenchmarkForBackend`
and **also** `state.benchmark_running` **in the pre-dispatch snapshot** — if
`benchmark_running` was already `true` before dispatch, dispatch set feedback
and returned early; if it was `false`, dispatch just flipped it to `true`.
So the effect layer can safely check: if the post-dispatch `state.benchmark_running`
is `true` AND the action was `StartBenchmark`/`StartBenchmarkForBackend`, spawn
the task (no double-spawn possible because dispatch cleared `benchmark_results`
and returned `false` in the guard case).

### Save/persist flow — step by step

| Step | Operation | Error case | User-visible message |
|------|-----------|------------|----------------------|
| 1 | User presses `b`; `StartBenchmark` dispatched | `benchmark_running == true` | inline feedback: "benchmark already running" |
| 2 | `maybe_spawn_command` calls `spawn_benchmark_task` | daemon unreachable | `DaemonMessage::Unreachable` → "daemon is not running" |
| 3 | Daemon sends `BenchmarkProgress` (×N) | IPC stream drops | `CommandError("stream closed")` |
| 4 | Daemon sends `BenchmarkResult`; `BenchmarkComplete` dispatched | deserialization error | `CommandError(serde error)` |
| 5 | `AppState::dispatch(BenchmarkComplete)` appends result; calls `cap_history` (50-run cap) | — | — |
| 6 | `spawn_save_history_task` writes `benchmarks.json` | directory not writable | `CommandError("failed to save benchmark history: …")` |
| 7 | `CommandOk` shown in status bar | — | "benchmark complete — boringtun 1.23 GB/s" |

### Key bindings conflict analysis

| Key | Context | Action | Conflict? |
|-----|---------|--------|-----------|
| `1` | global | `SelectTab(Overview)` | reserved — cannot use for backend |
| `2` | global | `SelectTab(Status)` | reserved — cannot use for backend |
| `3` | global | `SelectTab(Peers)` | reserved — cannot use for backend |
| `b` | Compare tab, idle | `StartBenchmark` | none |
| `b` | Compare tab, running | feedback only | none |
| `Enter` | Compare tab | `StartBenchmarkForBackend(selected)` | none |
| `w` | Compare tab | `SwitchBenchmarkBackend(selected)` | none (`w` is free globally) |
| `h` | Compare tab | `ToggleCompareView` | none (`h` is free globally) |
| `e` | Compare tab | `EnterExport` | none (`e` free outside Config tab) |
| `j` / `k` | Compare tab | `NextRow` / `PrevRow` | pre-existing, unchanged |
| `Esc` | Export prompt | `ExitExport` | matches Import pattern |
| `Enter` | Export prompt | `SubmitExport` | matches Import pattern |

**Status bar hint line for Compare tab (Live view):**

```
[b] benchmark  [Enter] run selected  [w] use backend  [h] history  [e] export  [j/k] navigate
```

**Status bar hint line for Compare tab (Historical view):**

```
[h] live view  [j/k] scroll  [e] export
```

---

## Implementation Steps

### Commit 1 — IPC extension + benchmark types

**Files:**
- `ferro-wg-core/src/stats.rs` — add `p50_latency`, `p95_latency`,
  `p99_latency: Duration` (all `#[serde(default)]`) to `BenchmarkResult`;
  update any existing struct-literal tests with `..BenchmarkResult::default()`
- `ferro-wg-core/src/ipc.rs` — add `BenchmarkProgress` struct (wire-format);
  add `DaemonCommand::Benchmark { connection_name, duration_secs }`;
  add `DaemonResponse::BenchmarkProgress(BenchmarkProgress)`;
  add `DaemonResponse::BenchmarkResult(BenchmarkResult)`
- `ferro-wg-core/src/client.rs` — add `send_streaming_command` (see API spec above)
- `ferro-wg-tui-core/src/benchmark.rs` — new module: `BenchmarkResultMap`,
  `BenchmarkRun`, `BenchmarkError`, and all pure calculation functions (no I/O)
- `ferro-wg-tui-core/src/action.rs` — add `StartBenchmark`,
  `StartBenchmarkForBackend`, `BenchmarkProgressUpdate`, `BenchmarkComplete`,
  `SwitchBenchmarkBackend`, `ToggleCompareView`, `EnterExport`, `ExportKey`,
  `SubmitExport`, `ExitExport`
- `ferro-wg-tui-core/src/app.rs` — add `InputMode::Export(String)`
- `ferro-wg-tui-core/src/state.rs` — add `CompareView` enum; add
  `benchmark_results`, `benchmark_history`, `benchmark_running`,
  `benchmark_progress_history: VecDeque<BenchmarkProgress>`, `compare_view`
  to `AppState`; add dispatch arms for all new actions; initialize in
  `AppState::new()`

**Tests:**
- `format_throughput(0.0)` → `"0 B/s"`
- `format_throughput(1_000_000.0)` → `"1.00 MB/s"`
- `format_throughput(1_073_741_824.0)` → `"1.00 GB/s"`
- `format_latency(Duration::from_micros(420))` → `"0.42 ms"`
- `throughput_bar_data` with a 3-entry map (`"charlie"`, `"alice"`, `"bob"`)
  returns entries in stable alphabetical order: `[("alice", ...), ("bob", ...), ("charlie", ...)]`
- `throughput_bar_data` with an empty map → returns `vec![]` (no panic)
- `throughput_sparkline_data` with `[BenchmarkProgress { current_throughput_bps: 1_234_567.0, ..}]`
  → returns `[1234]` (truncated kBps, not bytes or latency)
- `throughput_sparkline_data` with empty slice → returns `vec![]` (no panic)
- `best_backend` with all-zero throughput → `None`
- `best_backend` with one nonzero entry → `Some(backend_name)`
- `best_backend` with multiple nonzero entries → `Some` of the backend with the highest `throughput_bps`
- `best_backend` with tied throughput values → deterministic result (first alphabetically — document the tie-breaking rule)
- `cap_history(vec![], 50)` → `vec![]` (no panic on empty input)
- `cap_history(vec![r1], 0)` → `vec![]` (cap of zero evicts everything)
- `BenchmarkError::AlreadyRunning` displays `"benchmark already running"`
- `DaemonCommand::Benchmark { ... }` serializes and deserializes correctly (roundtrip)
- `DaemonResponse::BenchmarkProgress(...)` roundtrip
- `DaemonResponse::BenchmarkResult(...)` roundtrip
- `AppState::dispatch(StartBenchmark)` when `benchmark_running == true`
  → `benchmark_running` stays `true`; feedback contains "already running"
- `AppState::dispatch(StartBenchmark)` when `benchmark_running == false`
  → `benchmark_running` becomes `true`
- `AppState::dispatch(BenchmarkComplete(result))` → `benchmark_running`
  becomes `false`; result appears in `benchmark_results`; run added to
  `benchmark_history`
- `AppState::dispatch(BenchmarkProgressUpdate(p))` → `benchmark_progress_history.back() == Some(&p)`; length increases by 1
- `AppState::dispatch(BenchmarkProgressUpdate(p))` called 61 times → `benchmark_progress_history.len() == 60` (capped at 60)
- `AppState::dispatch(ToggleCompareView)` when `Live` → `Historical`;
  when `Historical` → `Live`
- Dispatching 51 `BenchmarkComplete` actions → `benchmark_history.len() == 50`
  (cap enforced via `cap_history`)

---

### Commit 2 — `CompareComponent` live benchmark UI

**Files:**
- `ferro-wg-tui-components/src/compare.rs` — replace static placeholder table
  with the full Live/Historical layout described above; add key bindings for
  `b`, `Enter`, `w`, `h`, `e`; reuse existing `table_state` for row selection
- `ferro-wg-tui-components/src/status_bar.rs` — add Compare tab hint lines
  for both Live and Historical views; route `ExportKey` / `SubmitExport` /
  `ExitExport` through the existing key-routing pattern used by Import

**Tests:**
- `CompareComponent::handle_key('b')` when `!state.benchmark_running` → `StartBenchmark`
- `CompareComponent::handle_key('b')` when `state.benchmark_running` → `DaemonError("benchmark already running")`
- `CompareComponent::handle_key(Enter)` with row 1 selected → `StartBenchmarkForBackend("neptun")`
- `CompareComponent::handle_key('w')` with row 2 selected → `SwitchBenchmarkBackend("gotatun")`
- `CompareComponent::handle_key('h')` → `ToggleCompareView`
- `CompareComponent::handle_key('e')` → `EnterExport`
- Render snapshot (TestBackend 80×24, Live view, no results): all throughput
  cells show `"—"`; `BarChart` absent or empty; no progress gauge
- Render snapshot (TestBackend 80×24, Live view, results populated):
  `BarChart` visible; throughput values formatted by `format_throughput`
- Render snapshot (TestBackend 80×24, `benchmark_running = true`): `Gauge`
  progress bar row visible
- Render snapshot (TestBackend 80×24, Historical view, 2 runs): list shows
  two entries with timestamps; best-backend annotation present

---

### Commit 3 — Background benchmark task

**Files:**
- `ferro-wg-tui/src/lib.rs` — add `DaemonMessage::BenchmarkProgress` and
  `DaemonMessage::BenchmarkComplete` variants; implement `spawn_benchmark_task`;
  extend `handle_daemon_messages` to dispatch `BenchmarkProgressUpdate` and
  `BenchmarkComplete`; extend `maybe_spawn_command` with arms for
  `StartBenchmark` / `StartBenchmarkForBackend` / `SwitchBenchmarkBackend`

**Mock IPC strategy for Commit 3 tests:**

`spawn_benchmark_task` is extracted to accept a generic stream parameter
instead of calling `client::send_streaming_command` directly, enabling
in-process tests without a live daemon socket:

```rust
// Explicit signature for the generic inner function:
fn spawn_benchmark_task_inner<S>(
    stream: S,
    daemon_tx: mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) where
    S: Stream<Item = DaemonResponse> + Send + 'static,
{
    tasks.spawn(async move {
        tokio::pin!(stream);
        let mut received_result = false;
        while let Some(response) = stream.next().await {
            match response {
                DaemonResponse::BenchmarkProgress(p) => {
                    let _ = daemon_tx.send(DaemonMessage::BenchmarkProgress(p));
                }
                DaemonResponse::BenchmarkResult(r) => {
                    let _ = daemon_tx.send(DaemonMessage::BenchmarkComplete(r));
                    received_result = true;
                    return;
                }
                DaemonResponse::Error(e) => {
                    let _ = daemon_tx.send(DaemonMessage::CommandError(e));
                    return;
                }
                _ => {}
            }
        }
        if !received_result {
            let _ = daemon_tx.send(DaemonMessage::CommandError(
                "stream closed unexpectedly".into(),
            ));
        }
    });
}

// Production call site:
match client::send_streaming_command(cmd).await {
    Ok(stream) => spawn_benchmark_task_inner(stream, daemon_tx, tasks),
    Err(e) => { let _ = daemon_tx.send(error_to_message(&e)); }
}

// Test helper — passes a pre-built iterator as the stream:
fn make_test_stream(
    responses: Vec<DaemonResponse>,
) -> impl Stream<Item = DaemonResponse> {
    futures::stream::iter(responses)
}
```

Tests call `spawn_benchmark_task_inner` with `make_test_stream(...)` directly.
No live socket, no `tokio::net::UnixListener` needed in unit tests.

**Tests:**
- `spawn_benchmark_task_inner` with `[BenchmarkProgress(p1), BenchmarkProgress(p2), BenchmarkResult(r)]`
  → channel receives `DaemonMessage::BenchmarkProgress(p1)`,
  `DaemonMessage::BenchmarkProgress(p2)`, `DaemonMessage::BenchmarkComplete(r)`
  in that order
- `spawn_benchmark_task_inner` with `[Error("daemon overloaded")]`
  → channel receives `DaemonMessage::CommandError("daemon overloaded")`
- `spawn_benchmark_task_inner` with empty stream (daemon closed connection)
  → channel receives `DaemonMessage::CommandError("stream closed unexpectedly")`
- `handle_daemon_messages` with `BenchmarkComplete(result)` dispatches
  `Action::BenchmarkComplete(result)` and sets `benchmark_running = false`
- `AppState::dispatch(StartBenchmark)` when `benchmark_running == false`
  → `benchmark_running` becomes `true`; `benchmark_progress_history` cleared

---

### Commit 4 — Historical storage

`ferro-wg-tui-core` has no `tokio` dependency and must stay I/O-free (it is
the pure state/calculation layer). History I/O belongs in the action/effect
layer. A new **`ferro-wg-tui/src/history.rs`** module owns all async
file operations for benchmark persistence.

**Files:**
- `ferro-wg-tui/src/history.rs` — new module with two async functions:
  ```rust
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
  pub async fn load_benchmark_history(
      path: &Path,
  ) -> Result<Vec<BenchmarkRun>, BenchmarkError> { ... }

  /// Persist `runs` (capped at `BENCHMARK_HISTORY_CAP`) to `path` as
  /// pretty-printed JSON.
  ///
  /// Parsing and serialization are synchronous calculations; only the
  /// `fs::write` call is async. Calls `cap_history` internally before
  /// serializing so the persisted file never exceeds the cap.
  pub async fn save_benchmark_history(
      path: &Path,
      runs: Vec<BenchmarkRun>,
  ) -> Result<(), BenchmarkError> { ... }
  ```
- `ferro-wg-tui/src/lib.rs` — `mod history;`; call
  `history::load_benchmark_history(&benchmarks_path)` on startup and
  propagate loaded history into `AppState::benchmark_history` before the
  event loop starts; call `spawn_save_history_task` after dispatching
  `BenchmarkComplete`; thread `benchmarks_path: PathBuf` through `run()`
  alongside `config_path`. `benchmarks_path` is resolved in `main` (or
  `run`) as `config_path.parent().unwrap_or(Path::new(".")).join("benchmarks.json")` —
  the same directory as the config file, ensuring both files live together.

**Tests** (in `ferro-wg-tui/src/history.rs` using `tempfile::tempdir`):
- `load_benchmark_history` with a non-existent path → `Ok(vec![])` (not an error)
- `load_benchmark_history` with valid JSON → returns correct `Vec<BenchmarkRun>`
- `save_benchmark_history` with 51 runs → written file parses to exactly 50
  entries (oldest evicted by `cap_history`)
- Roundtrip: `save_benchmark_history` then `load_benchmark_history` produces
  identical `Vec<BenchmarkRun>` with all percentile latency fields intact
- Legacy JSON roundtrip: write a JSON file manually without `p50_latency`,
  `p95_latency`, `p99_latency` fields, call `load_benchmark_history`, verify
  those fields deserialize to `Duration::ZERO` (via `#[serde(default)]`)
- `load_benchmark_history` with an empty file (0 bytes) → `Err(BenchmarkError::Serialize(_))`
  (not `Ok(vec![])` — empty is not the same as absent)
- `cap_history` with exactly `cap` items → returns unchanged `Vec` (no eviction)
- `cap_history` with `cap + 1` items → returns `Vec` of length `cap` (first item evicted)

---

### Commit 5 — Export functionality

**Files:**
- `ferro-wg-tui/src/lib.rs` — add `InputMode::Export(String)` to the routing
  guard in `handle_key_event`; capture `export_path` from state before
  dispatch (parallel to `import_path`); call `spawn_export_task` after
  dispatch; implement `spawn_export_task` (JSON vs CSV by extension)
- `ferro-wg-tui-core/src/state.rs` — add `export_buffer()` helper (returns
  `Option<&str>` from `InputMode::Export`); dispatch arms for `EnterExport`,
  `ExportKey`, `SubmitExport`, `ExitExport`
- `ferro-wg-tui-core/src/benchmark.rs` — ensure `benchmark_to_json` and
  `benchmark_to_csv` are test-covered for the empty-slice case

**Tests:**
- `InputMode::Export("path.json")` causes the status bar to render an export prompt
- `AppState::dispatch(SubmitExport)` resets `input_mode` to `Normal`;
  `export_buffer()` returns `None` afterwards
- `spawn_export_task` with a `.json` extension → written file is valid JSON
  parseable as `Vec<BenchmarkRun>`
- `spawn_export_task` with a `.csv` extension → written file's first line is
  exactly `"timestamp_ms,connection_name,backend,throughput_bps,avg_latency_us,p50_latency_us,p95_latency_us,p99_latency_us"`
- `spawn_export_task` with an unwritable path → sends `DaemonMessage::CommandError`
- `spawn_export_task` with 0 runs and `.json` extension → writes `"[]"` (valid
  empty JSON array)
- `benchmark_to_csv(&[])` → returns only the header line (no trailing newline panic)

---

## Tooling Checklist

Run these in order before every commit:

- [ ] `cargo fmt --all` — format first, always
- [ ] `cargo fmt --all --check` — confirms no drift
- [ ] `cargo test --workspace --features boringtun,neptun,gotatun` — all tests pass
- [ ] `cargo build --workspace` — zero warnings
- [ ] `cargo clippy --all-targets --all-features -- -D warnings -D clippy::pedantic` — clean

**Platform note:** any code inside `#[cfg(target_os = "linux")]` blocks cannot
be linted locally on macOS. Wait for CI (Linux runner) before declaring those
changes done.

---

## Verification (manual smoke-test)

1. Start the daemon (`ferro-wg daemon start`) and bring up a connection.
2. Open the TUI and navigate to the Compare tab (`4`).
3. Press `b` — verify the progress bar appears and the `Sparkline` begins
   updating once per second.
4. After the run completes, verify the `BarChart` shows throughput for the
   tested backend and the detail panel shows p50/p95/p99 latency.
5. Press `h` — verify the Historical view shows at least one run with a
   timestamp and best-backend annotation.
6. Press `h` again — verify the Live view is restored.
7. Press `j` to highlight the second backend row, then press `w` — verify the
   status bar shows a `DaemonOk` feedback message confirming the backend switch.
8. Press `e` — verify the export path prompt appears in the status bar.
9. Type `~/benchmarks.json` and press `Enter` — verify the file is created and
   contains valid JSON parseable as `Vec<BenchmarkRun>`.
10. Restart the TUI — verify the Historical view still shows the previous run
    (loaded from `benchmarks.json`).
11. Press `b` while a benchmark is running (within the first 10 s) — verify
    an inline error appears and no second task is spawned.

---

## File Summary

| File | Crate | Change |
|------|-------|--------|
| `ferro-wg-core/src/stats.rs` | `ferro-wg-core` | Add `p50_latency`, `p95_latency`, `p99_latency` (with `#[serde(default)]`) to `BenchmarkResult`; migrate existing struct-literal tests with `..BenchmarkResult::default()` |
| `ferro-wg-core/src/ipc.rs` | `ferro-wg-core` | Add `BenchmarkProgress` struct (wire-format type); add `DaemonCommand::Benchmark`; add `DaemonResponse::BenchmarkProgress` and `DaemonResponse::BenchmarkResult` |
| `ferro-wg-core/src/client.rs` | `ferro-wg-core` | Add `send_streaming_command(cmd) -> Result<impl Stream<Item=DaemonResponse>, DaemonClientError>` |
| `ferro-wg-tui-core/src/benchmark.rs` | `ferro-wg-tui-core` | New module: `BenchmarkResultMap`, `BenchmarkRun`, `BenchmarkError`, all pure calculation functions (no I/O, no tokio) |
| `ferro-wg-tui-core/src/action.rs` | `ferro-wg-tui-core` | Add 10 new action variants for benchmark and export lifecycle |
| `ferro-wg-tui-core/src/app.rs` | `ferro-wg-tui-core` | Add `InputMode::Export(String)`; add `CompareView` enum |
| `ferro-wg-tui-core/src/state.rs` | `ferro-wg-tui-core` | Add 5 benchmark fields (`benchmark_progress_history: VecDeque<BenchmarkProgress>`) to `AppState`; dispatch arms; `export_buffer()` helper |
| `ferro-wg-tui-components/src/compare.rs` | `ferro-wg-tui-components` | Replace static table with Live/Historical layout; `BarChart`, `Sparkline`, `Gauge`; full key bindings |
| `ferro-wg-tui-components/src/status_bar.rs` | `ferro-wg-tui-components` | Compare tab hint lines; `Export` mode routing |
| `ferro-wg-tui/src/history.rs` | `ferro-wg-tui` | **New module**: `load_benchmark_history` and `save_benchmark_history` async I/O helpers (effect layer — keeps I/O out of `ferro-wg-tui-core`) |
| `ferro-wg-tui/src/lib.rs` | `ferro-wg-tui` | `DaemonMessage` additions; `spawn_benchmark_task` / `spawn_benchmark_task_inner`; `spawn_export_task`; `spawn_save_history_task`; routing changes; startup history load via `history::load_benchmark_history` |
