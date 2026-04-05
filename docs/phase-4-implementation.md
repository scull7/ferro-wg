# Phase 4: Connection Lifecycle Management — Implementation Plan

## Pre-implementation checklist

Before writing any Rust code, invoke `/rust-code-writer` (per `CLAUDE.md`).

- Verify `tokio::process::Command` is available (ships with `tokio` `process` feature).
- Confirm `config::wg_quick::load_from_file` and `config::toml::save_app_config` compile
  cleanly — they are already used by the CLI and are the canonical import path.
- `overlay_block()` in `ferro-wg-tui-core/src/theme.rs` is currently unused — no need
  to add new theme helpers; use it directly for the confirmation overlay.
- Daemon start spawns `sudo` which may require a password. Use best-effort spawn with a
  `DaemonError` fallback showing the manual CLI command.

---

## Context

Phase 3 delivered live log streaming. Phase 4 closes the operational loop: users need to
bring connections up/down in bulk, import new configs, control the daemon lifecycle, and
be warned when a tunnel is transmitting but not receiving. Most IPC plumbing already
exists (`DaemonCommand::Up/Down/Shutdown`, `config::wg_quick::load_from_file`,
`config::toml::save_app_config`). This phase adds the UI layer on top.

**Done when:** full connection lifecycle (import → start daemon → up → monitor health →
down → stop daemon) can be performed entirely within the TUI without touching the CLI.

---

## User Stories

| ID | User story | Acceptance criteria |
|----|------------|---------------------|
| US-1 | As a user I want to bring all connections up at once | Pressing `u` on the Overview tab sends `DaemonCommand::Up { connection_name: None }` and shows a success feedback message |
| US-2 | As a user I want to tear all connections down at once | Pressing `d` on the Overview tab shows a confirmation dialog; `y` sends `DaemonCommand::Down { connection_name: None }` |
| US-3 | As a user I want confirmation before destructive actions | Any "tear down all" or "stop daemon" action shows an overlay with `[y] confirm  [n] cancel` before executing |
| US-4 | As a user I want to stop the running daemon | Pressing `S` on the Overview tab triggers a confirm dialog; confirming sends `DaemonCommand::Shutdown` |
| US-5 | As a user I want to start the daemon if it is not running | Pressing `s` on the Overview tab (when daemon is disconnected) spawns the daemon process in the background |
| US-6 | As a user I want to import a wg-quick config | Pressing `i` opens an import path prompt; submitting a valid path parses the file, writes it to config.toml, and adds the new connection to the TUI |
| US-7 | As a user I want to be warned when a tunnel is sending but not receiving | Connections with `tx > 0 && rx == 0` show a `[!]` warning in the Overview health column, the Status header, and the connection bar |
| US-8 | As a user I want to be warned when a handshake is stale | Connections with `last_handshake > 180 s` show the same `[!]` warning indicator |

---

## Architecture

### Existing infrastructure to reuse

```
DaemonCommand::Up   { connection_name: None }  ← all-connections up (ipc.rs:121)
DaemonCommand::Down { connection_name: None }  ← all-connections down (ipc.rs:121)
DaemonCommand::Shutdown                        ← stop daemon (ipc.rs:148)
config::wg_quick::load_from_file()             ← wg-quick parser (config/wg_quick.rs:22)
config::toml::save_app_config()                ← write config.toml (config/toml.rs)
theme.overlay_block()                          ← modal border (theme.rs:120, currently unused)
cmd_daemon_background() spawn pattern          ← sudo + current_exe + daemon (main.rs:313)
health check: rx == 0 && tx > 0               ← existing CLI warning (main.rs:187)
maybe_spawn_command() background task pattern  ← lib.rs:392
Feedback transient message system              ← state.rs:70
```

### New types

```
// ferro-wg-tui-core/src/action.rs
pub enum ConfirmAction { DisconnectAll, StopDaemon }

// ferro-wg-tui-core/src/state.rs
pub struct ConfirmPending { pub message: String, pub action: ConfirmAction }

// ferro-wg-tui-core/src/app.rs  (extend InputMode)
pub enum InputMode { Normal, Search, Import(String) }
```

### New `Action` variants

```rust
// Confirmation dialog
RequestConfirm { message: String, action: ConfirmAction }
ConfirmYes
ConfirmNo

// Bulk connection control
ConnectAll
DisconnectAll

// Daemon lifecycle
StartDaemon
StopDaemon

// wg-quick import
EnterImport
ImportInput(char)
ImportBackspace
SubmitImport
ExitImport

// Config hot-reload after import
ReloadConfig(AppConfig)
```

### New `AppState` fields

```rust
pub config_path: PathBuf        // for import + daemon start
pub pending_confirm: Option<ConfirmPending>
```

### Confirmation overlay rendering

When `pending_confirm.is_some()`, `render_ui` draws a centered floating box **after**
all tab renders (so it sits on top). Use `ratatui::widgets::Clear` + `theme.overlay_block()`
+ a `Paragraph` with the message and `[y] confirm  [n] cancel` prompt. Sized at 60 %
width × 5 rows, offset-centered in the content area. No separate `Component` needed.

### Key routing when confirmation is pending

In `handle_key_event`, check `state.pending_confirm.is_some()` **before** all other
routing. Only `y` → `ConfirmYes` and `n`/`Esc` → `ConfirmNo` are accepted; all other
keys are swallowed.

### Key routing for Import mode

In `handle_key_event`, check `InputMode::Import(_)` (similar to existing `Search`
branch). Route printable chars → `ImportInput`, `Backspace` → `ImportBackspace`,
`Enter` → `SubmitImport`, `Esc` → `ExitImport`.

### After import submit

`lib.rs` extracts the path string from the action, spawns a background task:
1. `config::wg_quick::load_from_file(path)` — parse
2. Name peers (same logic as `cmd_import` in `ferro-wg/src/main.rs:336`)
3. Load or create `AppConfig`, insert connection, `config::toml::save_app_config`
4. Re-read `AppConfig` from disk, send `DaemonMessage::ReloadConfig(new_config)`

On `ReloadConfig`, dispatch `Action::ReloadConfig(app_config)`, which rebuilds
`state.connections` (same logic as `AppState::new`).

### Daemon start

Spawn `tokio::process::Command::new("sudo")` with args `[current_exe, "daemon",
"--daemonize", "-c", config_path]` (mirrors `cmd_daemon_background` in `main.rs:313`).
If spawn fails, send `DaemonMessage::CommandError` with the manual command hint.

---

## New keybindings (Overview tab)

| Key | Action | Condition |
|-----|--------|-----------|
| `u` | `ConnectAll` | always |
| `d` | `DisconnectAll` → confirm | always |
| `s` | `StartDaemon` | `!daemon_connected` |
| `S` | `StopDaemon` → confirm | `daemon_connected` |
| `i` | `EnterImport` | global (Normal mode only) |

Status bar hint line updated to show these when on the Overview tab.

---

## Health indicator computation

Computed during `dispatch(UpdatePeers)` in `state.rs`, stored in `ConnectionStatus`:

```
connected && tx_bytes > 0 && rx_bytes == 0  →  "sending but not receiving"
connected && last_handshake > 180 s          →  "handshake stale"
```

Rendered as `[!]` in `theme.warning` color in:
- **Overview** — new `Health` column (after Backend)
- **Status** — extra line in the connection summary header
- **Connection bar** — `!` appended to the existing state indicator

---

## Implementation Steps (Commits)

### Commit 1 — Config path in TUI + Confirmation Dialog

**Files:**
- `ferro-wg/src/main.rs` — pass `config_path: PathBuf` to `ferro_wg_tui::run()`
- `ferro-wg-tui/src/lib.rs` — update `run()` and `event_loop()` signatures; add confirm
  key routing; render confirm overlay in `render_ui`
- `ferro-wg-tui-core/src/state.rs` — add `config_path`, `pending_confirm`,
  `ConfirmPending`; dispatch `RequestConfirm`, `ConfirmYes`, `ConfirmNo`
- `ferro-wg-tui-core/src/action.rs` — add `ConfirmAction`, `RequestConfirm`,
  `ConfirmYes`, `ConfirmNo`

**Tests:** `AppState::dispatch` roundtrip: `RequestConfirm` → state has pending → `ConfirmYes` → pending cleared and inner action dispatched.

---

### Commit 2 — Connect All / Disconnect All

**Files:**
- `ferro-wg-tui-core/src/action.rs` — add `ConnectAll`, `DisconnectAll`
- `ferro-wg-tui/src/lib.rs` — dispatch both in `maybe_spawn_command()`
- `ferro-wg-tui-core/src/state.rs` — `DisconnectAll` → `RequestConfirm`; `ConnectAll` passes through
- `ferro-wg-tui-components/src/overview.rs` — `u`/`d` keybindings
- `ferro-wg-tui-components/src/status_bar.rs` — `u up-all  d down-all` hints on Overview tab

**Tests:** Overview `handle_key` routes; `DisconnectAll` dispatch triggers `RequestConfirm`.

---

### Commit 3 — Daemon Start / Stop

**Files:**
- `ferro-wg-tui-core/src/action.rs` — add `StartDaemon`, `StopDaemon`
- `ferro-wg-tui-core/src/state.rs` — `StopDaemon` → `RequestConfirm`
- `ferro-wg-tui/src/lib.rs` — handle `StopDaemon` (Shutdown) and `StartDaemon` (subprocess spawn) in `maybe_spawn_command()`
- `ferro-wg-tui-components/src/overview.rs` — `s`/`S` keybindings
- `ferro-wg-tui-components/src/status_bar.rs` — `s start-daemon` / `S stop-daemon` hints

**Tests:** `StopDaemon` dispatch triggers `RequestConfirm`; `StartDaemon` routing.

---

### Commit 4 — wg-quick Import

**Files:**
- `ferro-wg-tui-core/src/app.rs` — add `InputMode::Import(String)`
- `ferro-wg-tui-core/src/action.rs` — add `EnterImport`, `ImportInput(char)`,
  `ImportBackspace`, `SubmitImport`, `ExitImport`, `ReloadConfig(AppConfig)`
- `ferro-wg-tui-core/src/state.rs` — dispatch import mode actions; dispatch `ReloadConfig` (rebuild `connections`)
- `ferro-wg-tui/src/lib.rs` — import mode key routing; spawn import background task; `DaemonMessage::ReloadConfig` variant
- `ferro-wg-tui-components/src/status_bar.rs` — render `Import path: <buf>█` in Import mode; `i import` hint in Normal mode
- Global key `i` → `EnterImport` (added to `handle_global_key` when not in Search/Import/confirm mode)

**Tests:** `EnterImport` sets mode; `ImportInput` accumulates buffer; `SubmitImport` returns to Normal; `ReloadConfig` rebuilds connections.

---

### Commit 5 — Connection Health Indicators

**Files:**
- `ferro-wg-tui-core/src/state.rs` — add `health_warning: Option<String>` to `ConnectionStatus`; compute during `dispatch(UpdatePeers)`
- `ferro-wg-tui-components/src/overview.rs` — `Health` column with `[!]` in `theme.warning`
- `ferro-wg-tui-components/src/status.rs` — warning line in connection summary header
- `ferro-wg-tui-components/src/connection_bar.rs` — `!` appended to state indicator

**Tests:** Unit test health computation for "sending but not receiving" and "handshake stale" conditions.

---

## Tooling Checklist (per commit)

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings -D clippy::pedantic
cargo test --workspace --features boringtun,neptun,gotatun
```

---

## Verification

```bash
# Manual smoke-test (full lifecycle cycle)
ferro-wg tui
  # Overview tab: verify u/d/s/S/i hints show in status bar
  # Press 's'  → daemon starts (or shows hint if already running)
  # Press 'u'  → ConnectAll → all connections come up
  # Press 'i'  → type path to .conf → Enter → connection appears
  # Press 'd'  → confirm overlay → 'y' → all go down
  # Press 'S'  → confirm overlay → 'y' → daemon stops
  # Health: bring up with no server → [!] warning appears within next poll cycle
```

---

## File Summary

| File | Commits |
|---|---|
| `ferro-wg/src/main.rs` | 1 |
| `ferro-wg-tui/src/lib.rs` | 1, 2, 3, 4 |
| `ferro-wg-tui-core/src/action.rs` | 1, 2, 3, 4 |
| `ferro-wg-tui-core/src/state.rs` | 1, 2, 3, 4, 5 |
| `ferro-wg-tui-core/src/app.rs` | 4 |
| `ferro-wg-tui-components/src/overview.rs` | 2, 3, 5 |
| `ferro-wg-tui-components/src/status.rs` | 5 |
| `ferro-wg-tui-components/src/status_bar.rs` | 2, 3, 4, 5 |
| `ferro-wg-tui-components/src/connection_bar.rs` | 5 |
