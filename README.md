# wcodex

This repository contains a Rust CLI wrapper that runs OpenAI Codex inside
Apple's `container` runtime on macOS.

`wcodex` runs Codex inside Apple's `container` runtime with a narrow host mount
set. It mounts the current repository at `/workspace`, an isolated Codex home at
`/root/.codex`, a per-repository cache at `/cache`, and a tmpfs at `/tmp`.

The generated runtime image installs Codex directly from the
`openai/codex` GitHub release binaries for Linux:

- `codex-x86_64-unknown-linux-musl.tar.gz`
- `codex-aarch64-unknown-linux-musl.tar.gz`

The image is based on Debian and does not include Node.js.

## Prerequisites

- macOS with Apple's `container` CLI installed and working.
- Rust and Cargo.
- Network access for the first runtime image build.

## Build

```bash
cargo build
```

For an optimized local binary:

```bash
cargo build --release
```

## Test

Run the unit tests:

```bash
cargo test
```

Run formatting and lint checks before review:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features
```

Runtime integration tests require Apple's `container` CLI, network access, image
build support, and a working Codex login flow. They are gated behind
`WCODEX_INTEGRATION=1`:

```bash
WCODEX_INTEGRATION=1 cargo test
```

## Install on macOS

Install directly from this checkout into Cargo's bin directory:

```bash
cargo install --path .
```

Make sure Cargo's bin directory is on your shell path:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

Alternatively, install the release binary into a local bin directory:

```bash
cargo build --release
mkdir -p "$HOME/.local/bin"
install -m 0755 target/release/wcodex "$HOME/.local/bin/wcodex"
```

Then make sure `$HOME/.local/bin` is on `PATH`.

## First Use

Log in to Codex inside the isolated wcodex home:

```bash
wcodex login
```

Build the runtime image explicitly:

```bash
wcodex build-image --pull
```

The default runtime image tag is `wcodex-runtime:latest`. The wrapper still
records a local implementation fingerprint under `~/.wcodex/images/` so changes
to the generated Containerfile trigger a rebuild without putting a hash in the
image tag.

To refresh the direct Codex binary download when using the default
`CODEX_VERSION=latest`, force a rebuild:

```bash
wcodex build-image --pull --no-cache
```

Run Codex in the current repository:

```bash
wcodex
```

Pass Codex arguments through as usual:

```bash
wcodex exec "fix the tests"
wcodex --model gpt-5 "explain this repo"
```

Inspect resolved state and mount paths:

```bash
wcodex paths
```

Run diagnostics:

```bash
wcodex doctor
```

## State

`wcodex` stores persistent state under:

```text
~/.wcodex/
```

The isolated Codex home is:

```text
~/.wcodex/auth/codex-home
```

Treat this directory as sensitive because it contains Codex authentication data.
