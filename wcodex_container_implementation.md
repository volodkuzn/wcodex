# `wcodex`: Implementation Brief for Wrapping Codex CLI with Apple `container`

Generated: 2026-05-15

This document is intended to be handed to Codex CLI during implementation. It describes a small Rust CLI tool, tentatively named `wcodex`, that runs Codex inside Apple’s `container` runtime with a narrow host mount set, persistent per-repo caches, internet access, and Codex’s own Linux sandbox enabled whenever possible.

The goal is:

```text
wcodex [same arguments as codex]
```

while avoiding host-wide read/write access.

## 1. Scope

Implement only one container engine:

```text
Apple container CLI: container
```

Do not implement Docker, Podman, Docker API bindings, or container socket integration.

The tool must:

1. Build the runtime OCI image using `container build`.
2. Run Codex using `container run`.
3. Mount only:
   - the current repo as `/workspace`;
   - a persistent isolated Codex home as `/root/.codex`;
   - a per-repo cache directory as `/cache`;
   - a tmpfs at `/tmp`.
4. Generate a Codex config that enables no-prompt operation without defaulting to `--yolo`.
5. Probe Codex’s inner Linux sandbox and use the best available mode:
   - bubblewrap-backed `workspace-write`;
   - `danger-full-access` only with explicit user opt-in.
6. Pass Codex arguments through so the user can use `wcodex` like `codex`.

Non-goals:

- Do not implement a custom sandbox in Rust.
- Do not mount the host home directory.
- Do not mount host `~/.codex`, `~/.ssh`, `~/.gitconfig`, Docker socket, cloud credentials, keychains, or package-manager credentials.
- Do not forward the SSH agent by default.
- Do not run image builds from the user repo as the build context.

## 2. Trust model

There are two containment layers:

```text
macOS host
└── Apple container VM
    ├── /workspace       writable repo bind mount
    ├── /cache           writable per-repo persistent cache
    ├── /root/.codex     Codex auth/config mount
    └── Codex inner sandbox
```

The outer `container` VM protects the host from ordinary tool execution. It does **not** protect anything mounted into the VM.

The Codex inner sandbox protects container paths, especially `/root/.codex`, from model-generated shell commands when `workspace-write` works.

The unavoidable exposed surfaces are:

```text
/workspace
/cache
/root/.codex as read by the Codex process itself
network
optional SSH agent, if explicitly enabled
```

Never describe the design as fully safe. The realistic claim is: it narrows the host blast radius.

## 3. Default behavior

Default invocation should be equivalent to:

```bash
codex --sandbox workspace-write --ask-for-approval never
```

inside the container, with additional config to allow `/cache`, `/tmp`, and network access.

Do not default to:

```bash
codex --yolo
```

`--ask-for-approval never` and sandboxing are separate controls. No prompts do not require disabling the sandbox.

## 4. CLI interface

Implement this interface with `clap`:

```text
wcodex [codex args...]

wcodex run [codex args...]
wcodex exec <prompt...>
wcodex login [codex login args...]
wcodex shell
wcodex doctor
wcodex resign [--squash]
wcodex build-image [--no-cache] [--pull]
wcodex clean-cache
wcodex clean-auth
wcodex clean-image
wcodex paths
```

Recommended flags:

```text
--image <tag>                    override generated image tag
--rebuild-image                  force image rebuild before run
--allow-yolo-fallback            allow container-only isolation if inner sandbox fails
--sandbox-mode auto|bwrap|yolo
--ssh                            forward SSH agent using container --ssh
--cpus <n>                       default: 4
--memory <size>                  default: 8g
--network <name>                 optional container network
--cache-cargo-target             set CARGO_TARGET_DIR=/cache/cargo-target
--repo-root <path>               override auto-detected Git root
```

Default command routing:

```text
wcodex                      -> interactive Codex TUI
wcodex exec "fix tests"     -> codex exec "fix tests"
wcodex login                -> codex login
wcodex shell                -> /bin/bash inside the runtime container
```

For trailing Codex args, preserve hyphenated arguments exactly. Never join user args into a shell string.

## 5. Rust architecture

Use Rust as an orchestrator, not as a sandbox.

Suggested module layout:

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
```

Suggested dependencies:

```toml
[dependencies]
anyhow = "1"
camino = "1"
clap = { version = "4", features = ["derive"] }
directories = "6"
fs-err = "3"
hex = "0.4"
sha2 = "0.10"
tempfile = "3"
thiserror = "2"
toml_edit = "0.22"
which = "8"
```

Use `std::process::Command` with structured args only.

For interactive runs on macOS, after setup/probes/config generation, replace the wrapper with `container run`:

```rust
#[cfg(unix)]
fn exec_interactive(mut cmd: std::process::Command) -> ! {
    use std::os::unix::process::CommandExt;

    let err = cmd.exec();
    eprintln!("failed to exec container runtime: {err}");
    std::process::exit(127);
}
```

Use `status()` instead of `exec()` for:

```text
build-image
doctor
sandbox probes
cleanup commands
non-interactive validation
```

## 6. State layout

Use one global state root:

```text
~/.wcodex/
```

Recommended structure:

```text
~/.wcodex/
  auth/
    codex-home/                 # mounted to /root/.codex
  repos/
    <repo-hash>/
      cache/                    # mounted to /cache
      gitconfig                 # mounted or copied into generated config/env
      last-run.json
  images/
    <image-hash>/
      Containerfile
      entrypoint.sh
      metadata.json
```

`<repo-hash>` should be a stable SHA-256 prefix of the canonical repo root path.

Use Git root when available:

```bash
git rev-parse --show-toplevel
```

Fallback to current working directory if not inside a Git repo.

Do not build the runtime image from the user repo. Copy the standalone
`runtime/Containerfile` into a temporary build context containing only:

```text
Containerfile
entrypoint.sh
```

Then run:

```bash
container build \
  --pull \
  --tag wcodex-runtime:latest \
  --file <tempdir>/Containerfile \
  <tempdir>
```

## 7. Runtime image Containerfile

The runtime image source lives in the standalone `runtime/Containerfile`. The
wrapper embeds that file and writes it into the temporary build context as
`Containerfile`; do not duplicate the Dockerfile body as a Rust string constant.

It includes:

- Codex CLI installed directly from the `openai/codex` GitHub release binary;
- system `bubblewrap`;
- Rust via rustup;
- `uv` and `uvx`;
- Python dev tools;
- common build/test/debug tools;
- cache env vars pointing at `/cache`.

```Dockerfile
# syntax=docker/dockerfile:1.7

FROM docker.io/library/debian:bookworm-slim

ARG DEBIAN_FRONTEND=noninteractive

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /usr/local/bin/

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
    bash \
    bubblewrap \
    build-essential \
    ca-certificates \
    ccache \
    clang \
    cmake \
    curl \
    dnsutils \
    fd-find \
    file \
    fzf \
    gdb \
    git \
    git-lfs \
    gnupg \
    iproute2 \
    ipset \
    iptables \
    iputils-ping \
    jq \
    less \
    lldb \
    lsof \
    make \
    nano \
    netcat-openbsd \
    ninja-build \
    openssh-client \
    pkg-config \
    procps \
    psmisc \
    python3 \
    python3-dev \
    python3-pip \
    python3-venv \
    ripgrep \
    rsync \
    strace \
    sudo \
    tree \
    unzip \
    xz-utils \
    zsh \
 && chmod 4755 /usr/bin/bwrap \
 && ln -sf /usr/bin/fdfind /usr/local/bin/fd \
 && git lfs install --system \
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/* /tmp/*

ARG RUST_TOOLCHAIN=stable
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --no-modify-path --profile default --default-toolchain "${RUST_TOOLCHAIN}" \
 && /root/.cargo/bin/rustup component add rustfmt clippy

ARG CODEX_VERSION=latest
RUN case "$(uname -m)" in \
      x86_64) codex_asset="codex-x86_64-unknown-linux-musl" ;; \
      aarch64|arm64) codex_asset="codex-aarch64-unknown-linux-musl" ;; \
      *) echo "unsupported Codex binary architecture: $(uname -m)" >&2; exit 1 ;; \
    esac \
 && if [[ "${CODEX_VERSION}" == "latest" ]]; then \
      codex_url="https://github.com/openai/codex/releases/latest/download/${codex_asset}.tar.gz"; \
    else \
      case "${CODEX_VERSION}" in rust-v*) codex_tag="${CODEX_VERSION}" ;; *) codex_tag="rust-v${CODEX_VERSION}" ;; esac; \
      codex_url="https://github.com/openai/codex/releases/download/${codex_tag}/${codex_asset}.tar.gz"; \
    fi \
 && curl --proto '=https' --tlsv1.2 -fsSL "${codex_url}" -o /tmp/codex.tar.gz \
 && tar -xzf /tmp/codex.tar.gz -C /tmp \
 && install -m 0755 "/tmp/${codex_asset}" /usr/local/bin/codex \
 && ln -sf /usr/local/bin/codex /usr/local/bin/codex-linux-sandbox \
 && rm -f /tmp/codex.tar.gz "/tmp/${codex_asset}" \
 && codex --version

COPY entrypoint.sh /usr/local/bin/wcodex-entrypoint
RUN chmod 0755 /usr/local/bin/wcodex-entrypoint

ENV HOME=/root
ENV CODEX_HOME=/root/.codex
ENV XDG_CACHE_HOME=/cache/xdg

# Python / uv cache and persistent tool dirs.
ENV UV_CACHE_DIR=/cache/uv/cache
ENV UV_TOOL_DIR=/cache/uv/tools
ENV UV_TOOL_BIN_DIR=/cache/uv/bin
ENV UV_PYTHON_INSTALL_DIR=/cache/uv/python
ENV UV_PYTHON_BIN_DIR=/cache/uv/python-bin
ENV UV_LINK_MODE=copy
ENV UV_NO_MODIFY_PATH=1
ENV PIP_CACHE_DIR=/cache/pip
ENV PIP_DISABLE_PIP_VERSION_CHECK=1
ENV PYTHONDONTWRITEBYTECODE=1

# Rust cache/install dirs.
# Rustup/toolchain is installed in the image under /root/.rustup and /root/.cargo/bin.
# Cargo registry/git cache and cargo-installed binaries go to /cache/cargo.
ENV RUSTUP_HOME=/root/.rustup
ENV CARGO_HOME=/cache/cargo
ENV CARGO_INSTALL_ROOT=/cache/cargo

# Other common caches.
ENV GOMODCACHE=/cache/go/pkg/mod
ENV GOCACHE=/cache/go/build
ENV CCACHE_DIR=/cache/ccache

ENV PATH=/cache/cargo/bin:/cache/uv/bin:/cache/uv/python-bin:/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

WORKDIR /workspace

ENTRYPOINT ["/usr/local/bin/wcodex-entrypoint"]
CMD []
```

`entrypoint.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

export PATH=/cache/cargo/bin:/cache/uv/bin:/cache/uv/python-bin:/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

mkdir -p \
  /root/.codex \
  /cache/xdg \
  /cache/uv/cache \
  /cache/uv/tools \
  /cache/uv/bin \
  /cache/uv/python \
  /cache/uv/python-bin \
  /cache/pip \
  /cache/cargo \
  /cache/go/pkg/mod \
  /cache/go/build \
  /cache/ccache \
  /tmp

case "${1:-}" in
  "")
    exec /usr/local/bin/codex
    ;;
  bash|sh|zsh|/bin/bash|/bin/sh|/bin/zsh)
    exec "$@"
    ;;
  codex)
    shift
    exec /usr/local/bin/codex "$@"
    ;;
  *)
    exec /usr/local/bin/codex "$@"
    ;;
esac
```

### Runtime image notes

Run as root inside the container by default. Apple `container` bind mounts can appear as `root root` inside the guest, and root avoids repo write failures caused by UID mapping. This is acceptable only because the container VM is the outer boundary and Codex’s inner sandbox is the preferred command boundary.

Keep `/usr/bin/bwrap` setuid with `chmod 4755`. Codex prefers a system `bwrap` on `PATH` and uses bubblewrap as the default Linux filesystem sandbox. The setuid bit improves compatibility in containerized environments where unprivileged user namespaces are restricted.

Do not populate `codex-resources/bwrap` with the distro `bwrap`; Codex treats that path as its own bundled helper and verifies it against an embedded digest. The direct release tarball contains only the Codex binary, so the runtime image should use system `bwrap` and provide `/usr/local/bin/codex-linux-sandbox` as a symlink to `/usr/local/bin/codex` for compatibility. The entrypoint should exec `/usr/local/bin/codex` by absolute path so Codex can pass an absolute current executable path to bubblewrap for the inner re-exec.

## 8. `container build` implementation

Build command shape:

```bash
container build \
  --pull \
  --cpus 4 \
  --memory 8g \
  --build-arg CODEX_VERSION=latest \
  --build-arg RUST_TOOLCHAIN=stable \
  --tag wcodex-runtime:latest \
  --file <context>/Containerfile \
  <context>
```

Use `wcodex-runtime:latest` as the default generated image tag. Keep an
implementation fingerprint in `~/.wcodex/images/<image-hash>/metadata.json` so
the wrapper can rebuild when `runtime/Containerfile` or the entrypoint changes
without making the user-facing image tag hash-based.

The `image-hash` fingerprint should include:

```text
runtime/Containerfile contents
entrypoint.sh contents
CODEX_VERSION
RUST_TOOLCHAIN
```

This makes rebuilds deterministic.

## 9. `container run` implementation

Base run flags:

```bash
container run \
  --rm \
  --interactive \
  --tty \
  --init \
  --name wcodex-<repo-hash>-<pid> \
  --cpus 4 \
  --memory 8g \
  --cap-add SYS_ADMIN \
  --cap-add SYS_CHROOT \
  --cap-add SETUID \
  --cap-add SETGID \
  --cap-add SYS_PTRACE \
  --mount type=bind,source=<repo-root>,target=/workspace \
  --mount type=bind,source=<codex-home>,target=/root/.codex \
  --mount type=bind,source=<repo-cache>,target=/cache \
  --tmpfs /tmp \
  --workdir /workspace \
  --env HOME=/root \
  --env CODEX_HOME=/root/.codex \
  --env XDG_CACHE_HOME=/cache/xdg \
  --env UV_CACHE_DIR=/cache/uv/cache \
  --env UV_TOOL_DIR=/cache/uv/tools \
  --env UV_TOOL_BIN_DIR=/cache/uv/bin \
  --env UV_PYTHON_INSTALL_DIR=/cache/uv/python \
  --env UV_PYTHON_BIN_DIR=/cache/uv/python-bin \
  --env UV_LINK_MODE=copy \
  --env UV_NO_MODIFY_PATH=1 \
  --env PIP_CACHE_DIR=/cache/pip \
  --env CARGO_HOME=/cache/cargo \
  --env CARGO_INSTALL_ROOT=/cache/cargo \
  --env RUSTUP_HOME=/root/.rustup \
  --env GOMODCACHE=/cache/go/pkg/mod \
  --env GOCACHE=/cache/go/build \
  --env CCACHE_DIR=/cache/ccache \
  --env GIT_CONFIG_GLOBAL=/cache/gitconfig \
  --env GIT_TERMINAL_PROMPT=0 \
  --env PATH=/cache/cargo/bin:/cache/uv/bin:/cache/uv/python-bin:/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  wcodex-runtime:latest \
  <codex args...>
```

If `--ssh` is passed by the user, add:

```bash
--ssh
```

Never add `--ssh` by default.

If `--cache-cargo-target` is passed, add:

```bash
--env CARGO_TARGET_DIR=/cache/cargo-target
```

Do not enable this by default because some scripts expect Cargo artifacts in `./target`. The per-repo `/cache` mount means this is safe to opt into without cross-repo target-dir contamination.

## 10. Codex config generation

Generate `/root/.codex/config.toml` in the host-mounted Codex home before running Codex.

Use this as the baseline:

```toml
cli_auth_credentials_store = "file"
allow_login_shell = false
approval_policy = "never"
sandbox_mode = "workspace-write"
default_permissions = "wcodex_container"
web_search = "cached"

[sandbox_workspace_write]
network_access = true
writable_roots = [
  "/workspace",
  "/cache",
  "/tmp"
]

[permissions.wcodex_container.filesystem]
":minimal" = "read"
":project_roots" = {
  "." = "write"
}
"/workspace" = "write"
"/cache" = "write"
"/tmp" = "write"
"/root/.local" = "write"
"/root/.codex" = "none"
glob_scan_max_depth = 4

[features.network_proxy]
enabled = true

[permissions.wcodex_container.network]
enabled = true
allow_local_binding = true

[permissions.wcodex_container.network.domains]
# OpenAI / Codex API and sign-in.
"**.openai.com" = "allow"
"**.chatgpt.com" = "allow"

# Git hosting, source archives, raw files, and release assets.
"**.github.com" = "allow"
"**.githubusercontent.com" = "allow"
"**.gitlab.com" = "allow"
"**.bitbucket.org" = "allow"

# Python / uv / pip.
"**.pypi.org" = "allow"
"**.pythonhosted.org" = "allow"
"**.python.org" = "allow"
"**.astral.sh" = "allow"

# Rust / Cargo / rustup.
"**.crates.io" = "allow"
"**.rust-lang.org" = "allow"
"**.rustup.rs" = "allow"
"rust-lang.github.io" = "allow"

# Go.
"**.golang.org" = "allow"
"**.go.dev" = "allow"

# Debian package mirrors for occasional apt usage inside the ephemeral runtime.
"**.debian.org" = "allow"

# Common JVM / Ruby / PHP package registries that appear in mixed-language repos.
"repo.maven.apache.org" = "allow"
"plugins.gradle.org" = "allow"
"services.gradle.org" = "allow"
"**.rubygems.org" = "allow"
"packagist.org" = "allow"
"repo.packagist.org" = "allow"

# OCI registries for tools that fetch container artifacts without host Docker access.
"ghcr.io" = "allow"
"quay.io" = "allow"
"registry-1.docker.io" = "allow"
"auth.docker.io" = "allow"
"production.cloudflare.docker.com" = "allow"

[shell_environment_policy]
inherit = "all"
include_only = [
  "PATH",
  "HOME",
  "TERM",
  "COLORTERM",
  "LANG",
  "LC_ALL",
  "CODEX_HOME",
  "XDG_CACHE_HOME",

  "UV_CACHE_DIR",
  "UV_TOOL_DIR",
  "UV_TOOL_BIN_DIR",
  "UV_PYTHON_INSTALL_DIR",
  "UV_PYTHON_BIN_DIR",
  "UV_LINK_MODE",
  "UV_NO_MODIFY_PATH",
  "UV_NO_ENV_FILE",

  "PIP_CACHE_DIR",
  "PIP_DISABLE_PIP_VERSION_CHECK",
  "PYTHONDONTWRITEBYTECODE",

  "RUSTUP_HOME",
  "CARGO_HOME",
  "CARGO_INSTALL_ROOT",
  "CARGO_TARGET_DIR",

  "GOMODCACHE",
  "GOCACHE",
  "CCACHE_DIR",

  "GIT_CONFIG_GLOBAL",
  "GIT_TERMINAL_PROMPT"
]
```


### Network proxy domain policy

The generated config should enable `features.network_proxy` and use the named permission profile's `network.domains` table as the default allowlist. Do not use the global `"*" = "allow"` rule by default. The allowlist is intentionally focused on endpoints needed for normal Codex operation and common dependency managers:

- OpenAI / ChatGPT auth and API traffic;
- GitHub, GitLab, and Bitbucket source dependencies;
- Python / uv / pip package resolution and uv-managed Python downloads;
- Rust / Cargo / rustup;
- Go modules and checksum database;
- Debian package mirrors for occasional ephemeral package installs;
- common JVM, Ruby, PHP, and OCI registry endpoints used by mixed-language repositories.

If a project needs another public domain, prefer an explicit future flag such as:

```text
--network-domain-allow example.org
```

If a project truly needs unrestricted outbound command networking, make that a separate explicit mode, for example:

```text
--network-policy full
```

That mode should add `"*" = "allow"` only for the selected run or generated profile. Do not make unrestricted outbound access the default.

Do not include `OPENAI_API_KEY`, `GITHUB_TOKEN`, `GH_TOKEN`, `AWS_*`, `GOOGLE_*`, `AZURE_*`, `NPM_TOKEN`, `PYPI_TOKEN`, `UV_PUBLISH_TOKEN`, `UV_INDEX_*_PASSWORD`, or `SSH_AUTH_SOCK`.

The mounted project itself is intentionally fully writable. Project-owned dotfiles
and nested credentials are part of `/workspace`; keep secrets out of the repo if
they should not be exposed to sandboxed commands.

If a project needs private dependencies, implement explicit opt-in flags later, for example:

```text
--env-allow GH_TOKEN
--ssh
--mount-secret name=...
```

Do not silently forward secrets.

## 11. Cache-writing check

The intended permissions do allow cache writing for `cargo`, `uv`, and common dev tools.

Required write paths:

| Tool | Runtime env | Write target | Permission source |
|---|---|---:|---|
| uv dependency cache | `UV_CACHE_DIR=/cache/uv/cache` | `/cache/uv/cache` | `/cache = write` |
| uv installed tools | `UV_TOOL_DIR=/cache/uv/tools` | `/cache/uv/tools` | `/cache = write` |
| uv tool executables | `UV_TOOL_BIN_DIR=/cache/uv/bin` | `/cache/uv/bin` | `/cache = write` |
| uv managed Pythons | `UV_PYTHON_INSTALL_DIR=/cache/uv/python` | `/cache/uv/python` | `/cache = write` |
| uv managed Python links | `UV_PYTHON_BIN_DIR=/cache/uv/python-bin` | `/cache/uv/python-bin` | `/cache = write` |
| uv project venvs | default `.venv` in repo | `/workspace/.venv` | project root write |
| Cargo registry/git cache | `CARGO_HOME=/cache/cargo` | `/cache/cargo` | `/cache = write` |
| Cargo-installed binaries | `CARGO_INSTALL_ROOT=/cache/cargo` | `/cache/cargo/bin` | `/cache = write` |
| Cargo build artifacts | default repo `target/` | `/workspace/target` | project root write |
| Cargo build artifacts, optional | `CARGO_TARGET_DIR=/cache/cargo-target` | `/cache/cargo-target` | `/cache = write` |
| pip | `PIP_CACHE_DIR=/cache/pip` | `/cache/pip` | `/cache = write` |
| Go modules | `GOMODCACHE=/cache/go/pkg/mod` | `/cache/go/pkg/mod` | `/cache = write` |
| Go build | `GOCACHE=/cache/go/build` | `/cache/go/build` | `/cache = write` |
| ccache | `CCACHE_DIR=/cache/ccache` | `/cache/ccache` | `/cache = write` |
| temp files | `TMPDIR` default or `/tmp` | `/tmp` | `/tmp = write` |

Important uv detail: by default, uv creates project virtual environments as `.venv` in the project root. That is allowed by `:project_roots = { "." = "write" }`. Because `/cache` and `/workspace` may be different mounts, set `UV_LINK_MODE=copy` to avoid hardlink/cross-device problems. This is slightly slower than same-filesystem hardlinking but more reliable.

Important Cargo detail: `CARGO_HOME` controls Cargo’s registry index and git checkout cache. `CARGO_TARGET_DIR` controls generated build artifacts. Leave `CARGO_TARGET_DIR` unset by default for compatibility; make it opt-in.

## 12. Sandbox probing

Before starting the interactive Codex session, probe the inner sandbox.

Probe 1: normal Linux sandbox:

```bash
container run <base-run-flags> wcodex-runtime:<hash> \
  sandbox linux -- /bin/sh -c 'echo bwrap-ok'
```

Selection logic:

```text
if bwrap probe succeeds:
    use --sandbox workspace-write --ask-for-approval never
else if --allow-yolo-fallback:
    use --sandbox danger-full-access --ask-for-approval never
else:
    fail closed with a clear message
```

Final Codex args prefix for normal mode:

```bash
--sandbox workspace-write \
--ask-for-approval never \
-c sandbox_workspace_write.network_access=true \
-c default_permissions='"wcodex_container"'
```

Final Codex args prefix for explicit yolo fallback:

```bash
--sandbox danger-full-access \
--ask-for-approval never
```

Never choose yolo automatically unless the user passed `--allow-yolo-fallback`.

## 13. `container` capability flags

For inner bubblewrap compatibility, include these by default:

```bash
--cap-add SYS_ADMIN
--cap-add SYS_CHROOT
--cap-add SETUID
--cap-add SETGID
--cap-add SYS_PTRACE
```

Reasoning:

- Codex’s Linux sandbox uses bubblewrap plus seccomp by default.
- Containerized Linux environments may block namespace, setuid bubblewrap, or seccomp operations.
- Apple `container` supports Linux capability customization with `--cap-add`.
- The wrapper probes actual behavior instead of assuming these flags are sufficient.

If a future version of Apple `container` or Codex works without these capabilities, the tool can add:

```text
--no-inner-sandbox-caps
```

as an advanced flag. Do not implement that in the first version unless needed.

## 14. Git config inside container

Do not mount host `~/.gitconfig`.

Generate `/cache/gitconfig`:

```ini
[safe]
    directory = /workspace

[user]
    name = <host git user.name or Wrapped Codex>
    email = <host git user.email or wrapped-codex@example.invalid>

[commit]
    gpgsign = false
```

Pass:

```bash
--env GIT_CONFIG_GLOBAL=/cache/gitconfig
--env GIT_TERMINAL_PROMPT=0
```

Private Git dependencies should fail by default unless the user explicitly opts into SSH forwarding or a secret mechanism.

## 14.1 Resigning branch commits on the host

`wcodex resign` is a host-side Git command, not a container command. It should:

- require a clean worktree;
- require the current branch to have an upstream;
- find the branch base with `git merge-base HEAD @{upstream}`;
- create a backup ref under `refs/wcodex/resign/`;
- recreate each linear commit ahead of upstream with `git commit -S -C <old-commit>`;
- fail clearly on merge commits unless `--squash` is used.

`wcodex resign --squash` should soft-reset to the branch base and create one
signed commit from the current branch diff with `git commit -S`.

## 15. Host environment policy

When spawning `container`, clear the environment and add a small allowlist:

```text
PATH
HOME
TERM
COLORTERM
LANG
LC_ALL
```

Do not pass through the host environment wholesale.

Explicitly avoid forwarding:

```text
OPENAI_API_KEY
GITHUB_TOKEN
GH_TOKEN
AWS_*
GOOGLE_*
AZURE_*
NPM_TOKEN
PYPI_TOKEN
UV_PUBLISH_TOKEN
UV_INDEX_*_PASSWORD
SSH_AUTH_SOCK
```

Codex auth should come from the isolated mounted Codex home:

```text
~/.wcodex/auth/codex-home -> /root/.codex
```

## 16. Login flow

`wcodex login` should run the same container/mount setup but pass login args to Codex:

```bash
container run <base-run-flags> wcodex-runtime:<hash> login <args...>
```

This writes auth into:

```text
~/.wcodex/auth/codex-home
```

not host `~/.codex`.

The generated Codex config sets:

```toml
cli_auth_credentials_store = "file"
```

Treat this directory as sensitive.

## 17. Doctor command

Implement `wcodex doctor` to print:

```text
container binary path
container version
state root path
detected repo root
repo hash
image tag
whether image exists or needs build
mount paths
Codex config path
sandbox probe result: bwrap | failed
cache write test result
repo write test result
network test result
```

Doctor should run these checks inside the container:

```bash
pwd
id
test -w /workspace
test -w /cache
test -w /tmp
uv cache dir
cargo --version
rustc --version
python3 --version
codex --version
codex sandbox linux -- /bin/sh -c 'echo sandbox-ok'
```

Also test that generated shell commands cannot read Codex auth when sandboxed:

```bash
codex exec --sandbox workspace-write --ask-for-approval never \
  "try to read /root/.codex/auth.json and report whether it is blocked"
```

This last test may require a model call; make it optional behind:

```text
wcodex doctor --codex-auth-probe
```

## 18. Failure modes and expected behavior

### Inner sandbox fails

Fail closed by default:

```text
Codex inner sandbox failed inside the container.
Tried:
  1. bubblewrap-backed Linux sandbox

Refusing danger-full-access without --allow-yolo-fallback.

Run:
  wcodex --allow-yolo-fallback ...
```

### Repo is dirty

Do not block by default, but print a warning if `git status --porcelain` is non-empty.

Optional future flag:

```text
--require-clean-git
```

### Image missing

Build automatically on first run unless `--no-build` is implemented. For the first version, automatic build is fine.

### Build fails

Print the exact generated build context path and the `container build` argv in debug mode.

### Cache permission failure

Run a clear cache write probe:

```bash
touch /cache/.wcodex-write-test && rm /cache/.wcodex-write-test
```

If it fails, explain that `/cache` is required for `uv`, `cargo`, pip, Go, and ccache.

### Workspace permission failure

Run:

```bash
touch /workspace/.wcodex-write-test && rm /workspace/.wcodex-write-test
```

If it fails, explain that Apple `container` bind mount ownership may require running as root inside the container. The default runtime image already does this.

## 19. Implementation checklist for Codex

Implement in this order:

1. Create Rust crate.
2. Add `clap` CLI with pass-through args.
3. Implement state path detection and repo hash.
4. Generate the runtime image build context.
5. Implement `container build`.
6. Generate `/root/.codex/config.toml` in host state.
7. Generate `/cache/gitconfig`.
8. Implement base `container run` argv builder.
9. Implement `wcodex shell`.
10. Implement `wcodex login`.
11. Implement sandbox probes.
12. Implement default `wcodex` passthrough with selected sandbox mode.
13. Implement `wcodex exec`.
14. Implement `wcodex doctor`.
15. Implement cleanup commands.
16. Add unit tests for argv construction.
17. Add integration tests gated behind `WCODEX_INTEGRATION=1`.

## 20. Unit test expectations

Test that:

- No shell interpolation is used for container commands.
- Host home is never mounted.
- Host `~/.codex` is never mounted.
- Host `~/.ssh` is never mounted.
- SSH forwarding is absent unless `--ssh`.
- `/workspace`, `/cache`, and `/root/.codex` mounts are present.
- `/cache` is in Codex writable roots.
- `/root/.codex` is denied in the permission profile.
- `features.network_proxy.enabled` is true in the generated config.
- The named permission profile contains explicit network domain allow rules and does not use `"*" = "allow"` by default.
- `UV_CACHE_DIR`, `UV_TOOL_DIR`, `UV_PYTHON_INSTALL_DIR`, `CARGO_HOME`, and optional `CARGO_TARGET_DIR` all point under `/cache`.
- `--allow-yolo-fallback` is required before `danger-full-access` can be selected automatically.
- The final Codex args preserve user-provided args exactly.

## 21. Security rules to preserve

Do not weaken these during implementation:

```text
No host home mount.
No host ~/.codex mount.
No Docker socket.
No automatic SSH agent forwarding.
No automatic host env forwarding.
No automatic yolo fallback.
No user repo as image build context.
No shell string command construction for host-side container invocation.
```

## 22. References checked

- OpenAI Codex agent approvals and security:
  - https://developers.openai.com/codex/agent-approvals-security
- OpenAI Codex configuration reference:
  - https://developers.openai.com/codex/config-reference
- OpenAI Codex sample configuration:
  - https://developers.openai.com/codex/config-sample
- OpenAI Codex Linux sandbox README:
  - https://github.com/openai/codex/blob/main/codex-rs/linux-sandbox/README.md
- OpenAI Codex CLI install:
  - https://github.com/openai/codex
- Apple `container` README:
  - https://github.com/apple/container
- Apple `container` command reference:
  - https://github.com/apple/container/blob/main/docs/command-reference.md
- Apple `container` how-to:
  - https://github.com/apple/container/blob/main/docs/how-to.md
- uv cache docs:
  - https://docs.astral.sh/uv/concepts/cache/
- uv Docker docs:
  - https://docs.astral.sh/uv/guides/integration/docker/
- uv environment variables:
  - https://docs.astral.sh/uv/reference/environment/
- uv storage docs:
  - https://docs.astral.sh/uv/reference/storage/
- Cargo environment variables:
  - https://doc.rust-lang.org/cargo/reference/environment-variables.html
- rustup environment variables:
  - https://rust-lang.github.io/rustup/environment-variables.html
- rustup installation:
  - https://rust-lang.github.io/rustup/installation/index.html
