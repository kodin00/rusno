use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::process::Command;

use crate::config::AppConfig;

/// Git operations for the deploy pipeline. All commands run with the
/// appropriate GIT_SSH_COMMAND when ssh_mode is rusno-managed.

pub struct GitOps {
    /// Path to the rusno-managed SSH key (used when ssh_mode = rusno-managed).
    ssh_key_path: Option<PathBuf>,
}

impl GitOps {
    pub fn new(config: &AppConfig) -> Self {
        let ssh_mode = config.ssh_mode.as_str();
        let key = PathBuf::from(&config.data_dir)
            .join("ssh")
            .join("rusno_ed25519");
        let ssh_key_path = if ssh_mode == "rusno-managed" && key.exists() {
            Some(key)
        } else {
            None
        };
        Self { ssh_key_path }
    }

    fn base_command(&self, cwd: Option<&Path>) -> Command {
        let mut cmd = Command::new("git");
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        if let Some(key) = &self.ssh_key_path {
            let ssh_cmd = format!(
                "ssh -i {} -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new",
                key.display()
            );
            cmd.env("GIT_SSH_COMMAND", ssh_cmd);
        }
        cmd
    }

    /// Clone a repo into the target folder.
    pub async fn clone(&self, url: &str, target: &Path) -> Result<String> {
        let mut cmd = self.base_command(None);
        cmd.args(["clone", url, &target.to_string_lossy()]);

        let output = cmd.output().await.context("git clone failed to spawn")?;
        let combined = format_output(&output);
        if !output.status.success() {
            anyhow::bail!("git clone failed:\n{combined}");
        }
        Ok(combined)
    }

    /// Fetch + checkout branch + pull --ff-only. Stashes local changes first
    /// if the tree is dirty, then retries the pull.
    pub async fn fetch_checkout_pull(&self, repo: &Path, branch: &str) -> Result<String> {
        let mut log = String::new();

        // fetch
        let out = self
            .base_command(Some(repo))
            .args(["fetch", "origin"])
            .output()
            .await?;
        log.push_str(&format_output(&out));
        if !out.status.success() {
            anyhow::bail!("git fetch failed:\n{}", format_output(&out));
        }

        // checkout branch
        let out = self
            .base_command(Some(repo))
            .args(["checkout", branch])
            .output()
            .await?;
        log.push_str(&format_output(&out));
        if !out.status.success() {
            anyhow::bail!("git checkout {} failed:\n{}", branch, format_output(&out));
        }

        // pull --ff-only; on failure stash and retry once
        let out = self
            .base_command(Some(repo))
            .args(["pull", "--ff-only"])
            .output()
            .await?;
        log.push_str(&format_output(&out));

        if !out.status.success() {
            // stash local changes (compose/.env edits may be tracked) and retry
            let stash_out = self
                .base_command(Some(repo))
                .args(["stash"])
                .output()
                .await?;
            log.push_str(&format_output(&stash_out));

            let retry = self
                .base_command(Some(repo))
                .args(["pull", "--ff-only"])
                .output()
                .await?;
            log.push_str(&format_output(&retry));
            if !retry.status.success() {
                anyhow::bail!("git pull failed:\n{}", format_output(&retry));
            }
        }

        Ok(log)
    }

    /// Rollback: fetch + checkout a specific commit (detached HEAD).
    pub async fn fetch_checkout_commit(&self, repo: &Path, commit: &str) -> Result<String> {
        let mut log = String::new();

        let out = self
            .base_command(Some(repo))
            .args(["fetch", "origin"])
            .output()
            .await?;
        log.push_str(&format_output(&out));
        if !out.status.success() {
            anyhow::bail!("git fetch failed:\n{}", format_output(&out));
        }

        let out = self
            .base_command(Some(repo))
            .args(["checkout", commit])
            .output()
            .await?;
        log.push_str(&format_output(&out));
        if !out.status.success() {
            anyhow::bail!("git checkout {commit} failed:\n{}", format_output(&out));
        }

        Ok(log)
    }

    /// Get the current HEAD commit SHA.
    pub async fn head_sha(&self, repo: &Path) -> Result<String> {
        let out = self
            .base_command(Some(repo))
            .args(["rev-parse", "HEAD"])
            .output()
            .await?;
        if !out.status.success() {
            anyhow::bail!("git rev-parse failed:\n{}", format_output(&out));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Get the commit subject line for a SHA.
    pub async fn commit_message(&self, repo: &Path) -> Result<String> {
        let out = self
            .base_command(Some(repo))
            .args(["log", "-1", "--pretty=%s"])
            .output()
            .await?;
        if !out.status.success() {
            return Ok(String::new());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Detect the default branch of a remote repo (for the register form autodetect).
    /// Returns e.g. "main" or "master".
    pub async fn detect_default_branch(&self, url: &str) -> Option<String> {
        let mut cmd = self.base_command(None);
        cmd.args(["ls-remote", "--symref", url, "HEAD"]);
        let output = cmd.output().await.ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Line looks like: "ref: refs/heads/main	HEAD"
        for line in stdout.lines() {
            if let Some(rest) = line.strip_prefix("ref: refs/heads/") {
                let branch = rest.split_whitespace().next()?;
                return Some(branch.to_string());
            }
        }
        None
    }

    /// Discard local changes to a path (reset to repo version — used by the compose editor).
    pub async fn checkout_path(&self, repo: &Path, path: &str) -> Result<String> {
        let out = self
            .base_command(Some(repo))
            .args(["checkout", "--", path])
            .output()
            .await?;
        let combined = format_output(&out);
        if !out.status.success() {
            anyhow::bail!("git checkout -- {path} failed:\n{combined}");
        }
        Ok(combined)
    }

    /// Whether the repo folder exists and looks like a git repo.
    pub fn is_repo(folder: &Path) -> bool {
        folder.join(".git").exists()
    }
}

fn format_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut combined = String::new();
    if !stdout.is_empty() {
        combined.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }
    combined
}
