use anyhow::{bail, Context, Result};
use camino::Utf8Path;
use clap::Parser;

use crate::cli::{Action, Cli};
use crate::config;
use crate::doctor::{self, DoctorInput};
use crate::engine_container::{
    container_run_args, default_name_suffix, format_argv, ContainerEngine, RunContext, StdioMode,
};
use crate::image::{self, ImageBuildOptions};
use crate::paths;
use crate::probe;
use crate::resign::{self, ResignOptions};
use crate::state::StatePaths;

pub fn main_entry() -> Result<()> {
    let cli = Cli::parse();
    execute(cli)
}

pub fn execute(cli: Cli) -> Result<()> {
    let action = cli.action();
    let state = StatePaths::discover(cli.options.repo_root.as_ref())?;

    match action {
        Action::CleanCache => {
            clean_cache(&state)?;
            Ok(())
        }
        Action::CleanAuth => {
            clean_auth(&state)?;
            Ok(())
        }
        Action::Paths => {
            state.ensure_dirs()?;
            let image_hash = image::runtime_image_hash();
            let image_tag = resolved_image_tag(&cli, &image_hash);
            print_paths(&state, &image_hash, &image_tag);
            Ok(())
        }
        Action::CleanImage => {
            state.ensure_dirs()?;
            let image_hash = image::runtime_image_hash();
            let image_tag = resolved_image_tag(&cli, &image_hash);
            let engine = ContainerEngine::detect()?;
            clean_image(&engine, &state, &image_hash, &image_tag)
        }
        Action::BuildImage { no_cache, pull } => {
            state.ensure_dirs()?;
            let image_hash = image::runtime_image_hash();
            let image_tag = resolved_image_tag(&cli, &image_hash);
            let engine = ContainerEngine::detect()?;
            build_image(
                &engine,
                &state,
                &image_hash,
                ImageBuildOptions {
                    tag: image_tag,
                    cpus: cli.options.cpus,
                    memory: cli.options.memory,
                    no_cache,
                    pull,
                },
            )
        }
        Action::Doctor { codex_auth_probe } => {
            state.ensure_dirs()?;
            let config_path = prepare_config(&state)?;
            let image_hash = image::runtime_image_hash();
            let image_tag = resolved_image_tag(&cli, &image_hash);
            let engine = ContainerEngine::detect()?;
            let initial_image_exists = engine.image_exists(&image_tag)?;
            let generated_image_state_missing = cli.options.image.is_none()
                && !image::image_state_exists(&state.image_dir(&image_hash));
            if cli.options.rebuild_image || !initial_image_exists || generated_image_state_missing {
                build_image(
                    &engine,
                    &state,
                    &image_hash,
                    ImageBuildOptions {
                        tag: image_tag.clone(),
                        cpus: cli.options.cpus.clone(),
                        memory: cli.options.memory.clone(),
                        no_cache: false,
                        pull: true,
                    },
                )?;
            }
            let context = run_context(&cli, &state, image_tag);
            doctor::run(DoctorInput {
                engine: &engine,
                state: &state,
                context: &context,
                image_hash: &image_hash,
                config_path: &config_path,
                initial_image_exists,
                codex_auth_probe,
            })
        }
        Action::Resign { squash } => {
            resign::resign_current_branch(&state.repo_root, ResignOptions { squash })
        }
        Action::Shell => {
            let prepared = prepare_runtime(&cli, &state)?;
            warn_if_dirty(&state);
            exec_container(prepared.engine, prepared.context, vec!["bash".into()])
        }
        Action::Login(args) => {
            let prepared = prepare_runtime(&cli, &state)?;
            let mut runtime_args = vec!["login".into()];
            runtime_args.extend(args);
            exec_container(prepared.engine, prepared.context, runtime_args)
        }
        Action::Run(args) => {
            let prepared = prepare_runtime(&cli, &state)?;
            warn_if_dirty(&state);
            let selection = probe::select_sandbox(
                &prepared.engine,
                &prepared.context,
                cli.options.sandbox_mode,
                cli.options.allow_yolo_fallback,
            )?;
            let mut runtime_args = probe::codex_arg_prefix(selection);
            runtime_args.extend(args);
            exec_container(prepared.engine, prepared.context, runtime_args)
        }
        Action::Exec(args) => {
            let prepared = prepare_runtime(&cli, &state)?;
            warn_if_dirty(&state);
            let selection = probe::select_sandbox(
                &prepared.engine,
                &prepared.context,
                cli.options.sandbox_mode,
                cli.options.allow_yolo_fallback,
            )?;
            let mut runtime_args = probe::codex_arg_prefix(selection);
            runtime_args.push("exec".into());
            runtime_args.extend(args);
            let run_args = container_run_args(
                &prepared.context,
                StdioMode::Batch,
                &default_name_suffix(),
                &runtime_args,
            );
            let status = prepared.engine.status(run_args)?;
            if let Some(code) = status.code() {
                std::process::exit(code);
            }
            bail!("container run was terminated by signal")
        }
    }
}

struct PreparedRuntime {
    engine: ContainerEngine,
    context: RunContext,
}

fn prepare_runtime(cli: &Cli, state: &StatePaths) -> Result<PreparedRuntime> {
    state.ensure_dirs()?;
    prepare_config(state)?;
    let image_hash = image::runtime_image_hash();
    let image_tag = resolved_image_tag(cli, &image_hash);
    let engine = ContainerEngine::detect()?;
    let generated_image_state_missing =
        cli.options.image.is_none() && !image::image_state_exists(&state.image_dir(&image_hash));
    if cli.options.rebuild_image
        || !engine.image_exists(&image_tag)?
        || generated_image_state_missing
    {
        build_image(
            &engine,
            state,
            &image_hash,
            ImageBuildOptions {
                tag: image_tag.clone(),
                cpus: cli.options.cpus.clone(),
                memory: cli.options.memory.clone(),
                no_cache: false,
                pull: true,
            },
        )?;
    }
    let context = run_context(cli, state, image_tag);
    run_write_preflight(&engine, &context)?;
    Ok(PreparedRuntime { engine, context })
}

fn prepare_config(state: &StatePaths) -> Result<camino::Utf8PathBuf> {
    let config_path = config::write_codex_config(&state.codex_home)?;
    config::write_gitconfig(&state.repo_gitconfig, &state.repo_root)?;
    fs_err::copy(&state.repo_gitconfig, &state.cache_gitconfig).with_context(|| {
        format!(
            "failed to copy {} to runtime gitconfig {}",
            state.repo_gitconfig, state.cache_gitconfig
        )
    })?;
    Ok(config_path)
}

fn run_context(cli: &Cli, state: &StatePaths, image_tag: String) -> RunContext {
    RunContext {
        repo_root: state.repo_root.clone(),
        codex_home: state.codex_home.clone(),
        repo_cache: state.repo_cache.clone(),
        repo_hash: state.repo_hash.clone(),
        image_tag,
        cpus: cli.options.cpus.clone(),
        memory: cli.options.memory.clone(),
        network: cli.options.network.clone(),
        ssh: cli.options.ssh,
        cache_cargo_target: cli.options.cache_cargo_target,
    }
}

fn resolved_image_tag(cli: &Cli, _image_hash: &str) -> String {
    cli.options
        .image
        .clone()
        .unwrap_or_else(image::runtime_image_tag)
}

fn build_image(
    engine: &ContainerEngine,
    state: &StatePaths,
    image_hash: &str,
    options: ImageBuildOptions,
) -> Result<()> {
    let context = image::create_build_context()?;
    let args = image::build_args(&context, &options);
    let status = engine.status(args.clone())?;
    if !status.success() {
        bail!(
            "container build failed with status {status}\n\
             build context: {}\n\
             argv: {}",
            context.root,
            format_argv(&engine.binary, &args)
        );
    }

    image::write_image_state(&state.image_dir(image_hash), &options.tag)?;
    Ok(())
}

fn exec_container(engine: ContainerEngine, context: RunContext, runtime_args: Vec<String>) -> ! {
    let run_args = container_run_args(
        &context,
        StdioMode::Interactive,
        &default_name_suffix(),
        &runtime_args,
    );
    exec_interactive(engine, run_args)
}

fn run_write_preflight(engine: &ContainerEngine, context: &RunContext) -> Result<()> {
    let cache_args = vec![
        "/bin/sh".into(),
        "-lc".into(),
        "p=/cache/.wcodex-write-test-$$; touch \"$p\" && rm \"$p\"".into(),
    ];
    let status = engine.status(container_run_args(
        context,
        StdioMode::Batch,
        "preflight-cache",
        &cache_args,
    ))?;
    if !status.success() {
        bail!(
            "cache write probe failed for /cache. The /cache mount is required for uv, cargo, pip, Go, and ccache persistent writes."
        );
    }

    let workspace_args = vec![
        "/bin/sh".into(),
        "-lc".into(),
        "p=/workspace/.wcodex-write-test-$$; touch \"$p\" && rm \"$p\"".into(),
    ];
    let status = engine.status(container_run_args(
        context,
        StdioMode::Batch,
        "preflight-workspace",
        &workspace_args,
    ))?;
    if !status.success() {
        bail!(
            "workspace write probe failed for /workspace. Apple container bind mount ownership may require running as root inside the container; the default runtime image is configured for that."
        );
    }

    Ok(())
}

#[cfg(unix)]
fn exec_interactive(engine: ContainerEngine, args: Vec<std::ffi::OsString>) -> ! {
    use std::os::unix::process::CommandExt;

    let mut cmd = engine.command();
    cmd.args(args);
    let err = cmd.exec();
    eprintln!("failed to exec container runtime: {err}");
    std::process::exit(127);
}

#[cfg(not(unix))]
fn exec_interactive(engine: ContainerEngine, args: Vec<std::ffi::OsString>) -> ! {
    let status = engine.status(args).unwrap_or_else(|err| {
        eprintln!("failed to run container runtime: {err}");
        std::process::exit(127);
    });
    std::process::exit(status.code().unwrap_or(127));
}

fn clean_cache(state: &StatePaths) -> Result<()> {
    if state.repo_cache.exists() {
        fs_err::remove_dir_all(&state.repo_cache)
            .with_context(|| format!("failed to remove {}", state.repo_cache))?;
    }
    println!("removed cache: {}", state.repo_cache);
    Ok(())
}

fn clean_auth(state: &StatePaths) -> Result<()> {
    if state.codex_home.exists() {
        fs_err::remove_dir_all(&state.codex_home)
            .with_context(|| format!("failed to remove {}", state.codex_home))?;
    }
    println!("removed Codex auth home: {}", state.codex_home);
    Ok(())
}

fn clean_image(
    engine: &ContainerEngine,
    state: &StatePaths,
    image_hash: &str,
    image_tag: &str,
) -> Result<()> {
    match engine.delete_image(image_tag) {
        Ok(true) => println!("removed image: {image_tag}"),
        Ok(false) => println!("image was not removed or did not exist: {image_tag}"),
        Err(err) => println!("failed to invoke image delete for {image_tag}: {err:#}"),
    }

    let image_dir = state.image_dir(image_hash);
    if image_dir.exists() {
        fs_err::remove_dir_all(&image_dir)
            .with_context(|| format!("failed to remove {image_dir}"))?;
    }
    println!("removed image state: {image_dir}");
    Ok(())
}

fn print_paths(state: &StatePaths, image_hash: &str, image_tag: &str) {
    println!("state root: {}", state.root);
    println!("repo root: {}", state.repo_root);
    println!("repo hash: {}", state.repo_hash);
    println!("codex home: {}", state.codex_home);
    println!("repo cache: {}", state.repo_cache);
    println!("repo gitconfig: {}", state.repo_gitconfig);
    println!("runtime gitconfig: {}", state.cache_gitconfig);
    println!("image hash: {image_hash}");
    println!("image tag: {image_tag}");
    println!("image state: {}", state.image_dir(image_hash));
}

fn warn_if_dirty(state: &StatePaths) {
    if paths::is_git_dirty(&state.repo_root) {
        eprintln!("warning: repository has uncommitted changes; wcodex will not block this run");
    }
}

#[allow(dead_code)]
fn _assert_utf8_path(_: &Utf8Path) {}
