## Plan Mode

- Make the plan extremely concise. Sacrifice grammar for the sake of concision.
- At the end of each plan, give me a list of unresolved questions to answer, if any.

## Agent Skills (agentskills.io Standard)

This project uses the official [Agent Skills](https://agentskills.io) format.  
All skills are located in `.agents/skills/`.

### Core Skills

- **`code-writer`**  
  Foundational skill for all code generation and refactoring.  
  Embodies the project's coding philosophy inspired by *Grokking Simplicity* and *SICP*.

- **`rust-code-writer`**  
  General Rust coding standards and practices.  
  Enforces idiomatic Rust, functional purity, stratified design, and flat combinator style.

- **`rust-errors`**  
  Clean, layered error handling patterns using `thiserror` and proper `From` impls.

### Backend Skills

- **`rust-axum-backend`**  
  Specialized rules for building HTTP APIs and web servers with Axum.  
  Includes handler patterns, layered extractors, middleware, shared state, and offloading CPU work.

### Frontend Skills

- **`rust-frontend`**  
  Specialized rules for Rust-based frontends using WASM, Leptos, and data processing with Polars.  
  Covers UI styling (Pico CSS), theming, typography, WASM build process, and Polars usage.

### Review & Quality Skills

- **`rust-code-reviewer`**  
  Obsessive, ruthless Rust code review skill.  
  Applies strict enforcement of all above principles with high pedantry.

## Skill Usage Rules

- For **any general code task**: Start with `code-writer`
- For **any Rust task**: Always combine `code-writer` + `rust-code-writer`
- For **Rust backend / HTTP APIs**: Add `rust-axum-backend`
- For **Rust frontend / WASM / Leptos / Polars**: Add `rust-frontend`
- For **error handling design**: Add `rust-errors`
- For **code review or quality check**: Use `rust-code-reviewer` ruthlessly

**Example activation**:
> Using `code-writer` + `rust-code-writer` + `rust-axum-backend` to implement the new login endpoint.

> Using `code-writer` + `rust-code-writer` + `rust-frontend` for the dashboard UI.

---

**All agents and contributors must follow the skills referenced above.**  
This AGENT.md serves as the single source of truth for project coding standards and agent behavior.

Last updated: March 2026
