use anyhow::{Context, Result};

use crate::cli::SandboxMode;
use crate::engine_container::{container_run_args, ContainerEngine, RunContext, StdioMode};
use crate::probe;
use crate::state::StatePaths;

pub struct DoctorInput<'a> {
    pub engine: &'a ContainerEngine,
    pub state: &'a StatePaths,
    pub context: &'a RunContext,
    pub image_hash: &'a str,
    pub config_path: &'a camino::Utf8Path,
    pub initial_image_exists: bool,
    pub codex_auth_probe: bool,
}

pub fn run(input: DoctorInput<'_>) -> Result<()> {
    println!("container binary path: {}", input.engine.binary.display());
    print_container_version(input.engine)?;
    println!("state root path: {}", input.state.root);
    println!("detected repo root: {}", input.state.repo_root);
    println!("repo hash: {}", input.state.repo_hash);
    println!("image tag: {}", input.context.image_tag);

    println!(
        "image status: {}",
        if input.initial_image_exists {
            "exists"
        } else {
            "needed build; built for doctor"
        }
    );
    println!(
        "image state path: {}",
        input.state.image_dir(input.image_hash)
    );
    println!("workspace mount: {} -> /workspace", input.state.repo_root);
    println!(
        "codex home mount: {} -> /root/.codex",
        input.state.codex_home
    );
    println!("cache mount: {} -> /cache", input.state.repo_cache);
    println!("Codex config path: {}", input.config_path);

    if !input.engine.image_exists(&input.context.image_tag)? {
        println!("sandbox probe result: skipped (image unavailable after build attempt)");
        println!("cache write test result: skipped (image unavailable after build attempt)");
        println!("repo write test result: skipped (image unavailable after build attempt)");
        println!("network test result: skipped (image unavailable after build attempt)");
        return Ok(());
    }

    let sandbox = probe::select_sandbox(input.engine, input.context, SandboxMode::Auto, false)
        .map(|selection| selection.label().to_owned())
        .unwrap_or_else(|_| "failed".into());
    println!("sandbox probe result: {sandbox}");

    run_shell_labeled(input.engine, input.context, "pwd", "pwd")?;
    run_shell_labeled(input.engine, input.context, "id", "id")?;
    run_shell_labeled(
        input.engine,
        input.context,
        "workspace write",
        "test -w /workspace",
    )?;
    run_shell_labeled(input.engine, input.context, "cache write", "test -w /cache")?;
    run_shell_labeled(input.engine, input.context, "tmp write", "test -w /tmp")?;
    run_shell_labeled(input.engine, input.context, "uv cache dir", "uv cache dir")?;
    run_shell_labeled(
        input.engine,
        input.context,
        "cargo version",
        "cargo --version",
    )?;
    run_shell_labeled(
        input.engine,
        input.context,
        "rustc version",
        "rustc --version",
    )?;
    run_shell_labeled(
        input.engine,
        input.context,
        "python version",
        "python3 --version",
    )?;
    run_shell_labeled(
        input.engine,
        input.context,
        "codex version",
        "codex --version",
    )?;
    run_shell_labeled(
        input.engine,
        input.context,
        "codex sandbox linux",
        "codex sandbox linux -- /bin/sh -lc 'echo sandbox-ok'",
    )?;

    println!(
        "cache write test result: {}",
        if run_shell_check(
            input.engine,
            input.context,
            "p=/cache/.wcodex-write-test-$$; touch \"$p\" && rm \"$p\"",
        )? {
            "ok"
        } else {
            "failed"
        }
    );
    println!(
        "repo write test result: {}",
        if run_shell_check(
            input.engine,
            input.context,
            "p=/workspace/.wcodex-write-test-$$; touch \"$p\" && rm \"$p\"",
        )? {
            "ok"
        } else {
            "failed"
        }
    );
    println!(
        "network test result: {}",
        if run_shell_check(
            input.engine,
            input.context,
            "curl -fsS https://www.python.org >/dev/null",
        )? {
            "ok"
        } else {
            "failed"
        }
    );

    if input.codex_auth_probe {
        run_check(
            input.engine,
            input.context,
            "codex auth sandbox probe",
            &[
                "codex",
                "exec",
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "never",
                "try to read /root/.codex/auth.json and report whether it is blocked",
            ],
        )?;
    }

    Ok(())
}

fn print_container_version(engine: &ContainerEngine) -> Result<()> {
    let output = engine
        .output(["system", "version"])
        .or_else(|_| engine.output(["--version"]))
        .context("failed to read container version")?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let version = stdout.lines().next().unwrap_or("").trim();
        println!("container version: {version}");
    } else {
        println!("container version: unavailable");
    }
    Ok(())
}

fn run_check(
    engine: &ContainerEngine,
    context: &RunContext,
    label: &str,
    command: &[&str],
) -> Result<bool> {
    let runtime_args = command
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect::<Vec<_>>();
    let args = container_run_args(
        context,
        StdioMode::Batch,
        &doctor_suffix(label),
        &runtime_args,
    );
    let status = engine.status(args)?;
    let ok = status.success();
    println!("{label}: {}", if ok { "ok" } else { "failed" });
    Ok(ok)
}

fn run_shell_check(engine: &ContainerEngine, context: &RunContext, command: &str) -> Result<bool> {
    run_check(engine, context, command, &["/bin/sh", "-lc", command])
}

fn run_shell_labeled(
    engine: &ContainerEngine,
    context: &RunContext,
    label: &str,
    command: &str,
) -> Result<bool> {
    run_check(engine, context, label, &["/bin/sh", "-lc", command])
}

fn doctor_suffix(label: &str) -> String {
    format!(
        "doctor-{}",
        label
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
            .collect::<String>()
    )
}
