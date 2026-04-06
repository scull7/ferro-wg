# TUI Improvements Plan

## Overview
Enhance the ferro-wg TUI beyond Phase 7 (UX Polish) with targeted improvements addressing TODOs, performance, and UX refinements. Focus on low-effort wins and maintainability without major rewrites.

## Suggested Improvements

### 1. Backend Cycling Implementation (New Feature)
- **Rationale**: `CyclePeerBackend` action hardcodes Neptun; users should cycle through available backends.
- **Implementation**: Modify `PeersComponent::handle_key` to check `peer.backend` and cycle to next available (Boringtun → Neptun → Gotatun → Boringtun).
- **Effort**: Small; requires backend availability check.

### 2. Enhanced Error Feedback (UX Improvement)
- **Rationale**: Current toasts are generic; provide specific messages for daemon timeouts vs. auth failures.
- **Implementation**: Extend `DaemonError` enum with variants; update dispatch to map errors to user-friendly messages.
- **Effort**: Small; leverages existing toast system.

### 3. Persistent Search State (UX Improvement)
- **Rationale**: Search resets on tab switch; save per-tab search queries in `AppState`.
- **Implementation**: Add `search_per_tab: HashMap<Tab, String>` to `AppState`; update on search input and tab change.
- **Effort**: Small; pure state addition.

### 4. Log Filtering Optimization (Performance)
- **Rationale**: Log filtering recomputes on every render; cache filtered results.
- **Implementation**: Add `filtered_log_cache: Option<Vec<&LogEntry>>` to `LogsComponent`; invalidate on new logs/search changes.
- **Effort**: Medium; requires cache invalidation logic.

### 5. Remove TODOs and Debt Cleanup (Maintainability)
- **Rationale**: Address remaining TODOs (e.g., multi-connection TUI expansion via connection bar).
- **Implementation**: Implement connection bar multi-selection; remove hardcoded backend cycles.
- **Effort**: Medium; modular.

### 6. Integration Test Expansion (Testing)
- **Rationale**: Add async event loop tests for full flows (benchmark, config edit).
- **Implementation**: Use tokio test framework for daemon-mocked scenarios.
- **Effort**: Medium; builds on existing test patterns.

## Implementation Phases

### Phase 1: Backend Cycling + Error Feedback
- Implement backend cycling in PeersComponent
- Extend error variants and toast mapping

### Phase 2: Persistent Search + Log Caching
- Add per-tab search state
- Implement filtered log caching

### Phase 3: Debt Cleanup + Testing
- Remove TODOs and implement multi-connection expansions
- Add integration tests for async flows

## Verification
- All changes maintain existing tests passing
- New features tested with unit/integration coverage
- Performance benchmarks show no regression
- UX validated with manual testing

## Effort Estimate
- Total: Medium (spread across 3 small phases)
- Focus on quick wins with high impact/low risk</content>
<parameter name="filePath">docs/tui-improvements.md