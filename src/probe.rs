use anyhow::{bail, Result};

use crate::cli::SandboxMode;
use crate::engine_container::{container_run_args, ContainerEngine, RunContext, StdioMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxSelection {
    Bwrap,
    Landlock,
    Yolo,
}

impl SandboxSelection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bwrap => "bwrap",
            Self::Landlock => "landlock",
            Self::Yolo => "danger-full-access",
        }
    }
}

pub fn select_sandbox(
    engine: &ContainerEngine,
    context: &RunContext,
    mode: SandboxMode,
    allow_yolo_fallback: bool,
) -> Result<SandboxSelection> {
    let bwrap_ok = if matches!(mode, SandboxMode::Auto | SandboxMode::Bwrap) {
        run_bwrap_probe(engine, context)?
    } else {
        false
    };
    let landlock_ok = if matches!(mode, SandboxMode::Auto | SandboxMode::Landlock) && !bwrap_ok {
        run_landlock_probe(engine, context)?
    } else {
        false
    };

    select_from_probe_results(mode, bwrap_ok, landlock_ok, allow_yolo_fallback)
}

pub fn select_from_probe_results(
    mode: SandboxMode,
    bwrap_ok: bool,
    landlock_ok: bool,
    allow_yolo_fallback: bool,
) -> Result<SandboxSelection> {
    match mode {
        SandboxMode::Bwrap if bwrap_ok => Ok(SandboxSelection::Bwrap),
        SandboxMode::Bwrap => bail!("requested bubblewrap sandbox mode, but the probe failed"),
        SandboxMode::Landlock if landlock_ok => Ok(SandboxSelection::Landlock),
        SandboxMode::Landlock => bail!("requested Landlock sandbox mode, but the probe failed"),
        SandboxMode::Yolo => Ok(SandboxSelection::Yolo),
        SandboxMode::Auto => {
            if bwrap_ok {
                return Ok(SandboxSelection::Bwrap);
            }
            if landlock_ok {
                return Ok(SandboxSelection::Landlock);
            }
            if allow_yolo_fallback {
                return Ok(SandboxSelection::Yolo);
            }

            bail!(
                "Codex inner sandbox failed inside the container.\n\
                 Tried:\n\
                   1. bubblewrap-backed Linux sandbox\n\
                   2. legacy Landlock fallback\n\n\
                 Refusing danger-full-access without --allow-yolo-fallback.\n\n\
                 Run:\n\
                   wcodex --allow-yolo-fallback ..."
            )
        }
    }
}

pub fn bwrap_probe_args(context: &RunContext) -> Vec<std::ffi::OsString> {
    let runtime_args = vec![
        "sandbox".into(),
        "linux".into(),
        "--".into(),
        "/bin/sh".into(),
        "-lc".into(),
        "echo bwrap-ok".into(),
    ];
    container_run_args(context, StdioMode::Batch, "probe-bwrap", &runtime_args)
}

pub fn landlock_probe_args(context: &RunContext) -> Vec<std::ffi::OsString> {
    let runtime_args = vec![
        "-c".into(),
        "use_legacy_landlock=true".into(),
        "sandbox".into(),
        "linux".into(),
        "--".into(),
        "/bin/sh".into(),
        "-lc".into(),
        "echo landlock-ok".into(),
    ];
    container_run_args(context, StdioMode::Batch, "probe-landlock", &runtime_args)
}

pub fn codex_arg_prefix(selection: SandboxSelection) -> Vec<String> {
    match selection {
        SandboxSelection::Bwrap => vec![
            "--sandbox".into(),
            "workspace-write".into(),
            "--ask-for-approval".into(),
            "never".into(),
            "-c".into(),
            "sandbox_workspace_write.network_access=true".into(),
            "-c".into(),
            "default_permissions=\"wcodex_container\"".into(),
        ],
        SandboxSelection::Landlock => vec![
            "-c".into(),
            "use_legacy_landlock=true".into(),
            "--sandbox".into(),
            "workspace-write".into(),
            "--ask-for-approval".into(),
            "never".into(),
            "-c".into(),
            "sandbox_workspace_write.network_access=true".into(),
            "-c".into(),
            "default_permissions=\"wcodex_container\"".into(),
        ],
        SandboxSelection::Yolo => vec![
            "--sandbox".into(),
            "danger-full-access".into(),
            "--ask-for-approval".into(),
            "never".into(),
        ],
    }
}

fn run_bwrap_probe(engine: &ContainerEngine, context: &RunContext) -> Result<bool> {
    Ok(engine.status(bwrap_probe_args(context))?.success())
}

fn run_landlock_probe(engine: &ContainerEngine, context: &RunContext) -> Result<bool> {
    Ok(engine.status(landlock_probe_args(context))?.success())
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use crate::engine_container::{os_args_to_strings, run_context_from_parts};

    use super::*;

    fn fake_context() -> RunContext {
        run_context_from_parts(
            Utf8PathBuf::from("/repo"),
            Utf8PathBuf::from("/state/auth/codex-home"),
            Utf8PathBuf::from("/state/repos/hash/cache"),
            "hash".into(),
            "wcodex-runtime:test".into(),
        )
    }

    #[test]
    fn bwrap_probe_uses_codex_linux_sandbox() {
        let args = os_args_to_strings(&bwrap_probe_args(&fake_context()));
        assert!(args.ends_with(&[
            "wcodex-runtime:test".into(),
            "sandbox".into(),
            "linux".into(),
            "--".into(),
            "/bin/sh".into(),
            "-lc".into(),
            "echo bwrap-ok".into(),
        ]));
    }

    #[test]
    fn landlock_probe_sets_legacy_flag() {
        let args = os_args_to_strings(&landlock_probe_args(&fake_context()));
        assert!(args.ends_with(&[
            "wcodex-runtime:test".into(),
            "-c".into(),
            "use_legacy_landlock=true".into(),
            "sandbox".into(),
            "linux".into(),
            "--".into(),
            "/bin/sh".into(),
            "-lc".into(),
            "echo landlock-ok".into(),
        ]));
    }

    #[test]
    fn yolo_prefix_is_explicit_danger_full_access() {
        assert_eq!(
            codex_arg_prefix(SandboxSelection::Yolo),
            vec![
                "--sandbox".to_string(),
                "danger-full-access".to_string(),
                "--ask-for-approval".to_string(),
                "never".to_string()
            ]
        );
    }

    #[test]
    fn workspace_prefix_uses_named_permission_profile() {
        let prefix = codex_arg_prefix(SandboxSelection::Bwrap);
        assert!(prefix.contains(&"--sandbox".into()));
        assert!(prefix.contains(&"workspace-write".into()));
        assert!(prefix.contains(&"default_permissions=\"wcodex_container\"".into()));
    }

    #[test]
    fn automatic_yolo_requires_explicit_fallback_flag() {
        assert!(select_from_probe_results(SandboxMode::Auto, false, false, false).is_err());
        assert_eq!(
            select_from_probe_results(SandboxMode::Auto, false, false, true).unwrap(),
            SandboxSelection::Yolo
        );
    }
}
