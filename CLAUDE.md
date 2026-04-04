# Agent Guidelines for ferro-wg

These rules **MUST** be followed by all AI coding agents and contributors.

**Before writing any Rust code, invoke `/rust-code-writer`.** For deeper reference:
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
- **MUST** run `cargo fmt --all` immediately before every `git commit` — never commit unformatted code
- **SHOULD** keep commits < 250 lines; justify if exceeded
- **SHOULD** keep PRs < 500 lines; justify if exceeded
- **SHOULD** prefer flat code; refactor when nesting exceeds 3-4 levels

## Tooling Checklist (Before Committing)

**Format first — this is mandatory, not optional:**

```
cargo fmt --all
```

Then verify the full checklist:

- [ ] `cargo fmt --all --check` — confirms no formatting drift
- [ ] `cargo test --workspace --features boringtun,neptun,gotatun` — all tests pass
- [ ] `cargo build --workspace` — no warnings
- [ ] `cargo clippy --all-targets --all-features -- -D warnings -D clippy::pedantic` — clean (matches CI exactly)
  - `--all-targets` covers lib, bins, tests, benches, and examples — not just the default target
  - `--all-features` enables every feature flag so no code path is skipped
- [ ] All public items have doc comments
- [ ] No commented-out code or debug statements
- [ ] No hardcoded credentials

## Platform-gated code (`#[cfg(target_os = ...)]`)

macOS dev machines cannot lint Linux-gated code locally — faking the
target with `RUSTFLAGS='--cfg target_os="linux"'` breaks `libc` and
other system deps. **After any change inside a `#[cfg(target_os = "linux")]`
block, wait for CI (Linux runner) to pass before declaring the work
done.** Do not merge until the CI clippy step is green.

---

**Remember:** Prioritize clarity and maintainability over cleverness.
