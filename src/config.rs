use std::process::Command;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

pub const CODEX_CONFIG_FILE: &str = "config.toml";

pub fn codex_config_toml() -> String {
    r#"cli_auth_credentials_store = "file"
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
  "/tmp",
]

[permissions.wcodex_container.filesystem]
"/workspace" = "write"
"/cache" = "write"
"/tmp" = "write"
"/root/.local" = "write"
"/root/.codex" = "none"
glob_scan_max_depth = 4

[permissions.wcodex_container.filesystem.":project_roots"]
"." = "write"
"**/.env" = "none"
"**/*.env" = "none"
"**/.pypirc" = "none"
"**/.netrc" = "none"
"**/id_rsa" = "none"
"**/id_ed25519" = "none"
"**/*_rsa" = "none"
"**/*_ed25519" = "none"

[features.network_proxy]
enabled = true

[permissions.wcodex_container.network]
enabled = true
allow_local_binding = true

[permissions.wcodex_container.network.domains]
"**.openai.com" = "allow"
"**.chatgpt.com" = "allow"
"**.github.com" = "allow"
"**.githubusercontent.com" = "allow"
"**.gitlab.com" = "allow"
"**.bitbucket.org" = "allow"
"**.pypi.org" = "allow"
"**.pythonhosted.org" = "allow"
"**.python.org" = "allow"
"**.astral.sh" = "allow"
"**.crates.io" = "allow"
"**.rust-lang.org" = "allow"
"**.rustup.rs" = "allow"
"rust-lang.github.io" = "allow"
"**.golang.org" = "allow"
"**.go.dev" = "allow"
"**.debian.org" = "allow"
"repo.maven.apache.org" = "allow"
"plugins.gradle.org" = "allow"
"services.gradle.org" = "allow"
"**.rubygems.org" = "allow"
"packagist.org" = "allow"
"repo.packagist.org" = "allow"
"ghcr.io" = "allow"
"quay.io" = "allow"
"registry-1.docker.io" = "allow"
"auth.docker.io" = "allow"
"production.cloudflare.docker.com" = "allow"

[shell_environment_policy]
inherit = "none"
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
  "GIT_TERMINAL_PROMPT",
]
"#
    .to_owned()
}

pub fn write_codex_config(codex_home: &Utf8Path) -> Result<Utf8PathBuf> {
    fs_err::create_dir_all(codex_home)
        .with_context(|| format!("failed to create Codex home {codex_home}"))?;
    let config_path = codex_home.join(CODEX_CONFIG_FILE);
    fs_err::write(&config_path, codex_config_toml())
        .with_context(|| format!("failed to write {config_path}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs_err::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to set permissions on {config_path}"))?;
    }

    Ok(config_path)
}

pub fn write_gitconfig(path: &Utf8Path, repo_root: &Utf8Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs_err::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    }

    let name = read_git_config(repo_root, "user.name").unwrap_or_else(|| "Wrapped Codex".into());
    let email = read_git_config(repo_root, "user.email")
        .unwrap_or_else(|| "wrapped-codex@example.invalid".into());
    let contents = format!(
        "[safe]\n    directory = /workspace\n\n[user]\n    name = {}\n    email = {}\n",
        sanitize_gitconfig_value(&name),
        sanitize_gitconfig_value(&email)
    );
    fs_err::write(path, contents).with_context(|| format!("failed to write {path}"))?;
    Ok(())
}

fn read_git_config(repo_root: &Utf8Path, key: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["config", "--get", key])
        .current_dir(repo_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn sanitize_gitconfig_value(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '\r' | '\n' => ' ',
            _ => ch,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use toml_edit::DocumentMut;

    use super::*;

    #[test]
    fn generated_config_is_valid_toml() {
        codex_config_toml().parse::<DocumentMut>().unwrap();
    }

    #[test]
    fn cache_is_in_writable_roots() {
        let doc = codex_config_toml().parse::<DocumentMut>().unwrap();
        let roots = doc["sandbox_workspace_write"]["writable_roots"]
            .as_array()
            .unwrap();

        assert!(roots.iter().any(|item| item.as_str() == Some("/cache")));
    }

    #[test]
    fn codex_home_is_denied_in_permission_profile() {
        let doc = codex_config_toml().parse::<DocumentMut>().unwrap();
        assert_eq!(
            doc["permissions"]["wcodex_container"]["filesystem"]["/root/.codex"].as_str(),
            Some("none")
        );
    }

    #[test]
    fn network_proxy_is_enabled_without_global_star_allow() {
        let doc = codex_config_toml().parse::<DocumentMut>().unwrap();
        assert_eq!(
            doc["features"]["network_proxy"]["enabled"].as_bool(),
            Some(true)
        );
        let domains = doc["permissions"]["wcodex_container"]["network"]["domains"]
            .as_table()
            .unwrap();
        assert!(!domains.contains_key("*"));
        assert_eq!(
            domains.get("**.openai.com").unwrap().as_str(),
            Some("allow")
        );
        assert!(!domains.contains_key("**.npmjs.org"));
        assert!(!domains.contains_key("**.npmjs.com"));
        assert!(!domains.contains_key("**.nodejs.org"));
    }
}
