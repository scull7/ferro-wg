# TUI Roadmap

High-level feature roadmap for the ferro-wg terminal UI.
The goal is a full management tool: operational dashboard, performance lab,
and connection lifecycle management — all from the terminal.

## Current State

The TUI has five tabs (Status, Peers, Compare, Config, Logs) with tab/row
navigation, vim keybindings, and search filtering. It is currently **read-only
and static**: it loads the first connection from the config file at startup and
never communicates with the daemon. The Compare tab is a placeholder and the
Logs tab is empty.

| What works today | What's missing |
|---|---|
| 5 tabs with keyboard navigation | No daemon communication |
| Search/filter across peers | Only first connection loaded |
| Rx/Tx display (static) | No live stats refresh |
| Backend availability check | No benchmarks or real data |
| Config display | No editing capability |
| Log tab exists | No log streaming |

## Architecture

Adopt the [tursotui](https://crates.io/crates/tursotui) component architecture:

- **Unidirectional data flow** — components emit `Action` variants, `AppState`
  processes state changes, results route back via two-phase dispatch
  (Phase 1: state mutation, Phase 2: effects and I/O).
- **`Component` trait** — every panel implements `handle_key()`, `update()`,
  `render()` with consistent `panel_block()` / `overlay_block()` helpers for
  styled borders.
- **Async operations** — `tokio::spawn` with results delivered via `mpsc`
  channel. Daemon IPC, benchmarks, and log streaming all run as async tasks
  without blocking the UI event loop.
- **Theme system** — Catppuccin Mocha (dark) and Latte (light) palettes with
  semantic color roles for connection states, log levels, and performance
  metrics.
- **No unsafe code** — enforce `#[forbid(unsafe_code)]` project-wide.
- **Modularity** — each tab/panel is an independent component behind the
  `Component` trait. New views can be added by implementing the trait and
  registering in the tab enum — no changes to the core event loop.

### Event Loop

Target ~60 fps with a 16 ms crossterm poll timeout. Each frame:
1. Drain crossterm events → route to active component's `handle_key()`
2. Drain async `mpsc` channel → dispatch `Action`s through `AppState`
3. Render all visible components

---

## Phase 0 — Architecture Refactor _(small)_

_Prerequisite for all other phases. Restructure the existing TUI to the
component architecture before adding features._

**Depends on:** nothing

- Extract `Component` trait with `handle_key()`, `update()`, `render()`
- Define `Action` enum and centralized `AppState` dispatcher
- Convert each existing tab view into a `Component` implementor
- Add `mpsc` channel plumbing for async results
- Wire up `panel_block()` / `overlay_block()` helpers
- Integrate Catppuccin theme with semantic color roles
- Add `#[forbid(unsafe_code)]` to the workspace

**Done when:** existing TUI renders identically with the new architecture,
all existing tests pass, and a new component can be added by implementing
`Component` + registering a tab variant — no event-loop changes required.

## Phase 1 — Live Daemon Integration _(medium)_

_Foundation for everything else._

**Depends on:** Phase 0 (Component trait, async channel plumbing)

- Connect the TUI to the daemon over the existing Unix socket IPC
- Periodic status polling using the tick event (already wired but unused)
- Live stats refresh: Rx/Tx bytes, handshake age, connection state
- Up/Down actions from the TUI (keybind on the selected peer)
- Backend switching from the TUI (`SwitchBackend` command already exists)
- Error and status feedback in the bottom status bar
- Daemon connection state indicator (connected / disconnected / reconnecting)
- Graceful degradation when daemon is unreachable (offline mode with last-known state)

**Done when:** TUI can reliably bring up/down any connection and reflect
daemon state within < 1 s. Daemon disconnect is detected and displayed
within 2 ticks. All daemon commands round-trip without blocking the UI.

## Phase 2 — Multi-Connection Support _(medium)_

**Depends on:** Phase 1 (daemon integration for live state)

- Load **all** connections from `AppConfig`, not just the first
- Connection selector or switcher (sidebar list or dedicated tab)
- Per-connection Status, Peers, and Config views
- Aggregate overview showing health across all connections
- Per-connection `Component` state (selection, scroll position)

**Done when:** all configured connections are visible, individually
selectable, and show live status. Aggregate view shows at-a-glance
health for every connection.

## Phase 3 — Log Streaming _(medium)_

**Depends on:** Phase 1 (daemon IPC), Phase 2 (multi-connection context)

- Stream log output from the daemon into the TUI via async channel
- Scrollable log buffer with a configurable capacity limit
- Log-level filtering (error / warn / info / debug)
- In-viewer search and grep
- Timestamp display per line
- Per-connection log filtering when multiple connections are active

**Done when:** logs stream in real time with < 100 ms latency, level
filtering works without dropping messages, and buffer wraps cleanly
at capacity without visible stutter.

## Phase 4 — Connection Lifecycle Management _(medium)_

**Depends on:** Phase 1 (up/down), Phase 2 (multi-connection)

- Bring up / tear down individual peers or all peers from the TUI
- Import wg-quick configs interactively (path input or file picker)
- Start and stop the daemon from within the TUI
- Connection health indicators and alerts (e.g. no Rx while Tx > 0)
- Confirmation dialogs for destructive actions (tear down all)

**Done when:** full connection lifecycle (import → up → monitor → down)
can be performed entirely within the TUI without touching the CLI.

## Phase 5 — Performance Comparison _(large)_

_Core differentiator — apples-to-apples backend comparison._

**Depends on:** Phase 1 (daemon IPC, backend switching), Phase 2 (multi-connection)

**Metrics to capture:**
- Handshake time (initial + rekey)
- Encapsulation throughput (packets/s and bytes/s)
- Packet processing latency (p50 / p95 / p99)
- CPU usage per backend during sustained load

**Features:**
- Run benchmarks from the TUI against any active connection
- Side-by-side comparison using ratatui `Sparkline` / `BarChart` widgets
- Ability to run the same connection on different backends simultaneously
  for true apples-to-apples comparison (if daemon supports concurrent backends)
- Switch a peer's backend directly from the Compare view
- Historical results stored locally; compare across runs
- Export results as JSON or CSV

**Done when:** Compare tab shows throughput, latency, and handshake times
side-by-side for all three backends with live-updating charts during an
active benchmark run.

## Phase 6 — Config Editing _(medium)_

**Depends on:** Phase 2 (multi-connection context), Phase 4 (lifecycle management)

- View and edit interface and peer configuration from the Config tab
- Form-based editing with inline validation (IP ranges, ports, base64 keys)
- **Safety:** automatic backup of the original config before saving
  (`config.toml.bak`)
- **Dry-run / preview:** config diff view showing exactly what will change
  before committing
- **Apply with confirmation:** explicit "Save & Apply" step; optionally
  reconnect affected tunnels after save
- Validation against WireGuard constraints (unique allowed-IPs, valid
  endpoints, key format)

**Done when:** a user can edit any config field, preview the diff, save
with automatic backup, and optionally re-apply the connection — all
without leaving the TUI. Invalid input is rejected with clear inline
errors before save is allowed.

## Phase 7 — UX Polish _(small — MVP scope)_

**Depends on:** all prior phases (polish what exists)

**MVP polish (in scope):**
- Responsive layout for narrow / short terminals (min 80×24)
- Catppuccin Mocha + Latte themes fully applied
- Mouse support (click tabs, scroll, select rows)
- Popup dialogs for destructive confirmations
- Help overlay (`?` key) showing all keybindings
- Notification toasts for async events (handshake completed, peer down)

**Nice-to-haves (out of scope for MVP, tracked separately):**
- Custom user themes beyond Catppuccin
- Configurable keybindings
- Session restore (remember last tab, scroll position)
- Sixel/Kitty image protocol for richer charts

---

## Testing Strategy

Each phase should include tests at the appropriate level:

| Phase | Test type | Approach |
|---|---|---|
| 0 | Unit | Component trait dispatch, Action routing, state transitions |
| 1–2 | Integration | Spawn daemon with mock backend, verify TUI state via `AppState` assertions |
| 3 | Integration | Mock log stream over IPC, verify buffer behavior and filtering |
| 4 | Integration | Full lifecycle: import → up → status → down against test daemon |
| 5 | Benchmark | Dedicated benchmark harness; verify chart rendering with synthetic data |
| 6 | Unit + Integration | Validation logic (unit), save/backup/diff flow (integration) |
| 7 | Snapshot | Terminal snapshot tests for layout at various terminal sizes |

For phases 1–4, integration tests should spin up the daemon with a real
or mock backend and exercise the TUI's `AppState` + `Action` pipeline
end-to-end without requiring a real WireGuard tunnel.

---

## Effort Estimates

| Phase | Scope | Rough size |
|---|---|---|
| 0 — Architecture Refactor | Restructure existing code | Small |
| 1 — Daemon Integration | IPC client, async polling, actions | Medium |
| 2 — Multi-Connection | State per connection, selector UI | Medium |
| 3 — Log Streaming | Async log channel, viewer component | Medium |
| 4 — Lifecycle Management | Import, daemon control, health | Medium |
| 5 — Performance Comparison | Benchmarking, charts, export | Large |
| 6 — Config Editing | Forms, validation, backup, diff | Medium |
| 7 — UX Polish | Themes, mouse, responsive, help | Small |
