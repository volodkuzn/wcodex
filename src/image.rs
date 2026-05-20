use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::paths;

pub const CODEX_VERSION: &str = "latest";
pub const RUST_TOOLCHAIN: &str = "stable";
pub const IMAGE_REPOSITORY: &str = "wcodex-runtime";

pub fn containerfile_contents() -> &'static str {
    include_str!("../runtime/Containerfile")
}

pub const ENTRYPOINT_SH: &str = r#"#!/usr/bin/env bash
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
    hasher.update(containerfile_contents().as_bytes());
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

    fs_err::write(&containerfile, containerfile_contents())
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
    fs_err::write(image_dir.join("Containerfile"), containerfile_contents())
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
    fn runtime_containerfile_contains_required_tools_and_cache_env() {
        let containerfile = containerfile_contents();

        assert!(containerfile.contains("FROM docker.io/library/debian:bookworm-slim"));
        assert!(containerfile.contains("bubblewrap"));
        assert!(containerfile
            .contains("ln -sf /usr/local/bin/codex /usr/local/bin/codex-linux-sandbox"));
        assert!(!containerfile.contains("codex-resources/bwrap"));
        assert!(containerfile.contains("COPY entrypoint.sh /usr/local/bin/wcodex-entrypoint"));
        assert!(containerfile.contains("ENTRYPOINT [\"/usr/local/bin/wcodex-entrypoint\"]"));
        assert!(containerfile.contains(&format!(
            "ENV PATH={}",
            crate::engine_container::RUNTIME_PATH
        )));
        assert!(ENTRYPOINT_SH.contains(&format!(
            "export PATH={}",
            crate::engine_container::RUNTIME_PATH
        )));
        assert!(ENTRYPOINT_SH.contains("exec /usr/local/bin/codex \"$@\""));
        assert!(containerfile.contains("ARG RUST_TOOLCHAIN=stable\nRUN curl"));
        assert!(containerfile.contains("ARG CODEX_VERSION=latest\nRUN case"));
        assert!(containerfile.contains(
            "https://github.com/openai/codex/releases/latest/download/${codex_asset}.tar.gz"
        ));
        assert!(containerfile.contains(
            "https://github.com/openai/codex/releases/download/${codex_tag}/${codex_asset}.tar.gz"
        ));
        assert!(containerfile.contains("codex_tag=\"rust-v${CODEX_VERSION}\""));
        assert!(
            containerfile.contains("install -m 0755 \"/tmp/${codex_asset}\" /usr/local/bin/codex")
        );
        assert!(!containerfile.contains("node:"));
        assert!(!containerfile.contains("nodejs"));
        assert!(!containerfile.contains("npm install -g"));
        assert!(!containerfile.contains("@openai/codex"));
        assert!(!containerfile.contains("npm_config_cache"));
        assert!(!containerfile.contains("/cache/npm"));
        assert!(containerfile.contains("ENV UV_CACHE_DIR=/cache/uv/cache"));
        assert!(containerfile.contains("ENV CARGO_HOME=/cache/cargo"));
        assert!(containerfile.contains("chmod 4755 /usr/bin/bwrap"));
    }

    #[test]
    fn build_context_uses_standalone_containerfile() {
        let context = create_build_context().unwrap();
        let written = fs_err::read_to_string(&context.containerfile).unwrap();

        assert_eq!(written, containerfile_contents());
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
