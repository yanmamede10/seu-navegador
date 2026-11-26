//! Isolated, platform-agnostic Studio backend (`seu://studio/`).
//!
//! Zero CEF or Win32 dependencies and zero background threads until a workspace
//! command is explicitly started.

pub mod agents;
pub mod git;
pub mod runner;
pub mod workspace;

use agents::{
    detect_local_clis, AgentNotifyFn, AgentOrchestrator, AgentProviderConfig, AgentRole,
    AgentRunSnapshot, BrowserQaSnapshot, DetectedLocalCli,
};
use git::{inspect_git_status, read_git_diff, GitSummary};
use runner::{TerminalManager, TerminalNotifyFn, TerminalSnapshot};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use workspace::{
    create_entry, list_directory, open_workspace_root, read_file_text, search_workspace,
    write_file_text, FileContent, FileEntry, SearchMatch, WorkspaceError,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSummary {
    pub root_path: String,
    pub name: String,
    pub git: GitSummary,
    pub entries: Vec<FileEntry>,
}

#[derive(Default)]
pub struct StudioService {
    workspace_root: Option<PathBuf>,
    terminals: TerminalManager,
    orchestrator: AgentOrchestrator,
    last_qa_snapshot: Option<BrowserQaSnapshot>,
}

impl StudioService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open_workspace(&mut self, raw_path: &str) -> Result<WorkspaceSummary, WorkspaceError> {
        let root = open_workspace_root(raw_path)?;
        self.workspace_root = Some(root);
        self.workspace_summary()
    }

    pub fn workspace_summary(&self) -> Result<WorkspaceSummary, WorkspaceError> {
        let root = self
            .workspace_root
            .as_ref()
            .ok_or(WorkspaceError::NoWorkspaceOpen)?;
        let git = inspect_git_status(root);
        let status_map = git.status_map();
        let entries = list_directory(root, "", &status_map)?;
        let name = root
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| root.to_string_lossy().to_string());
        Ok(WorkspaceSummary {
            root_path: root.to_string_lossy().to_string(),
            name,
            git,
            entries,
        })
    }

    pub fn list_dir(&self, rel_path: &str) -> Result<Vec<FileEntry>, WorkspaceError> {
        let root = self
            .workspace_root
            .as_ref()
            .ok_or(WorkspaceError::NoWorkspaceOpen)?;
        let git = inspect_git_status(root);
        list_directory(root, rel_path, &git.status_map())
    }

    pub fn read_file(&self, rel_path: &str) -> Result<FileContent, WorkspaceError> {
        let root = self
            .workspace_root
            .as_ref()
            .ok_or(WorkspaceError::NoWorkspaceOpen)?;
        read_file_text(root, rel_path)
    }

    pub fn write_file(&self, rel_path: &str, content: &str) -> Result<usize, WorkspaceError> {
        let root = self
            .workspace_root
            .as_ref()
            .ok_or(WorkspaceError::NoWorkspaceOpen)?;
        write_file_text(root, rel_path, content)
    }

    pub fn create_entry(&self, rel_path: &str, is_dir: bool) -> Result<String, WorkspaceError> {
        let root = self
            .workspace_root
            .as_ref()
            .ok_or(WorkspaceError::NoWorkspaceOpen)?;
        create_entry(root, rel_path, is_dir)
    }

    pub fn search(&self, query: &str) -> Result<Vec<SearchMatch>, WorkspaceError> {
        let root = self
            .workspace_root
            .as_ref()
            .ok_or(WorkspaceError::NoWorkspaceOpen)?;
        search_workspace(root, query, 120)
    }

    pub fn git_status(&self) -> Result<GitSummary, WorkspaceError> {
        let root = self
            .workspace_root
            .as_ref()
            .ok_or(WorkspaceError::NoWorkspaceOpen)?;
        Ok(inspect_git_status(root))
    }

    pub fn git_diff(&self, rel_path: Option<&str>) -> Result<String, WorkspaceError> {
        let root = self
            .workspace_root
            .as_ref()
            .ok_or(WorkspaceError::NoWorkspaceOpen)?;
        Ok(read_git_diff(root, rel_path))
    }

    pub fn run_command(
        &mut self,
        command: &str,
        on_update: Option<TerminalNotifyFn>,
    ) -> Result<TerminalSnapshot, String> {
        let root = self
            .workspace_root
            .clone()
            .ok_or_else(|| WorkspaceError::NoWorkspaceOpen.to_string())?;
        self.terminals.spawn_command(&root, command, on_update)
    }

    pub fn kill_command(&mut self, session_id: u64) -> Option<TerminalSnapshot> {
        self.terminals.kill_session(session_id)
    }

    pub fn get_terminal(&self, session_id: u64) -> Option<TerminalSnapshot> {
        self.terminals.get_snapshot(session_id)
    }

    pub fn shutdown(&mut self) {
        self.terminals.kill_all();
    }

    // -------------------------------------------------------------------------
    // Hybrid Multi-Agent Orchestrator & Live Browser QA
    // -------------------------------------------------------------------------

    pub fn detect_local_clis(&self) -> Vec<DetectedLocalCli> {
        detect_local_clis()
    }

    pub fn list_agent_runs(&self) -> Vec<AgentRunSnapshot> {
        self.orchestrator.list_runs()
    }

    pub fn set_last_qa_snapshot(&mut self, snapshot: BrowserQaSnapshot) {
        self.last_qa_snapshot = Some(snapshot);
    }

    pub fn last_qa_snapshot(&self) -> Option<BrowserQaSnapshot> {
        self.last_qa_snapshot.clone()
    }

    pub fn start_agent_run(
        &mut self,
        role: AgentRole,
        config: AgentProviderConfig,
        prompt: String,
        on_update: Option<AgentNotifyFn>,
    ) -> Result<AgentRunSnapshot, String> {
        self.orchestrator.start_run(
            self.workspace_root.clone(),
            role,
            config,
            prompt,
            self.last_qa_snapshot.clone(),
            on_update,
        )
    }

    pub fn approve_agent_patch(
        &self,
        run_id: u64,
        patch_index: usize,
    ) -> Result<AgentRunSnapshot, String> {
        let root = self
            .workspace_root
            .as_ref()
            .ok_or_else(|| WorkspaceError::NoWorkspaceOpen.to_string())?;
        self.orchestrator.approve_patch(root, run_id, patch_index)
    }
}
