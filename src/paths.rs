use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};

pub const HASH_PREFIX_LEN: usize = 16;

pub fn canonicalize_utf8(path: impl AsRef<Path>) -> Result<Utf8PathBuf> {
    let canonical = fs_err::canonicalize(path.as_ref())
        .with_context(|| format!("failed to canonicalize {}", path.as_ref().display()))?;
    Utf8PathBuf::from_path_buf(canonical).map_err(|path| {
        anyhow!(
            "path is not valid UTF-8, which wcodex currently requires: {}",
            path.display()
        )
    })
}

pub fn current_dir_utf8() -> Result<Utf8PathBuf> {
    let cwd = std::env::current_dir().context("failed to read current directory")?;
    Utf8PathBuf::from_path_buf(cwd).map_err(|path| {
        anyhow!(
            "current directory is not valid UTF-8, which wcodex currently requires: {}",
            path.display()
        )
    })
}

pub fn detect_repo_root(override_path: Option<&PathBuf>) -> Result<Utf8PathBuf> {
    if let Some(path) = override_path {
        return canonicalize_utf8(path);
    }

    let cwd = std::env::current_dir().context("failed to read current directory")?;
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&cwd)
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !root.is_empty() {
                return canonicalize_utf8(root);
            }
        }
    }

    canonicalize_utf8(cwd)
}

pub fn stable_path_hash(path: &Utf8Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.as_str().as_bytes());
    let digest = hasher.finalize();
    hex::encode(digest)[..HASH_PREFIX_LEN].to_owned()
}

pub fn is_git_dirty(repo_root: &Utf8Path) -> bool {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .output();

    output
        .map(|output| output.status.success() && !output.stdout.is_empty())
        .unwrap_or(false)
}

pub fn path_to_os(path: &Utf8Path) -> std::ffi::OsString {
    path.as_std_path().as_os_str().to_os_string()
}

pub fn pathbuf_to_utf8(path: PathBuf) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path).map_err(|path| {
        anyhow!(
            "path is not valid UTF-8, which wcodex currently requires: {}",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_hash_is_stable_sha256_prefix() {
        let path = Utf8Path::new("/tmp/example/repo");
        assert_eq!(stable_path_hash(path), stable_path_hash(path));
        assert_eq!(stable_path_hash(path).len(), HASH_PREFIX_LEN);
    }
}
