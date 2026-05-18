# Repository Guidelines

## Project Structure & Module Organization

This repository currently contains the implementation brief in
`wcodex_container_implementation.md`. Treat it as the source of truth until the
Rust crate is scaffolded.

When implementation begins, use the module layout described in the brief:

```text
src/
  main.rs
  cli.rs
  config.rs
  engine_container.rs
  image.rs
  paths.rs
  probe.rs
  run.rs
  state.rs
  doctor.rs
tests/
```

Keep generated runtime image assets, such as `Containerfile` and
`entrypoint.sh`, produced from code or temporary build contexts rather than
committed as ad hoc files unless the design changes.

## Build, Test, and Development Commands

No `Cargo.toml` exists yet. After creating the Rust crate, use standard Cargo
commands:

- `cargo build`: compile the CLI locally.
- `cargo test`: run unit tests, especially argv construction and config tests.
- `cargo fmt`: format Rust sources with `rustfmt`.
- `cargo clippy --all-targets --all-features`: run lint checks before review.
- `WCODEX_INTEGRATION=1 cargo test`: run integration tests that require Apple
  `container` and Codex runtime behavior.

## Coding Style & Naming Conventions

Use idiomatic Rust with four-space indentation and `rustfmt` defaults. Prefer
small modules with explicit responsibilities matching the proposed `src/`
layout. Use `snake_case` for functions, modules, and variables; `PascalCase`
for types; and `SCREAMING_SNAKE_CASE` for constants.

Build process invocations with `std::process::Command` and structured
arguments. Do not join user-provided args into shell strings.

## Testing Guidelines

Add unit tests for command argv generation, path hashing, config generation,
and safety invariants. Name tests by behavior, for example
`does_not_mount_host_home` or `preserves_codex_args_exactly`.

Gate runtime tests behind `WCODEX_INTEGRATION=1`. Integration tests may assume
Apple `container`, network access, image build support, and a Codex install.

## Commit & Pull Request Guidelines

This checkout has no Git history, so no existing commit convention is available.
Use concise imperative commit subjects, for example `Add sandbox probe
selection`.

Pull requests should include a short summary, test results, and any security
impact. Link related issues when available. For CLI behavior changes, include
example invocations and relevant output.

## Security & Configuration Tips

Preserve the trust model in the brief. Do not mount the host home directory,
host `~/.codex`, `~/.ssh`, Docker socket, cloud credentials, keychains, or
package-manager credentials by default. SSH forwarding and `danger-full-access`
fallbacks must remain explicit opt-ins.
