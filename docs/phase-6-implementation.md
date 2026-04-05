# Phase 6: Config Editing — Implementation Plan

## Pre-implementation checklist

Before writing any Rust code, invoke `/rust-code-writer` (per `CLAUDE.md`).

- Confirm `config::toml::load_app_config`, `save_app_config`, and
  `save_app_config_string` compile cleanly — all three are already used
  by the CLI and are the canonical import paths.
- Verify `tokio::fs::copy` and `tokio::fs::write` are used instead of their
  `std::fs` equivalents in the async save task — no blocking I/O inside
  `tokio::spawn`.
- Confirm `WgConfig::validate` is callable on the edited config before
  serialisation — it already rejects empty peer lists and missing allowed-IPs.
- Confirm `centered_rect` is extracted to a `pub(crate)` function in
  `ferro-wg-tui-components/src/util.rs` — both `ConfirmDialogComponent` and
  `DiffPreviewComponent` import it from there rather than duplicating it.
- Confirm `theme.overlay_block()` and `theme.muted` are already available in
  `ferro-wg-tui-core/src/theme.rs` — no new theme helpers needed.
- Confirm `DaemonMessage::ReloadConfig(AppConfig, String)` already exists in
  `ferro-wg-tui/src/lib.rs` — reuse it for the post-save reload path.
- Confirm `InputMode` is in `ferro-wg-tui-core/src/app.rs` — the new
  `EditField` variant is added there alongside `Normal`, `Search`, and
  `Import(String)`.

---

## Context

Phase 4 delivered full connection lifecycle management: users can bring tunnels
up/down in bulk, import new configs, and control the daemon — all from the TUI.
Phase 6 completes the configuration story by making the Config tab interactive.
The existing `ConfigComponent` is fully read-only: it renders public key, listen
port, addresses, DNS, and MTU but defines no `handle_key` bindings. This phase
replaces that static view with a form-based editor that covers all mutable
interface and peer fields.

**Safety and correctness are the primary concerns.** `PrivateKey` is never
exposed for editing — it is displayed as `(read-only)` in `theme.muted`. For
existing peers, `PublicKey` is equally protected: editing the peer's identity
after a handshake would silently break the tunnel. Only brand-new peers (created
via `+`) require a public key entry. All list fields (`addresses`, `dns`,
`allowed_ips`) are edited as comma-separated strings in a single-line buffer
and parsed on confirm. Before writing anything to disk, the save flow performs
an atomic backup (preserving the full filename, e.g. `config.toml.bak`), runs
`WgConfig::validate`, serialises to a string, presents a diff overlay, and only
commits after explicit user confirmation. If the backup itself fails, the write
is aborted and an error is shown — the original config is never modified without
a successful backup.

**Done when:** a user can navigate to the Config tab, edit any mutable interface
or peer field, add a new peer, delete an existing peer (with confirmation), preview
the exact diff of changes, save with automatic backup, and optionally reconnect
affected tunnels — all without leaving the TUI. Invalid input is rejected with
clear inline errors before the save step is reachable.

---

## User Stories

| ID | User story | Acceptance criteria |
|----|------------|---------------------|
| US-1 | As a user I want to edit interface fields from the Config tab | Pressing `e` on the Config tab when an interface field is focused enters `EditField` mode with the current value pre-filled in the buffer |
| US-2 | As a user I want inline validation while editing | Each field's validator runs on every `ConfigEditKey` dispatch; an inline error message is shown in `theme.error` beneath the field when the buffer is invalid, and the Save action is blocked |
| US-3 | As a user I want to edit peer fields | Pressing `e` on a focused peer row enters `EditField` mode; all mutable peer fields are editable; `PublicKey` is shown as `(read-only)` for existing peers |
| US-4 | As a user I want to add a new peer | Pressing `+` on the Config tab appends a blank peer and enters `EditField` mode on its `PublicKey` field, which is required for new peers |
| US-5 | As a user I want to delete a peer with confirmation | Pressing `x` on a focused peer row triggers a `ConfirmDialogComponent` overlay; confirming removes the peer from the pending edit state |
| US-6 | As a user I want to preview exactly what will change before saving | Pressing `p` (or `Enter` after the last field) opens the `DiffPreviewComponent` overlay showing a unified diff of old vs new TOML |
| US-7 | As a user I want to save with automatic backup | Pressing `s` in the diff preview writes `config.toml.bak` then `config.toml`; aborting if the backup step fails |
| US-8 | As a user I want to optionally reconnect after saving | Pressing `r` instead of `s` in the diff preview saves and then reconnects affected tunnels by sending `DaemonMessage::Reconnect(conn_name)` |
| US-9 | As a user I want to discard all edits and return to read-only view | Pressing `Esc` while in `EditField` mode or in the diff preview cancels the edit and restores the original config in `ConfigEditState` |

---

## Architecture

### Existing infrastructure to reuse

```
config::toml::load_app_config(path)        ← read current config before diffing (sync; wrap in spawn_blocking)
config::toml::save_app_config(cfg, path)   ← write after successful backup
config::toml::save_app_config_string(cfg)  ← generate diff and backup-free preview
WgConfig::validate()                       ← run before allowing the save step
DaemonMessage::ReloadConfig(AppConfig, String) ← hot-reload after save (lib.rs)
theme.overlay_block(title)                 ← modal border for diff overlay
theme.muted / theme.error / theme.accent   ← field label, error, and edit cues
ConfirmDialogComponent + ConfirmAction     ← reuse for DeletePeer confirmation
maybe_spawn_command() / spawn_import_task() pattern ← background save task shape
centered_rect(pct_x, height, area)         ← pub(crate) in ferro-wg-tui-components/src/util.rs
handle_key_event routing priority chain    ← DiffPreview checked before pending_confirm
```

### Architecture notes

**`config_diff` and `DiffLine` placement:** Although `config_diff` is a pure
string transform, `DiffLine` is TUI-specific (its variants drive color rendering),
so both live in `ferro-wg-tui-core` rather than `ferro-wg-core`.

**`centered_rect` extraction:** `centered_rect` is a `pub(crate)` function in
`ferro-wg-tui-components/src/util.rs`. Both `ConfirmDialogComponent` and
`DiffPreviewComponent` import it from there. This avoids code duplication and
coupling.

### New types

```rust
// ferro-wg-tui-core/src/config_edit.rs  (new file)

/// Which section of the Config tab is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSection {
    /// The `[interface]` block.
    Interface,
    /// A specific peer by index.
    Peer(usize),
}

/// A mutable field in the interface or peer form.
///
/// This is a field **descriptor only** — it carries a name, label, and
/// validator tag. It carries no runtime peer index or current value.
/// The `usize` in `ConfigSection::Peer(usize)` is ignored by
/// `fields_for_section` for field-set selection; all peers share the same
/// field structure. The function returns one of two pre-defined `static`
/// arrays based on `(section_variant, is_new_peer)`:
///
/// - `Interface` → always the same 10 fields
/// - `Peer(…), is_new_peer=false` → 5 fields excluding `PeerPublicKey`
/// - `Peer(…), is_new_peer=true` → 6 fields with `PeerPublicKey` first
///
/// This makes `&'static [EditableField]` achievable without allocation.
///
/// Determines which validator and label to use and which struct field
/// is written back on confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditableField {
    // -- Interface fields --
    ListenPort,
    Addresses,   // comma-separated CIDR strings
    Dns,         // comma-separated IP strings
    DnsSearch,   // comma-separated domain strings
    Mtu,
    Fwmark,
    PreUp,       // comma-separated command strings
    PostUp,
    PreDown,
    PostDown,
    // -- Peer fields --
    PeerName,
    PeerPublicKey,      // required for new peers; read-only for existing
    PeerEndpoint,
    PeerAllowedIps,     // comma-separated CIDR strings
    PeerPersistentKeepalive,
}

/// Pending edits for one connection, held in `AppState` during editing.
///
/// Cloned from `ConnectionView::config` when editing begins. Never
/// written back to `ConnectionView` until the user confirms the save.
/// Discarded on `Esc` or `ConfirmNo`.
#[derive(Debug, Clone)]
pub struct ConfigEditState {
    /// Connection being edited (used for lookup and for the save path).
    pub connection_name: String,
    /// The mutable working copy of the config.
    pub draft: WgConfig,
    /// Which section of the form is focused (interface vs peer N).
    pub focused_section: ConfigSection,
    /// Which field within the section is focused.
    pub focused_field_idx: usize,
    /// If `Some`, the field is in edit mode and this is the text buffer.
    pub edit_buffer: Option<String>,
    /// Inline validation error for the current buffer, if any.
    pub field_error: Option<String>,
}

/// A single line in a config diff.
///
/// `DiffLine` is TUI-specific — its variants drive color rendering decisions,
/// so it lives in `ferro-wg-tui-core` alongside `config_diff`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    /// A line present only in the old config.
    Removed(String),
    /// A line present only in the new config.
    Added(String),
    /// A line unchanged between old and new.
    Context(String),
}

/// Pending diff preview shown before save.
///
/// Stored in `AppState` when the user requests a preview. Cleared on
/// save, discard, or `Esc`.
#[derive(Debug, Clone)]
pub struct ConfigDiffPending {
    /// Connection name being saved.
    pub connection_name: String,
    /// The final validated draft to be written on confirm.
    pub draft: WgConfig,
    /// Diff lines to display (context ± 3 lines around changes).
    pub diff_lines: Vec<DiffLine>,
    /// Scroll offset for the diff overlay.
    pub scroll_offset: usize,
}
```

### Validation pure functions

All validators live in `ferro-wg-tui-core/src/config_edit.rs` as module-level
`pub fn`s with no `AppState` dependency. They operate on `&str` input and
return `Result<(), String>` where the `Err` string is the inline error shown
beneath the field.

```rust
// ferro-wg-tui-core/src/config_edit.rs

/// Validate a UDP port.
///
/// - Accepts the empty string `""` — mapped to `listen_port = 0` (OS picks a port).
/// - Accepts `"0"` explicitly for the same reason.
/// - Accepts any value 1–65535.
/// - Rejects values > 65535.
/// - Rejects non-numeric, non-empty strings.
pub fn validate_port(s: &str) -> Result<(), String> { ... }

/// Validate a comma-separated list of CIDR addresses (e.g. `10.0.0.2/24`).
pub fn validate_addresses(s: &str) -> Result<(), String> { ... }

/// Validate a comma-separated list of IP addresses (DNS servers).
pub fn validate_dns_ips(s: &str) -> Result<(), String> { ... }

/// Validate a comma-separated list of DNS search domains.
pub fn validate_dns_search(s: &str) -> Result<(), String> { ... }

/// Validate an MTU value (576–9000, or 0 for auto).
pub fn validate_mtu(s: &str) -> Result<(), String> { ... }

/// Validate a firewall mark (any `u32`, including 0).
pub fn validate_fwmark(s: &str) -> Result<(), String> { ... }

/// Validate a WireGuard base64 public key (44 characters, valid base64).
pub fn validate_public_key(s: &str) -> Result<(), String> { ... }

/// Validate a peer endpoint (`host:port` or empty for receive-only peers).
pub fn validate_endpoint(s: &str) -> Result<(), String> { ... }

/// Validate a comma-separated list of allowed-IP CIDR ranges.
///
/// Also checks for exact string duplicates against all peers in the draft —
/// WireGuard forbids duplicate allowed-IP entries. Note: only exact string
/// duplicates are rejected; CIDR overlaps that are not exact duplicates are
/// permitted (WireGuard kernel enforcement handles overlap detection at
/// runtime). The `other_peers_allowed_ips` slice is a flat list of all
/// existing allowed-IP strings across all other peers; callers flatten
/// `peer.allowed_ips.iter()` into a collected `Vec<String>` before calling.
pub fn validate_allowed_ips(s: &str, other_peers_allowed_ips: &[String]) -> Result<(), String> { ... }

/// Validate a persistent keepalive interval (0–65535 seconds).
pub fn validate_persistent_keepalive(s: &str) -> Result<(), String> { ... }

/// Compute a unified diff between two TOML strings as a `Vec<DiffLine>`.
///
/// Uses a simple line-by-line LCS diff (stdlib only; no external diff crate).
/// Returns context lines (up to 3 before and after each changed block) plus
/// `Added` / `Removed` lines. Called from `dispatch(PreviewConfig)` —
/// never from a render path.
///
/// Although this is a pure string transform, it lives in `ferro-wg-tui-core`
/// alongside `DiffLine` because `DiffLine` is TUI-specific.
pub fn config_diff(old_toml: &str, new_toml: &str) -> Vec<DiffLine> { ... }
```

These functions are trivially unit-testable in isolation: no `AppState`, no
`Component`, no `tokio`. They belong at the lowest calculation layer of the
stratified design.

### `fields_for_section`

```rust
// ferro-wg-tui-core/src/config_edit.rs

/// Return the ordered slice of editable fields for the given section.
///
/// `EditableField` is a descriptor only — no peer index or current value
/// is embedded. The `usize` in `ConfigSection::Peer(usize)` is ignored;
/// all peers share the same field structure. Returns one of three pre-defined
/// `static` arrays:
///
/// - `Interface` → 10 fields (all interface fields)
/// - `Peer(…), is_new_peer=false` → 5 fields (excludes `PeerPublicKey`)
/// - `Peer(…), is_new_peer=true` → 6 fields (`PeerPublicKey` first)
pub fn fields_for_section(section: ConfigSection, is_new_peer: bool) -> &'static [EditableField] { ... }
```

### New `InputMode` variant

```rust
// ferro-wg-tui-core/src/app.rs

pub enum InputMode {
    /// Arrow keys navigate, hotkeys active.
    Normal,
    /// Typing into the search bar.
    Search,
    /// Typing an import file path. Inner `String` is the current buffer.
    Import(String),
    /// Editing a single config field. Buffer lives in `AppState::config_edit`.
    EditField,
}
```

`EditField` carries no buffer of its own — the buffer lives in
`AppState::config_edit.as_ref().and_then(|s| s.edit_buffer.as_deref())`.
This keeps `InputMode` a lightweight enum while the full edit context is
accessed only when needed.

### New `Action` variants

Action naming follows the existing verb-first convention (`EnterImport`,
`SubmitImport`, `ExitImport`, `EnterSearch`, `ExitSearch`). Navigation and
section-selection actions use a consistent compound-noun form as a group.

```rust
// ferro-wg-tui-core/src/action.rs  (additions)

// -- Config editing --

/// Enter edit mode for the focused field in the Config tab.
/// Copies the current field value into `AppState::config_edit.edit_buffer`.
EnterConfigEdit,

/// Forward a key event to the active edit buffer.
/// `AppState::dispatch` unpacks char/backspace; `Enter` → `CommitConfigEdit`;
/// `Esc` → `CancelConfigEdit`.
ConfigEditKey(KeyEvent),

/// Commit the current buffer to the draft, run the field validator, and
/// return to focused-but-not-editing state. Blocked if `field_error` is Some.
CommitConfigEdit,

/// Discard the current buffer and return to focused-but-not-editing state.
CancelConfigEdit,

/// Move field focus down within the current section (wraps).
ConfigFocusNext,

/// Move field focus up within the current section (wraps).
ConfigFocusPrev,

/// Move section focus to the Interface block.
ConfigFocusInterface,

/// Move section focus to peer at the given index.
ConfigFocusPeer(usize),

/// Append a new blank peer to the draft and enter EditField on its PublicKey.
AddConfigPeer,

/// Remove the peer at the given index from the draft (after confirmation).
DeleteConfigPeer(usize),

/// Request the diff preview: serialise the draft to TOML, diff against the
/// original, and store the result in `AppState::config_diff_pending`.
/// Blocked if any field has a pending `field_error` or `WgConfig::validate` fails.
PreviewConfig,

/// Scroll the diff preview overlay down by one line.
ConfigDiffScrollDown,

/// Scroll the diff preview overlay up by one line.
ConfigDiffScrollUp,

/// Save the pending draft to disk (backup first), then reload config state.
/// Sent from within the diff preview; clears `config_diff_pending` on success.
SaveConfig {
    /// When `true`, reconnect affected tunnels after saving.
    reconnect: bool,
},

/// Discard all pending edits and clear `AppState::config_edit`.
DiscardConfigEdits,
```

Also extend `ConfirmAction`:

```rust
// ferro-wg-tui-core/src/action.rs  (extend existing enum)

pub enum ConfirmAction {
    DisconnectAll,
    StopDaemon,
    /// Delete the peer at this index from the draft.
    DeletePeer(usize),
}
```

`ConfirmAction::DeletePeer` is a non-exhaustive match extension — the compiler
will enforce that `confirmed_action()` in `ferro-wg-tui/src/lib.rs` is updated
to dispatch `Action::DeleteConfigPeer(i)` for this arm.

### New `AppState` fields

```rust
// ferro-wg-tui-core/src/state.rs  (additions to AppState)

/// Pending config edit session, `Some` while the Config tab is in edit mode.
///
/// Cleared on `DiscardConfigEdits`, `SaveConfig`, or `ConfirmNo` after
/// a `DeletePeer` dialog.
pub config_edit: Option<ConfigEditState>,

/// Pending diff preview, `Some` when the diff overlay is shown.
///
/// Cleared on `SaveConfig` (success or error) or `Esc` in the overlay.
pub config_diff_pending: Option<ConfigDiffPending>,
```

`config_path` is NOT added to `AppState`. It is threaded as `&Path` through
`handle_key_event` → `maybe_spawn_command` → background save task, exactly
as it is today for the import flow.

### `DiffPreviewComponent`

A new component in `ferro-wg-tui-components/src/diff_preview.rs`, implementing
the `Component` trait. It renders `state.config_diff_pending` — it owns no
state of its own.

```rust
// ferro-wg-tui-components/src/diff_preview.rs

/// Diff preview overlay shown before saving a config edit.
///
/// Renders a scrollable 80%-wide overlay of up to 15 visible lines when
/// `state.config_diff_pending` is `Some`. Key bindings:
///
/// - `s`        → [`Action::SaveConfig { reconnect: false }`]
/// - `r`        → [`Action::SaveConfig { reconnect: true }`]
/// - `j` / `↓`  → [`Action::ConfigDiffScrollDown`]
/// - `k` / `↑`  → [`Action::ConfigDiffScrollUp`]
/// - `Esc` / `q` → [`Action::DiscardConfigEdits`]
pub struct DiffPreviewComponent;

impl DiffPreviewComponent {
    pub fn new() -> Self { Self }
}

impl Default for DiffPreviewComponent {
    fn default() -> Self { Self::new() }
}

impl Component for DiffPreviewComponent {
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        state.config_diff_pending.as_ref()?;
        match key.code {
            KeyCode::Char('s') => Some(Action::SaveConfig { reconnect: false }),
            KeyCode::Char('r') => Some(Action::SaveConfig { reconnect: true }),
            KeyCode::Char('j') | KeyCode::Down  => Some(Action::ConfigDiffScrollDown),
            KeyCode::Char('k') | KeyCode::Up    => Some(Action::ConfigDiffScrollUp),
            KeyCode::Esc | KeyCode::Char('q')   => Some(Action::DiscardConfigEdits),
            _ => None,
        }
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {}

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        let Some(pending) = &state.config_diff_pending else { return };
        // 80%-wide, up to 15 visible diff lines plus 4 rows for border + hint
        let overlay_height = (pending.diff_lines.len().min(15) as u16) + 4;
        let overlay_area = util::centered_rect(80, overlay_height, area);
        frame.render_widget(Clear, overlay_area);
        let block = state.theme.overlay_block("Config Diff — preview");
        let inner = block.inner(overlay_area);
        frame.render_widget(block, overlay_area);
        // Render diff lines with colour: Added → theme.ok, Removed → theme.error,
        // Context → theme.muted.
        // Bottom hint line: "[s] save   [r] save & reconnect   [Esc] discard"
        render_diff_lines(frame, inner, pending, &state.theme);
    }
}

/// Pure render helper: converts `DiffLine` slice + scroll offset into
/// coloured `Line` spans and a hint footer. Separated from `render` so
/// it can be unit-tested with a `TestBackend`.
fn render_diff_lines(frame: &mut Frame, area: Rect, pending: &ConfigDiffPending, theme: &Theme) { ... }
```

`centered_rect` is imported from `crate::util` (the shared
`ferro-wg-tui-components/src/util.rs` module). The overlay is **80% wide ×
up to 19 rows** (15 diff lines + 4 chrome), distinguishing it clearly from
`ConfirmDialogComponent` (60% wide × 5 rows).

### Key routing for `EditField` mode

`DiffPreviewComponent` is checked **before** `pending_confirm` in
`handle_key_event` because it has higher z-order (it is the foremost overlay
when both could theoretically be present — though in practice the diff preview
blocks the save confirmation from opening simultaneously).

**Tab-switch exits EditField mode cleanly — no partial edit is committed.**
When `Action::NextTab`, `Action::PrevTab`, or `Action::SelectTab(_)` are
dispatched while `input_mode == InputMode::EditField`, `AppState::dispatch`
automatically exits `EditField` mode: the current field buffer is discarded
(the partial edit is NOT committed) and `input_mode` is set to `Normal`.

```rust
// ferro-wg-tui/src/lib.rs  — updated handle_key_event routing

fn handle_key_event(
    key: KeyEvent,
    state: &mut AppState,
    bundle: &mut ComponentBundle,
    daemon_tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
    config_path: &Path,
) {
    let action = if state.config_diff_pending.is_some() {
        // Diff preview captures all keys while open.
        bundle.diff_preview.handle_key(key, state)
    } else if state.pending_confirm.is_some() {
        bundle.confirm_dialog.handle_key(key, state)
    } else if matches!(state.input_mode, InputMode::Search | InputMode::Import(_)) {
        bundle.status_bar.handle_key(key, state)
    } else if matches!(state.input_mode, InputMode::EditField) {
        // Config field editing: Enter/Esc/char all handled by the Config tab.
        bundle.tabs[Tab::Config.index()].handle_key(key, state)
    } else {
        handle_global_key(key)
            .or_else(|| bundle.connection_bar.handle_key(key, state))
            .or_else(|| bundle.tabs[state.active_tab.index()].handle_key(key, state))
    };
    // ... same follow_up / import_path / dispatch_all / maybe_spawn_command logic
}
```

`bundle.diff_preview` is the new `DiffPreviewComponent` added to
`ComponentBundle`. It is rendered **after** `confirm_dialog` in `render_ui`
so it appears on top of confirmation dialogs:

```rust
// ferro-wg-tui/src/lib.rs  — updated render_ui

terminal.draw(|frame| {
    bundle.tab_bar.render(frame, chunks[0], false, state);
    if show_bar { bundle.connection_bar.render(frame, chunks[1], false, state); }
    bundle.tabs[state.active_tab.index()].render(frame, chunks[2], true, state);
    bundle.status_bar.render(frame, chunks[3], false, state);
    bundle.confirm_dialog.render(frame, chunks[2], false, state);
    bundle.diff_preview.render(frame, chunks[2], false, state); // topmost
})?;
```

### Save flow — step by step

**Backup path note:** `Path::with_extension` replaces the extension, not
appends it (`config.toml` → `config.bak`). To produce `config.toml.bak`,
`with_file_name` is used instead, constructing the backup name by appending
`.bak` to the full file name string.

| Step | Operation | Error case | User-visible message |
|------|-----------|------------|----------------------|
| 1 | `WgConfig::validate(&draft)` | Empty peers, missing allowed-IPs | `"Save blocked: <validation error>"` — diff preview stays open |
| 2 | `tokio::task::spawn_blocking(|| config_toml::load_app_config(&config_path))` | Read/parse failure | `"Save failed: could not read config: <e>"` |
| 3 | `save_app_config_string(&app_config_with_draft)` | Serialisation failure (rare) | `"Save failed: could not serialise config: <e>"` |
| 4 | `tokio::fs::copy(&config_path, &bak_path).await` where `bak_path = config_path.with_file_name(format!("{}.bak", filename))` | Permission denied, disk full | `"Save aborted: backup failed: <e>"` — original never touched |
| 5 | `tokio::fs::write(&config_path, &serialised).await` | Write error | `"Save failed: could not write config: <e>"` — backup exists; manual recovery possible |
| 6 | Send `DaemonMessage::ReloadConfig(new_config, "Saved: <conn_name>")` | — | `DaemonOk("Saved: <conn_name> (backup: config.toml.bak)")` |
| 7 | If `reconnect: true`, send `DaemonMessage::Reconnect(conn_name)` | Daemon unreachable | `DaemonError("Config saved but reconnect failed: daemon not running")` |

Step 4 (backup) is the hard abort gate: if `tokio::fs::copy` returns `Err`, the
function returns immediately without reaching step 5. The user sees a transient
`Feedback::error(...)` message and the diff overlay stays open so they can retry
or discard.

`DaemonMessage::Reconnect(String)` is a reusable semantic: "reconnect this
connection". The save task sends `ReloadConfig` on success and, if
`reconnect: true`, also sends `Reconnect(conn_name)`. On receipt, the event
loop dispatches `Action::DaemonOk("Reconnecting: <name>")` and then spawns a
background task that sends `DaemonCommand::Down { connection_name: Some(name.clone()) }`
followed by `DaemonCommand::Up { connection_name: Some(name) }`, with
`DaemonOk` / `DaemonError` feedback for each.

### Key bindings

#### Conflict analysis

| Key | Existing binding | New binding | Conflict? |
|-----|-----------------|-------------|-----------|
| `q` / `Esc` | Quit (Normal) / cancel (Search/Import) | `DiscardConfigEdits` (EditField/DiffPreview) | No — mode-gated |
| `Tab` / `Right` | NextTab | — exits EditField mode cleanly | No |
| `BackTab` / `Left` | PrevTab | — exits EditField mode cleanly | No |
| `1`–`6` | SelectTab | — exits EditField mode cleanly | No |
| `/` | EnterSearch | — | No |
| `j` / `k` / `↑` / `↓` | NextRow / PrevRow | `ConfigFocusNext/Prev` (EditField on Config tab) / diff scroll | No — mode-gated |
| `e` | (none) | `EnterConfigEdit` (Config tab, Normal mode) | No |
| `+` | (none) | `AddConfigPeer` (Config tab, Normal mode) | No |
| `x` | (none) | Trigger `RequestConfirm { DeletePeer(i) }` (Config tab, peer focused) | No |
| `p` | (none) | `PreviewConfig` (Config tab, Normal mode, no errors) | No |
| `s` | `StartDaemon` (Overview only) | `SaveConfig { reconnect: false }` (DiffPreview only) | No — component-gated |
| `r` | (none) | `SaveConfig { reconnect: true }` (DiffPreview only) | No |
| `Esc` | ExitSearch / ExitImport | `CancelConfigEdit` / `DiscardConfigEdits` | No — mode-gated |

`e`, `+`, `x`, and `p` are safe because they are only routed through the Config
tab component — no other tab registers them. `s` in the diff preview is safe
because `DiffPreviewComponent::handle_key` short-circuits before global key
handling and before the Overview tab component, which is the sole registrant of
`StartDaemon`.

#### Per-context key hint lines

- **Config tab, Normal (interface section focused):**
  `e edit  j/k nav  p preview  + add peer  5 Config tab`
- **Config tab, Normal (peer row focused):**
  `e edit  j/k nav  p preview  + add peer  x delete peer`
- **Config tab, EditField mode:**
  `Enter confirm  Esc cancel  (type to edit)`
- **Diff preview overlay:**
  `s save  r save & reconnect  j/k scroll  Esc discard`
- **Confirm dialog (DeletePeer):**
  `y confirm  n cancel` (existing `ConfirmDialogComponent` — no change)

---

## Implementation Steps (Commits)

### Commit 1 — Core edit types + validators

**Purpose:** Introduce the data layer for config editing. No UI changes yet —
all new code is pure calculations that can be unit-tested in isolation.

**Files:**
- `ferro-wg-tui-core/src/config_edit.rs` — new file: `ConfigSection`,
  `EditableField`, `ConfigEditState`, `DiffLine`, `ConfigDiffPending`;
  all validation pure functions; `config_diff()` pure function;
  `fields_for_section(section, is_new_peer) -> &'static [EditableField]`
- `ferro-wg-tui-core/src/lib.rs` — re-export `config_edit` module and its
  public types (`ConfigSection`, `EditableField`, `ConfigEditState`, `DiffLine`,
  `ConfigDiffPending`, `fields_for_section`, `config_diff`, validation fns) so
  downstream crates import from `ferro_wg_tui_core::config_edit::*`
- `ferro-wg-tui-core/src/action.rs` — add all new `Action` variants listed above
  (`EnterConfigEdit`, `ConfigEditKey`, `CommitConfigEdit`, `CancelConfigEdit`,
  `ConfigFocusNext`, `ConfigFocusPrev`, `ConfigFocusInterface`,
  `ConfigFocusPeer(usize)`, `AddConfigPeer`, `DeleteConfigPeer(usize)`,
  `PreviewConfig`, `ConfigDiffScrollDown`, `ConfigDiffScrollUp`,
  `SaveConfig { reconnect }`, `DiscardConfigEdits`);
  add `ConfirmAction::DeletePeer(usize)`
- `ferro-wg-tui-core/src/app.rs` — add `InputMode::EditField`
- `ferro-wg-tui-core/src/state.rs` — add `config_edit: Option<ConfigEditState>`
  and `config_diff_pending: Option<ConfigDiffPending>` to `AppState`;
  initialize both to `None` in `AppState::new()`; add dispatch arms for
  `EnterConfigEdit`, `ConfigEditKey`, `CommitConfigEdit`,
  `CancelConfigEdit`, `ConfigFocusNext`, `ConfigFocusPrev`,
  `ConfigFocusInterface`, `ConfigFocusPeer(i)`, `AddConfigPeer`,
  `DeleteConfigPeer(i)`, `PreviewConfig`, `ConfigDiffScrollDown`,
  `ConfigDiffScrollUp`, `DiscardConfigEdits`; extend `ConfirmAction::DeletePeer`
  dispatch arm in `ConfirmYes` handler; handle tab-switch actions
  (`NextTab`, `PrevTab`, `SelectTab(_)`) — when `input_mode == EditField`,
  discard the current field buffer and set `input_mode = Normal` before
  processing the tab switch
- `ferro-wg-tui/src/lib.rs` — update `confirmed_action()` to handle
  `ConfirmAction::DeletePeer(i)` by dispatching `Action::DeleteConfigPeer(i)`
  (non-exhaustive match extension — compiler enforces it)

**Tests (all in `ferro-wg-tui-core`):**
- `validate_port`: accepts the empty string `""` (maps to port 0); accepts
  `"0"` as valid random/unspecified port; rejects non-numeric; rejects
  `"65536"`; accepts `"51820"`; accepts `"65535"`
- `validate_addresses`: rejects bare IPs without prefix, invalid CIDR, non-IP
  strings; accepts `"10.0.0.1/24, 192.168.1.2/32"`
- `validate_dns_ips`: rejects non-IP; accepts comma list of IPv4 and IPv6
- `validate_public_key`: rejects 43-char string; rejects 45-char string (too
  long); rejects non-base64; accepts a known 44-char key
- `validate_endpoint`: accepts `"198.51.100.1:51820"`, `"vpn.example.com:51820"`,
  empty; rejects missing port; accepts `"[::1]:51820"` (IPv6)
- `validate_port("65535")` → Ok; `validate_port("65536")` → Err
- `validate_allowed_ips`: `"10.0.0.0/8"` in `other_peers_allowed_ips` with
  input `"10.0.0.0/8"` → Err (exact duplicate); `"10.0.0.1/32"` with other
  `"10.0.0.0/8"` → Ok (overlap but not exact duplicate); rejects non-CIDR
- `config_diff`: identical strings → all `Context` lines; one line changed →
  contains `Removed` and `Added`; new line added → contains `Added`
- `fields_for_section(Interface, _)` returns all 10 interface fields
- `fields_for_section(Peer(0), false)` omits `PeerPublicKey` (5 fields)
- `fields_for_section(Peer(0), true)` includes `PeerPublicKey` as first field
  (6 fields)
- `AppState::dispatch(EnterConfigEdit)`: pressing `e` when
  `active_connection()` is `Some` and a mutable field is focused — `config_edit`
  must be `None` before the action is dispatched; after
  `dispatch(EnterConfigEdit)`, assert `config_edit.is_some()` and
  `input_mode == EditField`
- `AppState::dispatch(CancelConfigEdit)`: returns to `Normal` mode, clears
  `edit_buffer`
- `AppState::dispatch(ConfigFocusNext)`: increments `focused_field_idx` within
  bounds
- `AppState::dispatch(ConfigFocusNext)` when focus is on the last interface
  field → `focused_field_idx` wraps to 0
- `AppState::dispatch(AddConfigPeer)`: appends blank peer to draft, enters
  `EditField` on `PeerPublicKey`
- `AppState::dispatch(DeleteConfigPeer(0))` → `RequestConfirm { DeletePeer(0) }`
  stored in `pending_confirm`
- `AppState::dispatch(ConfirmYes)` after `DeletePeer(0)`: removes peer 0 from
  draft
- `AppState::dispatch(ConfirmYes)` after `DeletePeer(0)` with 1-peer draft:
  draft has 0 peers and `focused_field_idx` is clamped to 0
- `AppState::dispatch(PreviewConfig)`: valid draft → populates
  `config_diff_pending`; draft with empty peers → `Feedback::error` set,
  `config_diff_pending` remains `None`
- Tab-switch while `input_mode == EditField`: dispatching `NextTab` discards
  the buffer and sets `input_mode = Normal`

---

### Commit 2 — Config tab interactive navigation and edit mode

**Purpose:** Make `ConfigComponent` interactive. Field focus navigation and
inline editing work end-to-end; no save plumbing yet.

**Files:**
- `ferro-wg-tui-components/src/config.rs` — replace `ConfigComponent` body:
  add `focused_section: ConfigSection` and `focused_field_idx: usize` as
  struct fields (component-local, not in `AppState`); implement `handle_key`
  for `j`/`k` (focus movement), `e` (emit `EnterConfigEdit`), `+` (emit
  `AddConfigPeer`), `x` (emit `RequestConfirm { DeletePeer(i) }`),
  `p` (emit `PreviewConfig`), `Esc` (emit `DiscardConfigEdits`);
  render each field as a `[focused]` → highlighted row or `[editing]` →
  text input row with buffer content and cursor `█`; show `(read-only)` in
  `theme.muted` for `PrivateKey` and for `PeerPublicKey` on existing peers;
  show `field_error` beneath the active field in `theme.error` when `Some`
- `ferro-wg-tui-components/src/status_bar.rs` — add Config tab hints for
  Normal and EditField modes

**Tests:**
- `ConfigComponent::handle_key` returns `EnterConfigEdit` when `e` pressed
  and `active_connection()` is `Some` and a mutable field is focused
  (`config_edit` is `None` before the action)
- `ConfigComponent::handle_key` returns `AddConfigPeer` when `+` pressed
- `ConfigComponent::handle_key` returns `RequestConfirm { DeletePeer(0) }` when
  `x` pressed and a peer is focused
- `ConfigComponent::handle_key` returns `PreviewConfig` when `p` pressed
- Render snapshot (TestBackend 80×24): no-connection state shows placeholder;
  focused field highlights; `(read-only)` appears for private key; `field_error`
  appears beneath the field when set
- Full roundtrip key sequence: dispatch `EnterConfigEdit`; assert
  `edit_buffer == Some("51820")`; dispatch
  `ConfigEditKey(KeyEvent::from(KeyCode::Char('1')))` → buffer becomes
  `"518201"`; dispatch `ConfigEditKey(KeyEvent::from(KeyCode::Backspace))` ×5
  → buffer empty; dispatch `ConfigEditKey(Char('5'))`,
  `ConfigEditKey(Char('1'))`, `ConfigEditKey(Char('8'))`,
  `ConfigEditKey(Char('2'))`, `ConfigEditKey(Char('1'))` → buffer `"51821"`;
  dispatch `CommitConfigEdit`; assert `draft.interface.listen_port == 51821`

---

### Commit 3 — `DiffPreviewComponent` overlay

**Purpose:** Introduce the diff overlay component and wire it into
`ComponentBundle` and `render_ui` / `handle_key_event`.

**Files:**
- `ferro-wg-tui-components/src/util.rs` — new file: `pub(crate) fn
  centered_rect(pct_x: u16, height: u16, area: Rect) -> Rect` shared utility
- `ferro-wg-tui-components/src/diff_preview.rs` — new file:
  `DiffPreviewComponent` with `handle_key` (s/r/j/k/Esc) and `render`
  (80%-wide scrollable overlay, `Added` → `theme.ok`, `Removed` → `theme.error`,
  `Context` → `theme.muted`, hint footer); uses `crate::util::centered_rect`;
  private `render_diff_lines` helper
- `ferro-wg-tui-components/src/lib.rs` — export `DiffPreviewComponent`;
  export `util` module; update `ConfirmDialogComponent` to import
  `centered_rect` from `crate::util` instead of its private copy
- `ferro-wg-tui/src/lib.rs` — add `diff_preview: DiffPreviewComponent` to
  `ComponentBundle`; update `handle_key_event` routing to check
  `config_diff_pending.is_some()` first; render `diff_preview` last in
  `render_ui` (topmost overlay)

**Tests:**
- `DiffPreviewComponent::handle_key` returns `None` when `config_diff_pending`
  is `None`
- `s` → `SaveConfig { reconnect: false }`; `r` → `SaveConfig { reconnect: true }`
- `j` / `↓` → `ConfigDiffScrollDown`; `k` / `↑` → `ConfigDiffScrollUp`
- `Esc` / `q` → `DiscardConfigEdits`
- `ConfigDiffScrollUp` when `scroll_offset == 0` → `scroll_offset` stays 0
  (no underflow; uses `saturating_sub(1)`)
- `ConfigDiffScrollDown` clamps at `diff_lines.len().saturating_sub(1)` (no
  overrun)
- Render snapshot: no overlay when `config_diff_pending` is `None`; overlay
  renders correct title and hint line when `Some`; `Added` lines begin with `+`;
  `Removed` lines begin with `-`; scroll offset advances visible window

**Scroll bounds:** `ConfigDiffScrollUp` uses `scroll_offset.saturating_sub(1)`;
`ConfigDiffScrollDown` uses
`scroll_offset.min(diff_lines.len().saturating_sub(1))`.

---

### Commit 4 — Background save task and backup

**Purpose:** Wire the save flow end-to-end: backup → write → reload → optional
reconnect. No UI changes beyond feedback messages.

**Files:**
- `ferro-wg-tui/src/lib.rs` — add `DaemonMessage::Reconnect(String)` variant;
  add `spawn_save_task(draft, conn_name, config_path, reconnect, daemon_tx,
  tasks)` following the `spawn_import_task` shape;
  `maybe_spawn_command` dispatches `SaveConfig { reconnect }` to this task;
  handle `DaemonMessage::Reconnect` in `handle_daemon_messages` by dispatching
  `Action::DaemonOk("Reconnecting: {name}")` and spawning a reconnect task
  that sends `Down` then `Up` for the named connection
- `ferro-wg-tui-core/src/state.rs` — `dispatch(SaveConfig)` clears
  `config_diff_pending` optimistically (feedback message set on
  `ReloadConfig` or `DaemonError`; a subsequent `DaemonMessage::CommandError`
  does NOT restore the overlay — shows `Feedback::error` instead)

**`spawn_save_task` signature and logic:**

```rust
// ferro-wg-tui/src/lib.rs

fn spawn_save_task(
    draft: WgConfig,
    conn_name: String,
    config_path: PathBuf,
    reconnect: bool,
    daemon_tx: mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    tasks.spawn(async move {
        spawn_save_task_inner(draft, conn_name, config_path, reconnect, daemon_tx).await
    });
}

async fn spawn_save_task_inner(
    draft: WgConfig,
    conn_name: String,
    config_path: PathBuf,
    reconnect: bool,
    daemon_tx: mpsc::UnboundedSender<DaemonMessage>,
) {
    // Step 1: validate
    if let Err(e) = draft.validate() {
        let _ = daemon_tx.send(DaemonMessage::CommandError(
            format!("Save blocked: {e}")
        ));
        return;
    }
    // Step 2: load current app config (sync fn — must run in spawn_blocking)
    let config_path_clone = config_path.clone();
    let mut app_config = match tokio::task::spawn_blocking(move || {
        config_toml::load_app_config(&config_path_clone)
    }).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            // Config parse or validation error.
            let _ = daemon_tx.send(DaemonMessage::CommandError(
                format!("Save failed: could not read config: {e}")
            ));
            return;
        }
        Err(join_err) => {
            // spawn_blocking task panicked.
            let _ = daemon_tx.send(DaemonMessage::CommandError(
                format!("Save failed: config read task panicked: {join_err}")
            ));
            return;
        }
    };
    app_config.insert(conn_name.clone(), draft);
    // Step 3: serialise (dry-run before touching disk)
    let serialised = match config_toml::save_app_config_string(&app_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = daemon_tx.send(DaemonMessage::CommandError(
                format!("Save failed: could not serialise config: {e}")
            ));
            return;
        }
    };
    // Step 4: backup (abort on failure — never write without backup)
    // Use with_file_name to append ".bak" to the full filename, not replace
    // the extension. with_extension("toml.bak") would replace the extension
    // ("config.toml" → "config.bak"), which is wrong. with_file_name produces
    // "config.toml.bak" correctly.
    let bak_path = config_path.with_file_name(
        format!("{}.bak", config_path.file_name().unwrap_or_default().to_string_lossy())
    );
    if let Err(e) = tokio::fs::copy(&config_path, &bak_path).await {
        let _ = daemon_tx.send(DaemonMessage::CommandError(
            format!("Save aborted: backup failed: {e}")
        ));
        return;
    }
    // Step 5: write (async — no blocking I/O in async context)
    if let Err(e) = tokio::fs::write(&config_path, &serialised).await {
        let _ = daemon_tx.send(DaemonMessage::CommandError(
            format!("Save failed: could not write config: {e} (backup: config.toml.bak)")
        ));
        return;
    }
    // Step 6: reload
    let ok_msg = format!("Saved: {conn_name} (backup: config.toml.bak)");
    let _ = daemon_tx.send(DaemonMessage::ReloadConfig(app_config, ok_msg));
    // Step 7: optionally reconnect
    if reconnect {
        let _ = daemon_tx.send(DaemonMessage::Reconnect(conn_name));
    }
}
```

**Tests:**
- `spawn_save_task` with a valid draft, temp config path → writes file, creates
  `.bak` (filename is `config.toml.bak`, not `config.bak`), sends `ReloadConfig`;
  verify both paths exist and `ReloadConfig` carries the updated config
- `spawn_save_task` with invalid draft (empty peers) → sends `CommandError`,
  no files written
- `spawn_save_task` when backup path is unwritable (read-only dir) → sends
  `CommandError("Save aborted: backup failed: …")`, original config unchanged
- `spawn_save_task` with `reconnect: true` on a valid draft sends both
  `DaemonMessage::ReloadConfig` and `DaemonMessage::Reconnect(conn_name)` on
  the channel, in that order
- `dispatch(SaveConfig { reconnect: false })` clears `config_diff_pending`
  optimistically
- `dispatch(SaveConfig { reconnect: false })` followed by
  `DaemonMessage::CommandError` does NOT restore `config_diff_pending` —
  shows `Feedback::error` instead
- `handle_daemon_messages(Reconnect("mia"))` dispatches
  `DaemonOk("Reconnecting: mia")` and sends Down then Up for `"mia"`

---

### Commit 5 — Peer editing and add/delete flows

**Purpose:** Complete the editing surface — peer field navigation, new-peer
creation, and peer deletion with confirmation.

**Files:**
- `ferro-wg-tui-components/src/config.rs` — extend `handle_key` and `render`
  for peer section focus; render each peer as a collapsible sub-list with
  `focused_section == Peer(i)` highlighting; `PeerPublicKey` field shows as
  `(read-only)` in `theme.muted` for existing peers and as an editable
  required field (with `*` required indicator) for newly added peers;
  `x` key on focused peer row emits `RequestConfirm { action:
  ConfirmAction::DeletePeer(i) }`
- `ferro-wg-tui-core/src/state.rs` — `dispatch(ConfirmYes)` when
  `pending_confirm.action == DeletePeer(i)`: remove peer `i` from
  `config_edit.draft.peers`, clamp `focused_field_idx`

**Tests:**
- `dispatch(AddConfigPeer)`: draft gains one peer; `focused_section ==
  Peer(new_idx)`; `input_mode == EditField`; `edit_buffer == Some("")`
- `dispatch(ConfirmYes)` after `DeletePeer(1)` on a 3-peer draft: draft has 2
  peers; `focused_field_idx` clamped to valid range
- `dispatch(ConfirmYes)` after `DeletePeer(0)` with 1-peer draft: draft has 0
  peers and `focused_field_idx` clamped to 0
- Full roundtrip integration test: construct `AppState` with a 2-connection
  config; dispatch `EnterConfigEdit` + `ConfigEditKey` chars + `CommitConfigEdit`
  to change listen port; dispatch `PreviewConfig`; assert `config_diff_pending`
  contains a `Removed` line for the old port and an `Added` line for the new
  port; dispatch `DiscardConfigEdits`; assert `config_diff_pending` is `None`
  and `config_edit` is `None`

---

## Tooling Checklist (per commit)

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings -D clippy::pedantic
cargo test --workspace --features boringtun,neptun,gotatun
```

Linux-gated code note: if any `#[cfg(target_os = "linux")]` blocks are touched
(e.g. `fwmark` handling), wait for CI (Linux runner) before declaring the commit
done — macOS cannot lint those paths locally.

---

## Verification

```bash
# Manual smoke-test (full config edit cycle)
ferro-wg tui
  # Navigate to Config tab (key 5)
  # Press j/k — field focus moves; PrivateKey shows "(read-only)"
  # Press e on ListenPort — buffer pre-fills with current port; cursor visible
  # Type an invalid value (e.g. "99999") — inline error appears beneath field
  # Type a valid value (e.g. "51821") — error clears
  # Press Enter — field commits; mode returns to Normal-within-Config
  # Press p — diff overlay appears: old port line in red, new port in green
  # Press Esc — diff discarded; Config tab returns to read-only view
  # Press e → Enter (no change) → p → s — backup created; success feedback shown
  # Verify config.toml updated and config.toml.bak exists beside it
  # Press + — blank peer row added; cursor in PublicKey field
  # Type valid base64 public key (44 chars) → Enter — moves to next peer field
  # Press p → r — saves and reconnects; DaemonOk feedback for Down then Up
  # Press 5 → j to a peer row → x — confirm dialog appears
  # Press y — peer removed from draft (not yet saved)
  # Press p → s — final config without deleted peer written to disk
```

---

## File Summary

| File | Commits |
|---|---|
| `ferro-wg-tui-core/src/config_edit.rs` | 1 (new) |
| `ferro-wg-tui-core/src/action.rs` | 1 |
| `ferro-wg-tui-core/src/app.rs` | 1 |
| `ferro-wg-tui-core/src/state.rs` | 1, 4, 5 |
| `ferro-wg-tui-core/src/lib.rs` | 1 |
| `ferro-wg-tui-components/src/util.rs` | 3 (new) |
| `ferro-wg-tui-components/src/config.rs` | 2, 5 |
| `ferro-wg-tui-components/src/status_bar.rs` | 2 |
| `ferro-wg-tui-components/src/diff_preview.rs` | 3 (new) |
| `ferro-wg-tui-components/src/lib.rs` | 3 |
| `ferro-wg-tui/src/lib.rs` | 1, 3, 4 |
