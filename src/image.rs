use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::paths;

pub const CODEX_VERSION: &str = "latest";
pub const RUST_TOOLCHAIN: &str = "stable";
pub const IMAGE_REPOSITORY: &str = "wcodex-runtime";

pub const CONTAINERFILE: &str = r#"# syntax=docker/dockerfile:1.7

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
    tini \
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
 && rm -f /tmp/codex.tar.gz "/tmp/${codex_asset}" \
 && codex --version

COPY entrypoint.sh /usr/local/bin/wcodex-entrypoint
RUN chmod 0755 /usr/local/bin/wcodex-entrypoint

ENV HOME=/root
ENV CODEX_HOME=/root/.codex
ENV XDG_CACHE_HOME=/cache/xdg
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
ENV RUSTUP_HOME=/root/.rustup
ENV CARGO_HOME=/cache/cargo
ENV CARGO_INSTALL_ROOT=/cache/cargo
ENV GOMODCACHE=/cache/go/pkg/mod
ENV GOCACHE=/cache/go/build
ENV CCACHE_DIR=/cache/ccache
ENV PATH=/cache/cargo/bin:/cache/uv/bin:/cache/uv/python-bin:/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

WORKDIR /workspace

ENTRYPOINT ["tini", "--", "/usr/local/bin/wcodex-entrypoint"]
CMD []
"#;

pub const ENTRYPOINT_SH: &str = r#"#!/usr/bin/env bash
set -euo pipefail

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
    exec codex
    ;;
  bash|sh|zsh|/bin/bash|/bin/sh|/bin/zsh)
    exec "$@"
    ;;
  codex)
    shift
    exec codex "$@"
    ;;
  *)
    exec codex "$@"
    ;;
esac
"#;

#[derive(Debug)]
pub struct ImageBuildContext {
    _tempdir: TempDir,
    pub root: Utf8PathBuf,
    pub containerfile: Utf8PathBuf,
    pub entrypoint: Utf8PathBuf,
}

#[derive(Debug, Clone)]
pub struct ImageBuildOptions {
    pub tag: String,
    pub cpus: String,
    pub memory: String,
    pub no_cache: bool,
    pub pull: bool,
}

pub fn runtime_image_hash() -> String {
    let mut hasher = Sha256::new();
    hasher.update(CONTAINERFILE.as_bytes());
    hasher.update([0]);
    hasher.update(ENTRYPOINT_SH.as_bytes());
    hasher.update([0]);
    hasher.update(CODEX_VERSION.as_bytes());
    hasher.update([0]);
    hasher.update(RUST_TOOLCHAIN.as_bytes());
    let digest = hasher.finalize();
    hex::encode(digest)[..20].to_owned()
}

pub fn runtime_image_tag() -> String {
    format!("{IMAGE_REPOSITORY}:latest")
}

pub fn create_build_context() -> Result<ImageBuildContext> {
    let tempdir = tempfile::Builder::new()
        .prefix("wcodex-image-")
        .tempdir()
        .context("failed to create image build context")?;
    let root = paths::pathbuf_to_utf8(tempdir.path().to_path_buf())?;
    let containerfile = root.join("Containerfile");
    let entrypoint = root.join("entrypoint.sh");

    fs_err::write(&containerfile, CONTAINERFILE)
        .with_context(|| format!("failed to write {containerfile}"))?;
    fs_err::write(&entrypoint, ENTRYPOINT_SH)
        .with_context(|| format!("failed to write {entrypoint}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs_err::set_permissions(&entrypoint, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("failed to chmod {entrypoint}"))?;
    }

    Ok(ImageBuildContext {
        _tempdir: tempdir,
        root,
        containerfile,
        entrypoint,
    })
}

pub fn build_args(
    context: &ImageBuildContext,
    options: &ImageBuildOptions,
) -> Vec<std::ffi::OsString> {
    let mut args = vec!["build".into()];

    if options.pull {
        args.push("--pull".into());
    }
    args.extend(["--cpus".into(), options.cpus.clone().into()]);
    args.extend(["--memory".into(), options.memory.clone().into()]);
    if options.no_cache {
        args.push("--no-cache".into());
    }
    args.extend([
        "--build-arg".into(),
        format!("CODEX_VERSION={CODEX_VERSION}").into(),
        "--build-arg".into(),
        format!("RUST_TOOLCHAIN={RUST_TOOLCHAIN}").into(),
        "--tag".into(),
        options.tag.clone().into(),
        "--file".into(),
        context
            .containerfile
            .as_std_path()
            .as_os_str()
            .to_os_string(),
        context.root.as_std_path().as_os_str().to_os_string(),
    ]);

    args
}

pub fn write_image_state(image_dir: &Utf8Path, tag: &str) -> Result<()> {
    fs_err::create_dir_all(image_dir).with_context(|| format!("failed to create {image_dir}"))?;
    fs_err::write(image_dir.join("Containerfile"), CONTAINERFILE)
        .with_context(|| format!("failed to write {}", image_dir.join("Containerfile")))?;
    fs_err::write(image_dir.join("entrypoint.sh"), ENTRYPOINT_SH)
        .with_context(|| format!("failed to write {}", image_dir.join("entrypoint.sh")))?;
    fs_err::write(image_dir.join("metadata.json"), metadata_json(tag))
        .with_context(|| format!("failed to write {}", image_dir.join("metadata.json")))?;
    Ok(())
}

pub fn image_state_exists(image_dir: &Utf8Path) -> bool {
    image_dir.join("metadata.json").is_file()
}

fn metadata_json(tag: &str) -> String {
    format!(
        "{{\n  \"tag\": \"{}\",\n  \"codex_version\": \"{}\",\n  \"rust_toolchain\": \"{}\"\n}}\n",
        json_escape(tag),
        CODEX_VERSION,
        RUST_TOOLCHAIN
    )
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            _ => vec![ch],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_hash_is_deterministic_prefix() {
        assert_eq!(runtime_image_hash(), runtime_image_hash());
        assert_eq!(runtime_image_hash().len(), 20);
    }

    #[test]
    fn default_runtime_image_tag_is_latest_not_hash_based() {
        let tag = runtime_image_tag();
        assert_eq!(tag, "wcodex-runtime:latest");
        assert!(!tag.contains(&runtime_image_hash()));
    }

    #[test]
    fn generated_image_contains_required_tools_and_cache_env() {
        assert!(CONTAINERFILE.contains("FROM docker.io/library/debian:bookworm-slim"));
        assert!(CONTAINERFILE.contains("bubblewrap"));
        assert!(CONTAINERFILE.contains("ARG RUST_TOOLCHAIN=stable\nRUN curl"));
        assert!(CONTAINERFILE.contains("ARG CODEX_VERSION=latest\nRUN case"));
        assert!(CONTAINERFILE.contains(
            "https://github.com/openai/codex/releases/latest/download/${codex_asset}.tar.gz"
        ));
        assert!(CONTAINERFILE.contains(
            "https://github.com/openai/codex/releases/download/${codex_tag}/${codex_asset}.tar.gz"
        ));
        assert!(CONTAINERFILE.contains("codex_tag=\"rust-v${CODEX_VERSION}\""));
        assert!(
            CONTAINERFILE.contains("install -m 0755 \"/tmp/${codex_asset}\" /usr/local/bin/codex")
        );
        assert!(!CONTAINERFILE.contains("node:"));
        assert!(!CONTAINERFILE.contains("nodejs"));
        assert!(!CONTAINERFILE.contains("npm install -g"));
        assert!(!CONTAINERFILE.contains("@openai/codex"));
        assert!(!CONTAINERFILE.contains("npm_config_cache"));
        assert!(!CONTAINERFILE.contains("/cache/npm"));
        assert!(CONTAINERFILE.contains("ENV UV_CACHE_DIR=/cache/uv/cache"));
        assert!(CONTAINERFILE.contains("ENV CARGO_HOME=/cache/cargo"));
        assert!(CONTAINERFILE.contains("chmod 4755 /usr/bin/bwrap"));
    }

    #[test]
    fn build_args_include_pull_when_requested() {
        let context = create_build_context().unwrap();
        let args = build_args(
            &context,
            &ImageBuildOptions {
                tag: "wcodex-runtime:test".into(),
                cpus: "4".into(),
                memory: "8g".into(),
                no_cache: false,
                pull: true,
            },
        );
        let strings = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(strings.contains(&"--pull".into()));
        assert!(strings.contains(&"--cpus".into()));
        assert!(strings.contains(&"--memory".into()));
        assert!(strings.contains(&"--tag".into()));
    }

    #[test]
    fn build_args_omit_pull_when_not_requested() {
        let context = create_build_context().unwrap();
        let args = build_args(
            &context,
            &ImageBuildOptions {
                tag: "wcodex-runtime:test".into(),
                cpus: "4".into(),
                memory: "8g".into(),
                no_cache: false,
                pull: false,
            },
        );
        let strings = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(!strings.contains(&"--pull".into()));
    }
}
