# Agent Guidelines for ferro-wg

These rules **MUST** be followed by all AI coding agents and contributors.

**Before writing any Rust code, invoke `/rust-practices`.** For deeper reference:
- `/rust-crates` — preferred crates by purpose
- `/rust-style` — nesting, combinators, documentation template
- `/rust-errors` — error type structure, `From`/`Into` conversion patterns

## Core Principles

All code you write **MUST** be fully optimized:
- Maximize algorithmic big-O efficiency for memory and runtime
- Use parallelization and SIMD where appropriate
- Follow DRY — maximize code reuse
- No extra code beyond what is necessary (no technical debt)

Workflow requirements:
- **MUST** work in small, coherent, testable batches
- **MUST** write tests for all code written
- **MUST** run all tests before handoff
- **MUST** ensure code compiles without warnings
- **SHOULD** keep commits < 250 lines; justify if exceeded
- **SHOULD** keep PRs < 500 lines; justify if exceeded
- **SHOULD** prefer flat code; refactor when nesting exceeds 3-4 levels

## Tooling Checklist (Before Committing)

- [ ] `cargo test --workspace --features boringtun,neptun,gotatun` — all tests pass
- [ ] `cargo build --workspace` — no warnings
- [ ] `cargo clippy --workspace --features boringtun,neptun,gotatun -- -W clippy::pedantic -D warnings` — clean
- [ ] `cargo fmt --all --check` — formatted
- [ ] All public items have doc comments
- [ ] No commented-out code or debug statements
- [ ] No hardcoded credentials

---

**Remember:** Prioritize clarity and maintainability over cleverness.
