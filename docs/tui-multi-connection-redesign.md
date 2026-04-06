# TUI Multi-Connection Redesign Plan

## Overview
Redesign tab flow to display information for all connections by default, with user-controlled filtering via a searchable toggle overlay. Eliminates buggy "selected connection" switching. Start with Status tab as prototype.

## Core Changes
- Remove global `active_connection` selection from `AppState`.
- Add `visible_connections: HashSet<String>` to `AppState` (default: all).
- Filter key (e.g., `f`) opens searchable toggle overlay for connection visibility.
- Each tab renders filtered data; overlays render topmost.

## Tab-by-Tab Breakdown

### Status Tab (Prototype)
- Show all visible connections in a single table: name, status, backend, endpoint, interface, tx/rx, last handshake, health warnings.
- Rows grouped by connection; filter applies to entire connections (hide/show).
- Keep existing keybindings: `u`/`d` for up/down per row, `b` for backend cycle.
- No per-connection selection; actions target the row's connection.

### Other Tabs (Future Phases)
- **Overview**: Aggregate health table for visible connections; filter hides rows.
- **Peers**: Multi-row peer list (connection.name + peer details); filter by connection.
- **Config**: Editable forms for visible connections; filter as selector (only one editable at a time).
- **Logs**: Filtered log stream; filter applies to connection_name.
- **Compare**: Benchmark views for visible connections.

## Implementation Phases

### Phase 1: Status Tab Prototype
- Update `StatusComponent` to render all connections, filtered by `visible_connections`.
- Add filter overlay component (`ConnectionFilterOverlay`) with search/toggle.
- Modify `AppState` dispatch for filter actions.
- Unit/integration tests for rendering and filtering.

### Phase 2: Core Infrastructure
- Remove `active_connection` from `AppState` and components.
- Update all tabs to use `visible_connections` filtering.
- Ensure overlays integrate with existing modal guards.

### Phase 3: Remaining Tabs + Polish
- Implement per-tab adaptations (e.g., Config editing guard).
- Add tests; verify no regressions.

## Edge Cases & Error Handling
- No connections: Filter overlay shows "no connections" message; tabs render empty states.
- Empty visible_connections: Tabs show "no visible connections" placeholder.
- Filter overlay integration: Respects modal guards (no input during help/diff); renders topmost with clear underlay.
- Config tab: Filter acts as selector—only one connection editable at a time; guard prevents multi-edit.
- Error flows: Invalid filter actions (e.g., toggle non-existent connection) emit toasts; daemon errors pass through unchanged.

## Testing Additions
- Snapshot tests for StatusComponent and ConnectionFilterOverlay.
- Unit tests for empty visible_connections (e.g., filter overlay, tab rendering).
- Integration tests for filter dispatch actions.

## Verification
- Status tab shows all connections with working filter overlay.
- No connection selection bugs.
- Maintains <1s daemon refresh; tests pass.</content>
<parameter name="filePath">docs/tui-multi-connection-redesign.md