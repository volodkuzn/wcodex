use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use directories::BaseDirs;

use crate::paths;

#[derive(Debug, Clone)]
pub struct StatePaths {
    pub root: Utf8PathBuf,
    pub codex_home: Utf8PathBuf,
    pub repo_root: Utf8PathBuf,
    pub repo_hash: String,
    pub repo_dir: Utf8PathBuf,
    pub repo_cache: Utf8PathBuf,
    pub repo_gitconfig: Utf8PathBuf,
    pub cache_gitconfig: Utf8PathBuf,
    pub images_dir: Utf8PathBuf,
}

impl StatePaths {
    pub fn discover(repo_override: Option<&std::path::PathBuf>) -> Result<Self> {
        let base_dirs = BaseDirs::new().context("failed to find user home directory")?;
        let home = paths::pathbuf_to_utf8(base_dirs.home_dir().to_path_buf())?;
        let root = home.join(".wcodex");
        let codex_home = root.join("auth").join("codex-home");
        let repo_root = paths::detect_repo_root(repo_override)?;
        let repo_hash = paths::stable_path_hash(&repo_root);
        let repo_dir = root.join("repos").join(&repo_hash);
        let repo_cache = repo_dir.join("cache");
        let repo_gitconfig = repo_dir.join("gitconfig");
        let cache_gitconfig = repo_cache.join("gitconfig");
        let images_dir = root.join("images");

        Ok(Self {
            root,
            codex_home,
            repo_root,
            repo_hash,
            repo_dir,
            repo_cache,
            repo_gitconfig,
            cache_gitconfig,
            images_dir,
        })
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        create_private_dir(&self.root)?;
        create_private_dir(&self.codex_home)?;
        fs_err::create_dir_all(&self.repo_cache)
            .with_context(|| format!("failed to create {}", self.repo_cache))?;
        fs_err::create_dir_all(&self.images_dir)
            .with_context(|| format!("failed to create {}", self.images_dir))?;
        Ok(())
    }

    pub fn image_dir(&self, image_hash: &str) -> Utf8PathBuf {
        self.images_dir.join(image_hash)
    }
}

fn create_private_dir(path: &Utf8Path) -> Result<()> {
    fs_err::create_dir_all(path).with_context(|| format!("failed to create {path}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let permissions = std::fs::Permissions::from_mode(0o700);
        fs_err::set_permissions(path, permissions)
            .with_context(|| format!("failed to set permissions on {path}"))?;
    }

    Ok(())
}
