# Phase 7: UX Polish — Implementation Progress

## Completed Phases

### Commit 1: Catppuccin color palette + ThemeKind + theme toggle (COMPLETED)
- Added ThemeKind enum with Mocha/Latte variants, into_theme(), toggle() in ferro-wg-tui-core/src/theme.rs
- Replaced all placeholder Color::* constants with official Catppuccin 24-bit RGB values in both mocha() and latte()
- Added ToggleTheme, ShowHelp, HideHelp variants to Action enum in ferro-wg-tui-core/src/action.rs
- Added theme_kind: ThemeKind and show_help: bool to AppState in ferro-wg-tui-core/src/state.rs with dispatch logic
- Added 'T' and '?' keybindings for ToggleTheme and ShowHelp in ferro-wg-tui/src/lib.rs handle_global_key
- Added insta = { version = "1", features = ["yaml"] } to dev-dependencies in both Cargo.toml files
- Comprehensive unit tests for theme toggle, color values, dispatch actions
- Snapshot tests for TabBarComponent at 80x24 with both Mocha and Latte themes
- Refactored dispatch() to eliminate clippy::too_many_lines allow and improve stratification
- All tests pass, clippy clean, no warnings

### Commit 2: Toast queue (replaces single-slot Feedback) (COMPLETED)
- Replaced AppState::feedback: Option<Feedback> with toasts: VecDeque<Toast>
- Added Toast struct with success/error constructors, is_expired method
- Added TOAST_DURATION (3s), MAX_VISIBLE_TOASTS (5) constants
- Added push_toast (FIFO eviction) and clear_expired_toasts to AppState
- Updated dispatch for DaemonOk/DaemonError to push_toast instead of setting feedback
- Added ToastComponent: handle_key None, renders bottom-right with Clear underlay
- Integrated ToastComponent into ComponentBundle and render_ui (above content)
- Removed feedback rendering branch from StatusBar
- Comprehensive unit tests for toast lifecycle, eviction, expiration, dispatch
- Snapshot tests for ToastComponent at 80x24 with multiple toasts
- All tests pass, clippy clean, no warnings

### Commit 3: Help overlay component (COMPLETED)
- Added KEYBINDINGS constant in ferro-wg-tui-core/src/ux.rs with 37 keybinding entries
- Extracted centered_rect to ferro-wg-tui-components/src/util.rs
- Added HelpOverlayComponent: renders two-column Table in Clear-backed overlay at 90% width x min(height/2, 30) rows
- Handle_key returns HideHelp for ?/Esc/q when show_help, swallows other keys
- Wired into ComponentBundle, resolve_key_action guard, render_ui topmost, dispatch_all
- Comprehensive tests for handle_key logic, KEYBINDINGS validation, render snapshots at 80x24 and 120x40
- All tests pass, clippy clean, no warnings

### Commit 4: Mouse support (COMPLETED)
- Added AppEvent::Mouse(crosstrom::event::MouseEvent) in ferro-wg-tui/src/event.rs
- Forwarded Event::Mouse in event-loop maybe_event arm
- Enabled EnableMouseCapture after EnterAlternateScreen, DisableMouseCapture on cleanup
- Added resolve_mouse_action fn in ferro-wg-tui-core/src/ux.rs: ScrollDown/Up -> NextRow/PrevRow, left-click tab bar -> SelectTab via tab_hit_test
- Added tab_hit_test and tab_label_at_column using Tab::ALL and title() at compile time
- Added handle_mouse default method to Component trait in ferro-wg-tui-core/src/component.rs
- Computed layout before event handling in event loop for tab bar rects
- Added guards: ignore mouse when show_help || pending_confirm || config_diff_pending
- Comprehensive tests for mouse actions, tab hit tests, guards
- All tests pass, clippy clean, no warnings

### Commit 5: Responsive layout (80×24 minimum) (COMPLETED)
- Added MIN_TERMINAL_WIDTH: u16 = 80 and MIN_TERMINAL_HEIGHT: u16 = 24 constants
- Added render_too_small fn: centered 'Terminal too small (min 80×24)' with theme.error
- Added size guard at top of render_ui: if area.width < MIN_TERMINAL_WIDTH || area.height < MIN_TERMINAL_HEIGHT, render_too_small and return
- Added early return guards in all tab components render: if area.height == 0 || area.width < 20, return
- Verified StatusComponent Layout::split uses Constraint::Min(0) for peer table collapse
- Comprehensive insta snapshot tests for all components at 80x24 and 120x40, render_ui at minimum sizes
- compute_layout unit tests for 80x24 layout validation
- All tests pass, clippy clean, no warnings

## Verification Status
- Tooling checks: PASSED (fmt, test, clippy, build)
- Adversary reviews: PASSED (reviewer, tester, architect)

## Phase 7 Complete
Phase 7 UX Polish is fully verified and committed. Summary of all fixes and additions:

- Catppuccin Mocha/Latte themes fully applied with RGB values, 'T' toggle keybinding
- Toast queue replaces single-slot feedback, up to 5 visible toasts expiring after 3s
- Help overlay on '?', full keybindings in two-column table, modal with Clear background
- Mouse support: tab clicks navigate, scroll wheel moves rows, with modal guards
- Responsive layout enforces 80×24 minimum, graceful degradation with too-small message

## Verification Status
- Tooling checks: PASSED (fmt, test, clippy, build)
- Adversary reviews: PASSED (reviewer, tester, architect)