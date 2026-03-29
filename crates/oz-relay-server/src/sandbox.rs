// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Sandbox executor — process isolation for build sessions.
//!
//! Security fix #2: On Linux with nsjail available, commands run inside nsjail
//! with network isolation, PID namespace, and filesystem restrictions.
//! Falls back to direct execution only in tests or when nsjail is unavailable.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

/// Configuration for a sandbox session.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Used when agent bridge is wired into request path
pub struct SandboxConfig {
    /// Path to the ArcFlow source repository.
    pub source_repo: PathBuf,
    /// Maximum runtime for the sandbox session.
    pub timeout: Duration,
    /// Whether to disable network access (true in production).
    pub disable_network: bool,
    /// Path to nsjail config file (None = direct execution for tests).
    pub nsjail_config: Option<PathBuf>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            source_repo: PathBuf::from("."),
            timeout: Duration::from_secs(30 * 60),
            disable_network: true,
            nsjail_config: None,
        }
    }
}

/// Result of a sandbox session.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used when agent bridge is wired into request path
pub struct SandboxResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub worktree_path: PathBuf,
    pub timed_out: bool,
}

/// Create an isolated git worktree for the build session.
#[allow(dead_code)] // Used when agent bridge is wired into request path
pub async fn create_worktree(repo: &Path, session_id: &str) -> Result<PathBuf, String> {
    let worktree_path = repo.join(format!("../.relay-worktrees/{}", session_id));

    let branch_name = format!("relay/{}", session_id);
    let output = Command::new("git")
        .args(["worktree", "add", "-b", &branch_name])
        .arg(&worktree_path)
        .arg("HEAD")
        .current_dir(repo)
        .output()
        .await
        .map_err(|e| format!("failed to create worktree: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(worktree_path)
}

/// Remove a git worktree after the session completes.
#[allow(dead_code)] // Used when agent bridge is wired into request path
pub async fn remove_worktree(repo: &Path, worktree_path: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(worktree_path)
        .current_dir(repo)
        .output()
        .await
        .map_err(|e| format!("failed to remove worktree: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "git worktree remove failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(())
}

/// Check if nsjail is available on this system.
fn nsjail_available() -> bool {
    std::process::Command::new("nsjail")
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Run a command inside the sandbox with timeout enforcement.
///
/// FIX #2: When `nsjail_config` is provided and nsjail is available,
/// the command runs inside nsjail with full namespace isolation.
/// Otherwise falls back to direct execution (tests, macOS dev).
pub async fn run_sandboxed(
    command: &str,
    args: &[&str],
    working_dir: &Path,
    timeout: Duration,
) -> SandboxResult {
    run_sandboxed_with_config(command, args, working_dir, timeout, None).await
}

/// Run a command with optional nsjail config.
pub async fn run_sandboxed_with_config(
    command: &str,
    args: &[&str],
    working_dir: &Path,
    timeout: Duration,
    nsjail_config: Option<&Path>,
) -> SandboxResult {
    let child = if let Some(config_path) = nsjail_config.filter(|_| nsjail_available()) {
        // Production path: run inside nsjail
        let mut nsjail_args = vec![
            "--config",
            config_path.to_str().unwrap_or(""),
            "--",
            command,
        ];
        nsjail_args.extend(args);

        Command::new("nsjail")
            .args(&nsjail_args)
            .current_dir(working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    } else {
        // Test/dev fallback: direct execution
        Command::new(command)
            .args(args)
            .current_dir(working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    };

    let child = match child {
        Ok(c) => c,
        Err(e) => {
            return SandboxResult {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("failed to spawn: {}", e),
                worktree_path: working_dir.to_path_buf(),
                timed_out: false,
            }
        }
    };

    let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

    match result {
        Ok(Ok(output)) => SandboxResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            worktree_path: working_dir.to_path_buf(),
            timed_out: false,
        },
        Ok(Err(e)) => SandboxResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("process error: {}", e),
            worktree_path: working_dir.to_path_buf(),
            timed_out: false,
        },
        Err(_) => SandboxResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: "sandbox timeout exceeded".into(),
            worktree_path: working_dir.to_path_buf(),
            timed_out: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_sandboxed_echo() {
        let result = run_sandboxed(
            "echo",
            &["hello sandbox"],
            Path::new("."),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello sandbox"));
        assert!(!result.timed_out);
    }

    #[tokio::test]
    async fn run_sandboxed_timeout() {
        let result = run_sandboxed(
            "sleep",
            &["10"],
            Path::new("."),
            Duration::from_millis(100),
        )
        .await;
        assert!(result.timed_out);
    }

    #[tokio::test]
    async fn run_sandboxed_failure() {
        let result = run_sandboxed(
            "false",
            &[],
            Path::new("."),
            Duration::from_secs(5),
        )
        .await;
        assert_ne!(result.exit_code, 0);
        assert!(!result.timed_out);
    }
}
