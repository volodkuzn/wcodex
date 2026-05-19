use std::ffi::{OsStr, OsString};
use std::process::{Command, ExitStatus, Output};

use anyhow::{Context, Result};
use camino::Utf8PathBuf;

use crate::paths;

const HOST_ENV_ALLOWLIST: &[&str] = &["PATH", "HOME", "TERM", "COLORTERM", "LANG", "LC_ALL"];

const RUNTIME_ENV: &[(&str, &str)] = &[
    ("HOME", "/root"),
    ("CODEX_HOME", "/root/.codex"),
    ("XDG_CACHE_HOME", "/cache/xdg"),
    ("UV_CACHE_DIR", "/cache/uv/cache"),
    ("UV_TOOL_DIR", "/cache/uv/tools"),
    ("UV_TOOL_BIN_DIR", "/cache/uv/bin"),
    ("UV_PYTHON_INSTALL_DIR", "/cache/uv/python"),
    ("UV_PYTHON_BIN_DIR", "/cache/uv/python-bin"),
    ("UV_LINK_MODE", "copy"),
    ("UV_NO_MODIFY_PATH", "1"),
    ("PIP_CACHE_DIR", "/cache/pip"),
    ("PIP_DISABLE_PIP_VERSION_CHECK", "1"),
    ("PYTHONDONTWRITEBYTECODE", "1"),
    ("CARGO_HOME", "/cache/cargo"),
    ("CARGO_INSTALL_ROOT", "/cache/cargo"),
    ("RUSTUP_HOME", "/root/.rustup"),
    ("GOMODCACHE", "/cache/go/pkg/mod"),
    ("GOCACHE", "/cache/go/build"),
    ("CCACHE_DIR", "/cache/ccache"),
    ("GIT_CONFIG_GLOBAL", "/cache/gitconfig"),
    ("GIT_TERMINAL_PROMPT", "0"),
    (
        "PATH",
        "/cache/cargo/bin:/cache/uv/bin:/cache/uv/python-bin:/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    ),
];

const INNER_SANDBOX_CAPS: &[&str] = &["SYS_ADMIN", "SYS_CHROOT", "SETUID", "SETGID", "SYS_PTRACE"];

#[derive(Debug, Clone)]
pub struct ContainerEngine {
    pub binary: std::path::PathBuf,
}

#[derive(Debug, Clone)]
pub struct RunContext {
    pub repo_root: Utf8PathBuf,
    pub codex_home: Utf8PathBuf,
    pub repo_cache: Utf8PathBuf,
    pub repo_hash: String,
    pub image_tag: String,
    pub cpus: String,
    pub memory: String,
    pub network: Option<String>,
    pub ssh: bool,
    pub cache_cargo_target: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdioMode {
    Interactive,
    Batch,
}

impl ContainerEngine {
    pub fn detect() -> Result<Self> {
        let binary =
            which::which("container").context("failed to find Apple `container` on PATH")?;
        Ok(Self { binary })
    }

    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.binary);
        apply_host_env_policy(&mut cmd);
        cmd
    }

    pub fn status<I, S>(&self, args: I) -> Result<ExitStatus>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = self.command();
        cmd.args(args);
        cmd.status()
            .with_context(|| format!("failed to run {}", self.binary.display()))
    }

    pub fn output<I, S>(&self, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = self.command();
        cmd.args(args);
        cmd.output()
            .with_context(|| format!("failed to run {}", self.binary.display()))
    }

    pub fn image_exists(&self, tag: &str) -> Result<bool> {
        let output = self.output(["image", "inspect", tag])?;
        Ok(output.status.success())
    }

    pub fn delete_image(&self, tag: &str) -> Result<bool> {
        let status = self.status(["image", "delete", tag])?;
        Ok(status.success())
    }
}

pub fn apply_host_env_policy(cmd: &mut Command) {
    cmd.env_clear();
    for key in HOST_ENV_ALLOWLIST {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
}

pub fn container_run_args(
    context: &RunContext,
    stdio_mode: StdioMode,
    name_suffix: &str,
    runtime_args: &[String],
) -> Vec<OsString> {
    let mut args = base_run_args(context, stdio_mode, name_suffix);
    args.push(context.image_tag.clone().into());
    args.extend(runtime_args.iter().cloned().map(OsString::from));
    args
}

pub fn base_run_args(
    context: &RunContext,
    stdio_mode: StdioMode,
    name_suffix: &str,
) -> Vec<OsString> {
    let mut args = vec![
        "run".into(),
        "--rm".into(),
        "--init".into(),
        "--name".into(),
        format!("wcodex-{}-{name_suffix}", context.repo_hash).into(),
        "--cpus".into(),
        context.cpus.clone().into(),
        "--memory".into(),
        context.memory.clone().into(),
    ];

    if stdio_mode == StdioMode::Interactive {
        args.push("--interactive".into());
        args.push("--tty".into());
    }

    for cap in INNER_SANDBOX_CAPS {
        args.push("--cap-add".into());
        args.push((*cap).into());
    }

    args.push("--mount".into());
    args.push(format!("type=bind,source={},target=/workspace", context.repo_root).into());
    args.push("--mount".into());
    args.push(
        format!(
            "type=bind,source={},target=/root/.codex",
            context.codex_home
        )
        .into(),
    );
    args.push("--mount".into());
    args.push(format!("type=bind,source={},target=/cache", context.repo_cache).into());
    args.extend([
        "--tmpfs".into(),
        "/tmp".into(),
        "--workdir".into(),
        "/workspace".into(),
    ]);

    for (key, value) in RUNTIME_ENV {
        args.push("--env".into());
        args.push(format!("{key}={value}").into());
    }

    if context.cache_cargo_target {
        args.push("--env".into());
        args.push("CARGO_TARGET_DIR=/cache/cargo-target".into());
    }

    if let Some(network) = &context.network {
        args.push("--network".into());
        args.push(network.clone().into());
    }

    if context.ssh {
        args.push("--ssh".into());
    }

    args
}

pub fn format_argv(binary: &std::path::Path, args: &[OsString]) -> String {
    std::iter::once(binary.as_os_str().to_string_lossy().to_string())
        .chain(
            args.iter()
                .map(|arg| shellish_quote(&arg.to_string_lossy())),
        )
        .collect::<Vec<_>>()
        .join(" ")
}

fn shellish_quote(arg: &str) -> String {
    if arg
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-_=/:.,+".contains(ch))
    {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

pub fn default_name_suffix() -> String {
    std::process::id().to_string()
}

pub fn run_context_from_parts(
    repo_root: Utf8PathBuf,
    codex_home: Utf8PathBuf,
    repo_cache: Utf8PathBuf,
    repo_hash: String,
    image_tag: String,
) -> RunContext {
    RunContext {
        repo_root,
        codex_home,
        repo_cache,
        repo_hash,
        image_tag,
        cpus: "4".into(),
        memory: "8g".into(),
        network: None,
        ssh: false,
        cache_cargo_target: false,
    }
}

pub fn mount_sources(args: &[OsString]) -> Vec<String> {
    args.windows(2)
        .filter(|window| window[0] == "--mount")
        .filter_map(|window| window[1].to_str())
        .filter_map(|mount| {
            mount.split(',').find_map(|field| {
                field
                    .strip_prefix("source=")
                    .or_else(|| field.strip_prefix("src="))
                    .map(ToOwned::to_owned)
            })
        })
        .collect()
}

pub fn env_values(args: &[OsString]) -> Vec<String> {
    args.windows(2)
        .filter(|window| window[0] == "--env")
        .filter_map(|window| window[1].to_str().map(ToOwned::to_owned))
        .collect()
}

pub fn os_args_to_strings(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

pub fn append_runtime_arg(args: &mut Vec<OsString>, arg: impl Into<OsString>) {
    args.push(arg.into());
}

pub fn push_path_arg(args: &mut Vec<OsString>, path: &camino::Utf8Path) {
    args.push(paths::path_to_os(path));
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use super::*;

    fn fake_context() -> RunContext {
        run_context_from_parts(
            Utf8PathBuf::from("/Users/test/project"),
            Utf8PathBuf::from("/Users/test/.wcodex/auth/codex-home"),
            Utf8PathBuf::from("/Users/test/.wcodex/repos/abc/cache"),
            "abc".into(),
            "wcodex-runtime:test".into(),
        )
    }

    #[test]
    fn includes_only_required_mounts() {
        let args = base_run_args(&fake_context(), StdioMode::Batch, "1");
        let strings = os_args_to_strings(&args);

        assert!(strings.contains(&"type=bind,source=/Users/test/project,target=/workspace".into()));
        assert!(strings.contains(
            &"type=bind,source=/Users/test/.wcodex/auth/codex-home,target=/root/.codex".into()
        ));
        assert!(strings.contains(
            &"type=bind,source=/Users/test/.wcodex/repos/abc/cache,target=/cache".into()
        ));
        assert!(strings.contains(&"/tmp".into()));
    }

    #[test]
    fn does_not_mount_host_home_or_secret_dirs() {
        let args = base_run_args(&fake_context(), StdioMode::Batch, "1");
        let sources = mount_sources(&args);

        assert!(!sources.iter().any(|source| source == "/Users/test"));
        assert!(!sources.iter().any(|source| source == "/Users/test/.codex"));
        assert!(!sources.iter().any(|source| source == "/Users/test/.ssh"));
    }

    #[test]
    fn ssh_forwarding_is_absent_unless_requested() {
        let without_ssh = base_run_args(&fake_context(), StdioMode::Batch, "1");
        assert!(!os_args_to_strings(&without_ssh).contains(&"--ssh".into()));

        let mut context = fake_context();
        context.ssh = true;
        let with_ssh = base_run_args(&context, StdioMode::Batch, "1");
        assert!(os_args_to_strings(&with_ssh).contains(&"--ssh".into()));
    }

    #[test]
    fn cache_env_points_under_cache_and_cargo_target_is_opt_in() {
        let args = base_run_args(&fake_context(), StdioMode::Batch, "1");
        let env = env_values(&args);

        assert!(env.contains(&"UV_CACHE_DIR=/cache/uv/cache".into()));
        assert!(env.contains(&"UV_TOOL_DIR=/cache/uv/tools".into()));
        assert!(env.contains(&"UV_PYTHON_INSTALL_DIR=/cache/uv/python".into()));
        assert!(env.contains(&"CARGO_HOME=/cache/cargo".into()));
        assert!(!env.iter().any(|value| value.contains("npm")));
        assert!(!env.contains(&"CARGO_TARGET_DIR=/cache/cargo-target".into()));

        let mut context = fake_context();
        context.cache_cargo_target = true;
        let args = base_run_args(&context, StdioMode::Batch, "1");
        let env = env_values(&args);
        assert!(env.contains(&"CARGO_TARGET_DIR=/cache/cargo-target".into()));
    }

    #[test]
    fn container_command_args_append_codex_args_verbatim() {
        let runtime_args = vec!["--model".into(), "gpt-5".into(), "fix tests".into()];
        let args = container_run_args(&fake_context(), StdioMode::Batch, "1", &runtime_args);
        let strings = os_args_to_strings(&args);

        assert_eq!(
            &strings[strings.len() - 4..],
            &[
                "wcodex-runtime:test".to_string(),
                "--model".to_string(),
                "gpt-5".to_string(),
                "fix tests".to_string()
            ]
        );
    }
}
