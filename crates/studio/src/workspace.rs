//! Sandboxed project workspace filesystem operations (tree, read, write, create, search).
//!
//! All paths are canonicalized and strictly confined to the active workspace root
//! so neither the Studio UI nor agents can read or modify files outside the project.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

const MAX_READ_BYTES: u64 = 2 * 1024 * 1024; // 2 MB text cap
const MAX_SEARCH_FILE_BYTES: u64 = 512 * 1024; // 512 KB per file in workspace search

const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".next",
    ".nuxt",
    ".venv",
    "__pycache__",
];

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("no workspace folder is currently open")]
    NoWorkspaceOpen,
    #[error("path escapes workspace boundary")]
    PathOutsideWorkspace,
    #[error("path is not a directory: {0}")]
    NotADirectory(String),
    #[error("file too large to open in editor (> 2 MB)")]
    FileTooLarge,
    #[error("file is not valid UTF-8 text")]
    BinaryOrInvalidUtf8,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub git_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileContent {
    pub rel_path: String,
    pub name: String,
    pub content: String,
    pub size_bytes: usize,
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchMatch {
    pub rel_path: String,
    pub line: usize,
    pub preview: String,
}

/// Validates and canonicalizes a workspace root directory.
pub fn open_workspace_root(raw_path: &str) -> Result<PathBuf, WorkspaceError> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(WorkspaceError::NotADirectory(trimmed.to_string()));
    }
    let path = PathBuf::from(trimmed);
    let canon = fs::canonicalize(&path)?;
    if !canon.is_dir() {
        return Err(WorkspaceError::NotADirectory(trimmed.to_string()));
    }
    Ok(canon)
}

/// Resolves a relative path safely inside `root`, rejecting `..` traversal or
/// symlinks pointing outside `root`.
pub fn resolve_inside_workspace(
    root: &Path,
    rel_path: &str,
    must_exist: bool,
) -> Result<PathBuf, WorkspaceError> {
    let canon_root = fs::canonicalize(root)?;
    let clean_rel = rel_path
        .trim()
        .trim_start_matches(['/', '\\'])
        .replace('\\', "/");

    let mut joined = canon_root.clone();
    for comp in Path::new(&clean_rel).components() {
        match comp {
            Component::Normal(seg) => joined.push(seg),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(WorkspaceError::PathOutsideWorkspace);
            }
        }
    }

    if must_exist {
        let canon = fs::canonicalize(&joined)?;
        if !canon.starts_with(&canon_root) {
            return Err(WorkspaceError::PathOutsideWorkspace);
        }
        return Ok(canon);
    }

    // For new files/dirs, canonicalize the deepest existing ancestor to block symlink escapes.
    let mut ancestor = joined.as_path();
    while !ancestor.exists() {
        let Some(parent) = ancestor.parent() else {
            return Err(WorkspaceError::PathOutsideWorkspace);
        };
        ancestor = parent;
    }
    let canon_ancestor = fs::canonicalize(ancestor)?;
    if !canon_ancestor.starts_with(&canon_root) {
        return Err(WorkspaceError::PathOutsideWorkspace);
    }
    Ok(joined)
}

pub fn list_directory(
    root: &Path,
    rel_dir: &str,
    git_status_map: &HashMap<String, String>,
) -> Result<Vec<FileEntry>, WorkspaceError> {
    let dir_path = if rel_dir.trim().is_empty() {
        root.to_path_buf()
    } else {
        resolve_inside_workspace(root, rel_dir, true)?
    };
    if !dir_path.is_dir() {
        return Err(WorkspaceError::NotADirectory(rel_dir.to_string()));
    }

    let mut entries = Vec::new();
    for item in fs::read_dir(&dir_path)? {
        let Ok(entry) = item else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let is_dir = meta.is_dir();
        if is_dir && name == ".git" {
            continue;
        }
        let full = entry.path();
        let rel = full
            .strip_prefix(root)
            .unwrap_or(&full)
            .to_string_lossy()
            .replace('\\', "/");
        let git_status = git_status_map.get(&rel).cloned();
        entries.push(FileEntry {
            name,
            rel_path: rel,
            is_dir,
            size_bytes: if is_dir { 0 } else { meta.len() },
            git_status,
        });
    }

    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });
    Ok(entries)
}

pub fn read_file_text(root: &Path, rel_path: &str) -> Result<FileContent, WorkspaceError> {
    let full = resolve_inside_workspace(root, rel_path, true)?;
    let meta = fs::metadata(&full)?;
    if meta.len() > MAX_READ_BYTES {
        return Err(WorkspaceError::FileTooLarge);
    }
    let bytes = fs::read(&full)?;
    let content = String::from_utf8(bytes).map_err(|_| WorkspaceError::BinaryOrInvalidUtf8)?;
    let rel_norm = full
        .strip_prefix(root)
        .unwrap_or(&full)
        .to_string_lossy()
        .replace('\\', "/");
    let name = full
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| rel_norm.clone());
    let language = detect_language(&name).to_string();
    Ok(FileContent {
        rel_path: rel_norm,
        name,
        size_bytes: content.len(),
        content,
        language,
    })
}

pub fn write_file_text(
    root: &Path,
    rel_path: &str,
    content: &str,
) -> Result<usize, WorkspaceError> {
    let full = resolve_inside_workspace(root, rel_path, false)?;
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&full, content.as_bytes())?;
    Ok(content.len())
}

pub fn create_entry(root: &Path, rel_path: &str, is_dir: bool) -> Result<String, WorkspaceError> {
    let full = resolve_inside_workspace(root, rel_path, false)?;
    if is_dir {
        fs::create_dir_all(&full)?;
    } else {
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        if !full.exists() {
            fs::write(&full, b"")?;
        }
    }
    let rel_norm = full
        .strip_prefix(root)
        .unwrap_or(&full)
        .to_string_lossy()
        .replace('\\', "/");
    Ok(rel_norm)
}

pub fn search_workspace(
    root: &Path,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchMatch>, WorkspaceError> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Ok(Vec::new());
    }
    let cap = max_results.clamp(1, 200);
    let mut matches = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        if matches.len() >= cap {
            break;
        }
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for item in rd.flatten() {
            if matches.len() >= cap {
                break;
            }
            let name = item.file_name().to_string_lossy().to_string();
            let path = item.path();
            let Ok(meta) = item.metadata() else { continue };
            if meta.is_dir() {
                if !IGNORED_DIRS.contains(&name.as_str()) {
                    stack.push(path);
                }
                continue;
            }
            if meta.len() > MAX_SEARCH_FILE_BYTES {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else { continue };
            if bytes.contains(&0) {
                continue;
            }
            let Ok(text) = String::from_utf8(bytes) else {
                continue;
            };
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            for (idx, line) in text.lines().enumerate() {
                if line.to_ascii_lowercase().contains(&needle) {
                    let preview: String = line.trim().chars().take(140).collect();
                    matches.push(SearchMatch {
                        rel_path: rel.clone(),
                        line: idx + 1,
                        preview,
                    });
                    if matches.len() >= cap {
                        break;
                    }
                }
            }
        }
    }

    Ok(matches)
}

pub fn detect_language(filename: &str) -> &'static str {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".rs") {
        "rust"
    } else if lower.ends_with(".ts") || lower.ends_with(".tsx") {
        "typescript"
    } else if lower.ends_with(".js") || lower.ends_with(".jsx") || lower.ends_with(".mjs") {
        "javascript"
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "html"
    } else if lower.ends_with(".css") || lower.ends_with(".scss") {
        "css"
    } else if lower.ends_with(".json") {
        "json"
    } else if lower.ends_with(".md") || lower.ends_with(".markdown") {
        "markdown"
    } else if lower.ends_with(".py") {
        "python"
    } else {
        "plain"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_workspace_boundary_and_reads_writes_searches() {
        let tmp = std::env::temp_dir().join(format!("seu-studio-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        assert!(fs::create_dir_all(&tmp).is_ok());

        let root = open_workspace_root(&tmp.to_string_lossy());
        assert!(root.is_ok());
        if let Ok(root) = root {
            // Traversal outside workspace MUST fail.
            assert!(matches!(
                resolve_inside_workspace(&root, "../secret.txt", false),
                Err(WorkspaceError::PathOutsideWorkspace)
            ));

            // Write, read, list, search inside workspace.
            assert!(write_file_text(
                &root,
                "src/main.rs",
                "fn main() { println!(\"hello studio\"); }\n"
            )
            .is_ok());
            let file = read_file_text(&root, "src/main.rs");
            assert!(file.is_ok());
            if let Ok(f) = file {
                assert_eq!(f.language, "rust");
                assert!(f.content.contains("hello studio"));
            }

            let found = search_workspace(&root, "hello studio", 10).unwrap_or_default();
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].rel_path, "src/main.rs");
            assert_eq!(found[0].line, 1);
        }

        let _ = fs::remove_dir_all(&tmp);
    }
}
