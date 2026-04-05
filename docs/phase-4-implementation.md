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
ImportKey(KeyEvent)   // replaces ImportInput(char) + ImportBackspace; state.rs unpacks
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

### Confirmation dialog component

Extract to a dedicated `ConfirmDialogComponent` in
`ferro-wg-tui-components/src/confirm_dialog.rs` implementing the `Component` trait.
This keeps rendering logic out of `render_ui` and maintains clean layering.

```rust
pub struct ConfirmDialogComponent;

impl Component for ConfirmDialogComponent {
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        state.pending_confirm.as_ref()?;   // no-op when no dialog is pending
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Some(Action::ConfirmYes),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(Action::ConfirmNo),
            _ => Some(Action::ConfirmNo),  // swallow all other keys
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        let Some(pending) = &state.pending_confirm else { return };
        // Center a 60%-wide × 5-row box over the content area.
        let overlay_area = centered_rect(60, 5, area);
        frame.render_widget(ratatui::widgets::Clear, overlay_area);
        let block = state.theme.overlay_block("Confirm");
        let inner = block.inner(overlay_area);
        frame.render_widget(block, overlay_area);
        let text = format!("{}\n\n[y] confirm   [n] cancel", pending.message);
        frame.render_widget(
            ratatui::widgets::Paragraph::new(text).centered(),
            inner,
        );
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {}
}
```

`centered_rect(pct_x, height, area)` is a private helper in `confirm_dialog.rs`
(same pattern used in other ratatui projects — compute offsets from area width/height).

**Wiring in `ComponentBundle`:** add `confirm_dialog: ConfirmDialogComponent` field.
In `render_ui`, render it **last** (after tab content) using the full content `Rect`
so it floats on top. In `handle_key_event`, check `pending_confirm.is_some()` first
and route to `confirm_dialog.handle_key()` exclusively; swallow all other routing.

### Key routing for Import mode

In `handle_key_event`, check `InputMode::Import(_)` (similar to existing `Search`
branch). Route `Enter` → `SubmitImport`, `Esc` → `ExitImport`, all other keys →
`ImportKey(key_event)`. `AppState::dispatch` unpacks `ImportKey` internally:

```rust
Action::ImportKey(key) => {
    if let InputMode::Import(ref mut buf) = self.input_mode {
        match key.code {
            KeyCode::Char(c) => buf.push(c),
            KeyCode::Backspace => { buf.pop(); }
            _ => {}
        }
    }
}
```

This collapses two Action variants (`ImportInput` + `ImportBackspace`) into one
(`ImportKey`), keeping the enum lean while placing the char-handling logic where it
belongs — in `dispatch`, not in the event router.

### After import submit — validation and error handling

`lib.rs` extracts the path string from the action and spawns a background task.
Each step fails fast and sends a typed `DaemonMessage::CommandError` on failure.

**Step-by-step with error cases:**

| Step | Operation | Error case | User-visible message |
|------|-----------|------------|----------------------|
| 1 | Check `path.exists()` | File not found | `"Import failed: file not found: <path>"` |
| 2 | Check `std::fs::metadata(path).is_ok()` | Permission denied | `"Import failed: cannot read file: <path>"` |
| 3 | `config::wg_quick::load_from_file(path)` | Malformed config | `"Import failed: <WgQuickParse error with line number>"` |
| 4 | Name peers (mirrors `cmd_import` in `main.rs:336`) | — | — |
| 5 | `config::toml::load_app_config` or `AppConfig::default()` if not exists | Parse error on existing config | `"Import failed: could not read existing config: <e>"` |
| 6 | Check `app_config.connections.contains_key(&conn_name)` | Name conflict | `DaemonOk("Replaced existing connection: <name>")` — overwrite silently, matching CLI behaviour |
| 7 | `app_config.insert(conn_name, wg_config)` | — | — |
| 8 | `config::toml::save_app_config(&app_config, config_path)` | Write error (permissions, disk full) | `"Import failed: could not save config: <e>"` |
| 9 | Send `DaemonMessage::ReloadConfig(new_config)` | — | `DaemonOk("Imported: <conn_name>")` |

**`WgError::WgQuickParse`** already carries `line` and `reason` fields
(`ferro-wg-core/src/error.rs:51`). Surface them verbatim — no extra wrapping needed.

On `ReloadConfig`, dispatch `Action::ReloadConfig(app_config)`, which rebuilds
`state.connections` (same logic as `AppState::new`).

### Daemon start — error messaging and recovery

Spawn `tokio::process::Command::new("sudo")` with args `[current_exe, "daemon",
"--daemonize", "-c", config_path]` (mirrors `cmd_daemon_background` in `main.rs:313`).

**Error cases and UX responses:**

| Scenario | Response |
|----------|----------|
| Spawn fails (sudo not found, permission denied) | `DaemonError("Could not start daemon: run 'sudo ferro-wg daemon --daemonize'")` |
| Daemon already running (socket exists) | `DaemonError("Daemon is already running")` — detected by checking socket existence before spawn |
| Daemon starts but socket not available within 3 s | `DaemonError("Daemon started but not yet reachable — try again in a moment")` |
| Daemon starts successfully | `DaemonOk("Daemon started")` + connectivity poll picks it up within next tick |

**Post-spawn socket poll:** after spawning, the background task attempts
`client::send_command(&DaemonCommand::Status)` up to 6 times with 500 ms intervals
(3 s total). On first success send `DaemonMessage::CommandOk`; on timeout send
`DaemonMessage::CommandError` with the "not yet reachable" message. This gives the
user immediate feedback without hanging the event loop.

**Pre-spawn guard:** check if `/tmp/ferro-wg.sock` already exists and is connectable;
if so, skip spawn and send `DaemonError("Daemon is already running")`. This prevents
double-spawn races.

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

### Pure computation function

Health warnings are derived from peer data via a standalone pure function — **not**
computed inline in `dispatch`. This isolates the calculation layer from the state
mutation layer (Grokking Simplicity: separate calculations from actions).

```rust
/// Pure: derives a health warning from a connected tunnel's stats.
///
/// Returns `None` when the connection is healthy or disconnected.
/// Called from `dispatch(UpdatePeers)` and unit-testable without `AppState`.
pub fn compute_health_warning(connected: bool, stats: &TunnelStats) -> Option<String> {
    if !connected {
        return None;
    }
    if stats.tx_bytes > 0 && stats.rx_bytes == 0 {
        return Some("sending but not receiving".to_owned());
    }
    if stats.last_handshake.map_or(false, |hs| hs > Duration::from_secs(180)) {
        return Some("handshake stale".to_owned());
    }
    None
}
```

Lives in `ferro-wg-tui-core/src/state.rs` as a module-level `pub fn` (not a method).
`dispatch(UpdatePeers)` calls it after updating each `ConnectionStatus`:

```rust
status.health_warning = compute_health_warning(peer.connected, &peer.stats);
```

`ConnectionStatus` gains `pub health_warning: Option<String>`.

### Rendering

Rendered as `[!]` in `theme.warning` color in:
- **Overview** — new `Health` column (after Backend)
- **Status** — extra line in the connection summary header
- **Connection bar** — `!` appended to the existing state indicator

---

## Implementation Steps (Commits)

### Commit 1 — Config path in TUI + Confirmation Dialog

**Files:**
- `ferro-wg/src/main.rs` — pass `config_path: PathBuf` to `ferro_wg_tui::run()`
- `ferro-wg-tui/src/lib.rs` — update `run()` and `event_loop()` signatures; add
  `confirm_dialog` to `ComponentBundle`; route keys to it when `pending_confirm.is_some()`
  before all other handlers; render it last in `render_ui`
- `ferro-wg-tui-core/src/state.rs` — add `config_path`, `pending_confirm`,
  `ConfirmPending`; dispatch `RequestConfirm`, `ConfirmYes`, `ConfirmNo`
- `ferro-wg-tui-core/src/action.rs` — add `ConfirmAction`, `RequestConfirm`,
  `ConfirmYes`, `ConfirmNo`
- `ferro-wg-tui-components/src/confirm_dialog.rs` — new `ConfirmDialogComponent`
  with `centered_rect` helper; export from `ferro-wg-tui-components/src/lib.rs`

**Tests:** `AppState::dispatch` roundtrip: `RequestConfirm` → state has pending →
`ConfirmYes` → pending cleared and inner action dispatched; `ConfirmDialogComponent`
returns `None` from `handle_key` when `pending_confirm` is `None`.

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

**Tests:** `StopDaemon` dispatch triggers `RequestConfirm`; `StartDaemon` routing;
socket-exists guard prevents double-spawn; post-spawn poll sends `CommandOk` on success
and `CommandError` on timeout (mock the socket check with a test helper).

---

### Commit 4 — wg-quick Import

**Files:**
- `ferro-wg-tui-core/src/app.rs` — add `InputMode::Import(String)`
- `ferro-wg-tui-core/src/action.rs` — add `EnterImport`, `ImportKey(KeyEvent)`,
  `SubmitImport`, `ExitImport`, `ReloadConfig(AppConfig)`
- `ferro-wg-tui-core/src/state.rs` — dispatch import mode actions; dispatch `ReloadConfig` (rebuild `connections`)
- `ferro-wg-tui/src/lib.rs` — import mode key routing; spawn import background task; `DaemonMessage::ReloadConfig` variant
- `ferro-wg-tui-components/src/status_bar.rs` — render `Import path: <buf>█` in Import mode; `i import` hint in Normal mode
- Global key `i` → `EnterImport` (added to `handle_global_key` when not in Search/Import/confirm mode)

**Tests:** `EnterImport` sets mode; `ImportKey(Char('x'))` appends to buffer;
`ImportKey(Backspace)` pops last char; `SubmitImport` returns to Normal;
`ReloadConfig` rebuilds connections; import task returns `CommandError` for missing file,
unreadable file, and malformed wg-quick content (unit-test the background task logic
using a temp dir).

---

### Commit 5 — Connection Health Indicators

**Files:**
- `ferro-wg-tui-core/src/state.rs` — add `health_warning: Option<String>` to `ConnectionStatus`; compute during `dispatch(UpdatePeers)`
- `ferro-wg-tui-components/src/overview.rs` — `Health` column with `[!]` in `theme.warning`
- `ferro-wg-tui-components/src/status.rs` — warning line in connection summary header
- `ferro-wg-tui-components/src/connection_bar.rs` — `!` appended to state indicator

**Tests:** Unit test `compute_health_warning` directly (no `AppState` needed):
disconnected → `None`; `tx > 0 && rx == 0` → `Some`; `last_handshake > 180s` → `Some`;
both healthy → `None`; `dispatch(UpdatePeers)` propagates warning into `ConnectionStatus`.

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
| `ferro-wg-tui-components/src/confirm_dialog.rs` | 1 (new) |
