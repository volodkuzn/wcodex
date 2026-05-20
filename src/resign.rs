use std::ffi::OsStr;
use std::process::{Command, ExitStatus};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct ResignOptions {
    pub squash: bool,
}

pub fn resign_current_branch(repo_root: &Utf8Path, options: ResignOptions) -> Result<()> {
    ensure_clean_worktree(repo_root)?;
    ensure_no_rewrite_in_progress(repo_root)?;

    let branch = git_output(repo_root, ["branch", "--show-current"])
        .context("failed to determine current Git branch")?;
    if branch.is_empty() {
        bail!("cannot resign commits while HEAD is detached");
    }

    let upstream = git_output(
        repo_root,
        [
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
    .with_context(|| {
        format!("branch {branch} has no upstream; set one before running `wcodex resign`")
    })?;
    let base = git_output(repo_root, ["merge-base", "HEAD", "@{upstream}"])
        .with_context(|| format!("failed to find merge-base with upstream {upstream}"))?;
    let old_head =
        git_output(repo_root, ["rev-parse", "HEAD"]).context("failed to read HEAD commit")?;
    let range = format!("{base}..HEAD");
    let commits = git_output(repo_root, ["rev-list", "--reverse", range.as_str()])
        .context("failed to list current-branch commits")?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if commits.is_empty() {
        bail!("branch {branch} has no commits ahead of upstream {upstream}");
    }

    let backup_ref = backup_ref(&branch);
    git_status(
        repo_root,
        ["update-ref", backup_ref.as_str(), old_head.as_str()],
    )
    .with_context(|| format!("failed to create backup ref {backup_ref}"))?;

    let result = if options.squash {
        squash_signed(repo_root, &base, &old_head)
    } else {
        recreate_each_commit_signed(repo_root, &base, &commits)
    };

    if let Err(err) = result {
        bail!(
            "{err:#}\n\nBackup ref preserved at {backup_ref} ({old_head}). Resolve the Git state or restore with:\n  git reset --hard {backup_ref}"
        );
    }

    println!(
        "resigned {} commit{} on branch {branch}; backup ref: {backup_ref}",
        commits.len(),
        if commits.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

fn squash_signed(repo_root: &Utf8Path, base: &str, old_head: &str) -> Result<()> {
    git_status(repo_root, ["reset", "--soft", base])
        .context("failed to soft-reset to branch base")?;
    git_status(repo_root, ["commit", "-S", "--allow-empty", "-C", old_head])
        .context("failed to create signed squash commit")?;
    Ok(())
}

fn recreate_each_commit_signed(repo_root: &Utf8Path, base: &str, commits: &[String]) -> Result<()> {
    let range = format!("{base}..HEAD");
    let merge_commits = git_output(repo_root, ["rev-list", "--merges", range.as_str()])
        .context("failed to check branch for merge commits")?;
    if !merge_commits.trim().is_empty() {
        bail!("non-squash resign currently supports linear branches only; rerun with `wcodex resign --squash` or handle merge commits manually");
    }

    git_status(repo_root, ["reset", "--hard", base]).context("failed to reset branch to base")?;
    for commit in commits {
        git_status(
            repo_root,
            [
                "cherry-pick",
                "--allow-empty",
                "--no-commit",
                commit.as_str(),
            ],
        )
        .with_context(|| format!("failed to cherry-pick {commit}"))?;
        git_status(
            repo_root,
            ["commit", "-S", "--allow-empty", "-C", commit.as_str()],
        )
        .with_context(|| format!("failed to create signed replacement for {commit}"))?;
    }
    Ok(())
}

fn ensure_clean_worktree(repo_root: &Utf8Path) -> Result<()> {
    let status = git_output(repo_root, ["status", "--porcelain"])
        .context("failed to inspect Git worktree status")?;
    if !status.is_empty() {
        bail!("refusing to rewrite commits with a dirty worktree; commit, stash, or remove changes first");
    }
    Ok(())
}

fn ensure_no_rewrite_in_progress(repo_root: &Utf8Path) -> Result<()> {
    for marker in [
        "rebase-merge",
        "rebase-apply",
        "CHERRY_PICK_HEAD",
        "MERGE_HEAD",
        "REVERT_HEAD",
    ] {
        let path = git_path(repo_root, marker)?;
        if path.exists() {
            bail!("refusing to rewrite commits while Git operation is in progress: {marker}");
        }
    }
    Ok(())
}

fn git_path(repo_root: &Utf8Path, name: &str) -> Result<Utf8PathBuf> {
    let path = git_output(repo_root, ["rev-parse", "--git-path", name])
        .with_context(|| format!("failed to resolve Git path {name}"))?;
    Ok(repo_root.join(path))
}

fn git_output<I, S>(repo_root: &Utf8Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_command(repo_root, args)
        .output()
        .context("failed to run git")?;
    if !output.status.success() {
        bail!(
            "git failed with status {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn git_status<I, S>(repo_root: &Utf8Path, args: I) -> Result<ExitStatus>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = git_command(repo_root, args)
        .status()
        .context("failed to run git")?;
    if !status.success() {
        bail!("git failed with status {status}");
    }
    Ok(status)
}

fn git_command<I, S>(repo_root: &Utf8Path, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command.current_dir(repo_root).args(args);
    command
}

fn backup_ref(branch: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!(
        "refs/wcodex/resign/{}-{timestamp}",
        sanitize_ref_fragment(branch)
    )
}

fn sanitize_ref_fragment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    sanitized.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_ref_sanitizes_branch_name() {
        let reference = backup_ref("feature/sign commits");
        assert!(reference.starts_with("refs/wcodex/resign/feature-sign-commits-"));
        assert!(!reference.contains(' '));
    }
}
