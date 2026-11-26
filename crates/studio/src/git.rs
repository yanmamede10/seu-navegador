//! Lightweight Git status and unified diff helpers for the active Studio workspace.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn git_cmd(root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root);
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitFileStatus {
    pub rel_path: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GitSummary {
    pub is_repo: bool,
    pub branch: String,
    pub changed_files: Vec<GitFileStatus>,
}

impl GitSummary {
    pub fn status_map(&self) -> HashMap<String, String> {
        self.changed_files
            .iter()
            .map(|f| (f.rel_path.clone(), f.code.clone()))
            .collect()
    }
}

/// Runs `git status --porcelain=v1 -b` inside `root` without ever failing hard
/// if Git is missing or the directory is not a repository.
pub fn inspect_git_status(root: &Path) -> GitSummary {
    let output = git_cmd(root)
        .args(["status", "--porcelain=v1", "-b"])
        .output();

    let Ok(out) = output else {
        return GitSummary::default();
    };
    if !out.status.success() {
        return GitSummary::default();
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_git_porcelain(&stdout)
}

pub fn parse_git_porcelain(stdout: &str) -> GitSummary {
    let mut branch = String::new();
    let mut changed_files = Vec::new();

    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            let clean = rest
                .split("...")
                .next()
                .unwrap_or(rest)
                .trim_start_matches("No commits yet on ")
                .trim();
            branch = clean.to_string();
            continue;
        }
        if line.len() < 4 {
            continue;
        }
        let xy = line[..2].trim();
        let code = if xy.contains('?') {
            "?".to_string()
        } else if xy.contains('A') {
            "A".to_string()
        } else if xy.contains('D') {
            "D".to_string()
        } else if xy.contains('R') {
            "R".to_string()
        } else {
            "M".to_string()
        };
        let raw_path = line[3..].trim();
        let path = raw_path
            .rsplit(" -> ")
            .next()
            .unwrap_or(raw_path)
            .trim_matches('"')
            .replace('\\', "/");
        if !path.is_empty() {
            changed_files.push(GitFileStatus {
                rel_path: path,
                code,
            });
        }
    }

    GitSummary {
        is_repo: true,
        branch: if branch.is_empty() {
            "HEAD".to_string()
        } else {
            branch
        },
        changed_files,
    }
}

/// Returns unified diff for either a single file or the full working tree.
pub fn read_git_diff(root: &Path, rel_path: Option<&str>) -> String {
    let mut cmd = git_cmd(root);
    cmd.args(["diff", "--no-color"]);
    if let Some(p) = rel_path {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            cmd.arg("--").arg(trimmed);
        }
    }
    match cmd.output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_git_porcelain_branch_and_statuses() {
        let sample = "## main...origin/main\n M crates/core/src/lib.rs\nA  crates/studio/src/lib.rs\n?? notes.txt\n";
        let summary = parse_git_porcelain(sample);
        assert!(summary.is_repo);
        assert_eq!(summary.branch, "main");
        assert_eq!(summary.changed_files.len(), 3);
        assert_eq!(summary.changed_files[0].code, "M");
        assert_eq!(summary.changed_files[1].code, "A");
        assert_eq!(summary.changed_files[2].code, "?");
    }
}
