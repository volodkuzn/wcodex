use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "wcodex",
    version,
    about = "Run Codex inside Apple's container runtime with narrow host mounts.",
    trailing_var_arg = true
)]
pub struct Cli {
    #[command(flatten)]
    pub options: GlobalOptions,

    #[command(subcommand)]
    pub command: Option<CommandKind>,

    #[arg(
        value_name = "codex args",
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    pub codex_args: Vec<String>,
}

#[derive(Debug, Clone, Args)]
pub struct GlobalOptions {
    #[arg(long, global = true, value_name = "tag")]
    pub image: Option<String>,

    #[arg(long, global = true)]
    pub rebuild_image: bool,

    #[arg(long, global = true)]
    pub allow_yolo_fallback: bool,

    #[arg(long, global = true, value_enum, default_value_t = SandboxMode::Auto)]
    pub sandbox_mode: SandboxMode,

    #[arg(long, global = true)]
    pub ssh: bool,

    #[arg(long, global = true, default_value = "4", value_name = "n")]
    pub cpus: String,

    #[arg(long, global = true, default_value = "8g", value_name = "size")]
    pub memory: String,

    #[arg(long, global = true, value_name = "name")]
    pub network: Option<String>,

    #[arg(long, global = true)]
    pub cache_cargo_target: bool,

    #[arg(long, global = true, value_name = "path")]
    pub repo_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SandboxMode {
    Auto,
    Bwrap,
    Yolo,
}

#[derive(Debug, Clone, Subcommand)]
pub enum CommandKind {
    /// Run the Codex TUI or pass arguments through to Codex.
    Run(PassThroughArgs),
    /// Run `codex exec <prompt...>`.
    Exec(PassThroughArgs),
    /// Run `codex login`.
    Login(PassThroughArgs),
    /// Start a shell in the runtime container.
    Shell,
    /// Run diagnostics.
    Doctor(DoctorArgs),
    /// Recreate current branch commits as signed host Git commits.
    Resign(ResignArgs),
    /// Build the runtime image.
    BuildImage(BuildImageArgs),
    /// Remove this repository's persistent cache.
    CleanCache,
    /// Remove the isolated Codex auth home.
    CleanAuth,
    /// Remove the generated runtime image.
    CleanImage,
    /// Print resolved state and mount paths.
    Paths,
}

#[derive(Debug, Clone, Args)]
pub struct PassThroughArgs {
    #[arg(
        value_name = "args",
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Args)]
pub struct DoctorArgs {
    #[arg(long)]
    pub codex_auth_probe: bool,
}

#[derive(Debug, Clone, Args)]
pub struct BuildImageArgs {
    #[arg(long)]
    pub no_cache: bool,

    #[arg(long)]
    pub pull: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ResignArgs {
    /// Squash all current-branch commits into one signed commit.
    #[arg(long)]
    pub squash: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Run(Vec<String>),
    Exec(Vec<String>),
    Login(Vec<String>),
    Shell,
    Doctor { codex_auth_probe: bool },
    Resign { squash: bool },
    BuildImage { no_cache: bool, pull: bool },
    CleanCache,
    CleanAuth,
    CleanImage,
    Paths,
}

impl Cli {
    pub fn action(&self) -> Action {
        match &self.command {
            None => Action::Run(self.codex_args.clone()),
            Some(CommandKind::Run(args)) => Action::Run(args.args.clone()),
            Some(CommandKind::Exec(args)) => Action::Exec(args.args.clone()),
            Some(CommandKind::Login(args)) => Action::Login(args.args.clone()),
            Some(CommandKind::Shell) => Action::Shell,
            Some(CommandKind::Doctor(args)) => Action::Doctor {
                codex_auth_probe: args.codex_auth_probe,
            },
            Some(CommandKind::Resign(args)) => Action::Resign {
                squash: args.squash,
            },
            Some(CommandKind::BuildImage(args)) => Action::BuildImage {
                no_cache: args.no_cache,
                pull: args.pull,
            },
            Some(CommandKind::CleanCache) => Action::CleanCache,
            Some(CommandKind::CleanAuth) => Action::CleanAuth,
            Some(CommandKind::CleanImage) => Action::CleanImage,
            Some(CommandKind::Paths) => Action::Paths,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_root_codex_args_exactly() {
        let cli = Cli::try_parse_from([
            "wcodex",
            "--",
            "--model",
            "gpt-5",
            "-c",
            "foo=bar",
            "fix tests",
        ])
        .unwrap();

        assert_eq!(
            cli.action(),
            Action::Run(vec![
                "--model".into(),
                "gpt-5".into(),
                "-c".into(),
                "foo=bar".into(),
                "fix tests".into()
            ])
        );
    }

    #[test]
    fn preserves_root_codex_args_without_separator() {
        let cli = Cli::try_parse_from(["wcodex", "--model", "gpt-5", "fix tests"]).unwrap();

        assert_eq!(
            cli.action(),
            Action::Run(vec!["--model".into(), "gpt-5".into(), "fix tests".into()])
        );
    }

    #[test]
    fn preserves_exec_args_exactly() {
        let cli =
            Cli::try_parse_from(["wcodex", "exec", "--model", "gpt-5", "repair --the --suite"])
                .unwrap();

        assert_eq!(
            cli.action(),
            Action::Exec(vec![
                "--model".into(),
                "gpt-5".into(),
                "repair --the --suite".into()
            ])
        );
    }

    #[test]
    fn parses_global_flags_before_command() {
        let cli =
            Cli::try_parse_from(["wcodex", "--ssh", "--cpus", "8", "--memory", "16g", "shell"])
                .unwrap();

        assert!(cli.options.ssh);
        assert_eq!(cli.options.cpus, "8");
        assert_eq!(cli.options.memory, "16g");
        assert_eq!(cli.action(), Action::Shell);
    }

    #[test]
    fn parses_resign_squash() {
        let cli = Cli::try_parse_from(["wcodex", "resign", "--squash"]).unwrap();

        assert_eq!(cli.action(), Action::Resign { squash: true });
    }
}
