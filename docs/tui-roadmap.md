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

---

## Phase 1 — Live Daemon Integration

_Foundation for everything else._

- Connect the TUI to the daemon over the existing Unix socket IPC
- Periodic status polling using the tick event (already wired but unused)
- Live stats refresh: Rx/Tx bytes, handshake age, connection state
- Up/Down actions from the TUI (keybind on the selected peer)
- Backend switching from the TUI (`SwitchBackend` command already exists)
- Error and status feedback in the bottom status bar
- Daemon connection state indicator (connected / disconnected / reconnecting)

## Phase 2 — Multi-Connection Support

- Load **all** connections from `AppConfig`, not just the first
- Connection selector or switcher (sidebar list or dedicated tab)
- Per-connection Status, Peers, and Config views
- Aggregate overview showing health across all connections

## Phase 3 — Log Streaming

- Stream log output from the daemon into the TUI
- Scrollable log buffer with a configurable capacity limit
- Log-level filtering (error / warn / info / debug)
- In-viewer search and grep
- Timestamp display per line

## Phase 4 — Connection Lifecycle Management

- Bring up / tear down individual peers or all peers from the TUI
- Import wg-quick configs interactively (path input or file picker)
- Start and stop the daemon from within the TUI
- Connection health indicators and alerts (e.g. no Rx while Tx > 0)

## Phase 5 — Performance Comparison

- Populate the Compare tab with real benchmark data
- Run encapsulation-throughput and latency benchmarks from the TUI
- Side-by-side backend comparison using sparklines or bar charts
- Switch a peer's backend directly from the Compare view
- Historical performance tracking across runs

## Phase 6 — Config Editing

- View and edit interface and peer configuration from the Config tab
- Form-based editing with inline validation
- Save changes to disk with a confirmation prompt
- Config diff view before committing changes

## Phase 7 — UX Polish

- Responsive layout for narrow / short terminals
- Color theming and dark-mode variants
- Mouse support (click tabs, scroll, select rows)
- Popup dialogs for destructive confirmations
- Help overlay (`?` key)
- Notification toasts for asynchronous events (handshake completed, peer went down)
