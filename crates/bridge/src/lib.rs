//! Typed JSON bridge protocol between the privileged `seu://` UI and Rust.
//!
//! Strictly enforces origin gating (`seu://` only) and validates payloads
//! before dispatching them to `BrowserCore` or window/engine actions.

use serde::{Deserialize, Serialize};
use seu_blocker::{BlockerService, BlockerStats};
use seu_core::{
    memory::{MemoryPolicy, MemoryStats},
    shortcuts::QuickShortcut,
    BrowserCore, CoreError, CoreSettings, DevToolsDock, GroupId, Locale, SearchCategory,
    SearchEngine, TabId, TabLifecycleState, WindowId, WindowSnapshot,
};
use seu_storage::{
    importer::{
        detect_browser_profiles, run_profile_import, DetectedBrowserProfile, ImportOptions,
        ImportReport,
    },
    BookmarkItem, DownloadItem, HistoryItem, SitePermissionItem, Storage,
};
use seu_studio::{
    agents::{
        AgentNotifyFn, AgentProviderConfig, AgentRole, AgentRunSnapshot, BrowserQaSnapshot,
        DetectedLocalCli,
    },
    git::GitSummary,
    runner::{TerminalNotifyFn, TerminalSnapshot},
    workspace::{FileContent, FileEntry, SearchMatch},
    StudioService, WorkspaceSummary,
};
use seu_themes::{LoadedTheme, ThemeRegistry, ThemeSummary};
use thiserror::Error;

pub const MAX_BRIDGE_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BridgeError {
    #[error("access denied: bridge is restricted to seu:// origins")]
    OriginDenied,
    #[error("payload exceeds maximum size")]
    PayloadTooLarge,
    #[error("invalid json request: {0}")]
    InvalidJson(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error(transparent)]
    Core(#[from] CoreError),
}

/// Checks whether a frame URL is allowed to access `window.seu`.
pub fn is_privileged_origin(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("seu://")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "method", content = "params")]
pub enum BridgeRequest {
    #[serde(rename = "windows.getSnapshot")]
    WindowGetSnapshot { window_id: WindowId },
    #[serde(rename = "windows.create")]
    WindowCreate {
        incognito: bool,
        url: Option<String>,
    },
    #[serde(rename = "windows.minimize")]
    WindowMinimize { window_id: WindowId },
    #[serde(rename = "windows.maximize")]
    WindowMaximize { window_id: WindowId },
    #[serde(rename = "windows.close")]
    WindowClose { window_id: WindowId },
    #[serde(rename = "windows.startDrag")]
    WindowStartDrag { window_id: WindowId },
    #[serde(rename = "windows.setShellBounds")]
    WindowSetShellBounds { window_id: WindowId, height: u32 },

    #[serde(rename = "tabs.create")]
    TabCreate {
        window_id: WindowId,
        url: Option<String>,
        activate: Option<bool>,
    },
    #[serde(rename = "tabs.close")]
    TabClose { tab_id: TabId },
    #[serde(rename = "tabs.activate")]
    TabActivate { window_id: WindowId, tab_id: TabId },
    #[serde(rename = "tabs.navigate")]
    TabNavigate { tab_id: TabId, input: String },
    #[serde(rename = "tabs.reload")]
    TabReload { tab_id: TabId },
    #[serde(rename = "tabs.back")]
    TabBack { tab_id: TabId },
    #[serde(rename = "tabs.forward")]
    TabForward { tab_id: TabId },
    #[serde(rename = "tabs.pin")]
    TabPin { tab_id: TabId },
    #[serde(rename = "tabs.duplicate")]
    TabDuplicate { tab_id: TabId },
    #[serde(rename = "tabs.reopenClosed")]
    TabReopenClosed { window_id: WindowId },
    #[serde(rename = "tabs.reorder")]
    TabReorder {
        window_id: WindowId,
        tab_id: TabId,
        to_index: usize,
    },
    #[serde(rename = "tabs.detachToWindow")]
    TabDetachToWindow { tab_id: TabId },

    #[serde(rename = "tabGroups.create")]
    TabGroupCreate {
        window_id: WindowId,
        title: String,
        color: String,
        tab_ids: Vec<TabId>,
    },
    #[serde(rename = "tabGroups.toggleCollapse")]
    TabGroupToggleCollapse {
        window_id: WindowId,
        group_id: GroupId,
    },
    #[serde(rename = "tabGroups.assignTab")]
    TabGroupAssignTab {
        tab_id: TabId,
        group_id: Option<GroupId>,
    },
    #[serde(rename = "tabGroups.ungroup")]
    TabGroupUngroup {
        window_id: WindowId,
        group_id: GroupId,
    },

    #[serde(rename = "overlay.open")]
    OverlayOpen {
        window_id: WindowId,
        panel: String,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    #[serde(rename = "overlay.close")]
    OverlayClose { window_id: WindowId },

    #[serde(rename = "settings.get")]
    SettingsGet,
    #[serde(rename = "settings.setLocale")]
    SettingsSetLocale { locale: String },
    #[serde(rename = "settings.setSearchEngine")]
    SettingsSetSearchEngine { engine: SearchEngine },

    #[serde(rename = "search.submit")]
    SearchSubmit {
        tab_id: TabId,
        query: String,
        #[serde(default)]
        category: SearchCategory,
    },

    #[serde(rename = "gameMode.toggle")]
    GameModeToggle,

    #[serde(rename = "themes.list")]
    ThemesList,
    #[serde(rename = "themes.getActive")]
    ThemesGetActive,
    #[serde(rename = "themes.setActive")]
    ThemesSetActive { id: String },

    #[serde(rename = "shortcuts.set")]
    ShortcutsSet { action: String, combo: String },
    #[serde(rename = "shortcuts.reset")]
    ShortcutsReset,
    #[serde(rename = "quickShortcuts.set")]
    QuickShortcutsSet { items: Vec<QuickShortcut> },

    #[serde(rename = "import.listProfiles")]
    ImportListProfiles,
    #[serde(rename = "import.run")]
    ImportRun(ImportOptions),
    #[serde(rename = "data.summary")]
    DataSummary,

    #[serde(rename = "memory.getStats")]
    MemoryGetStats,
    #[serde(rename = "memory.sleepTab")]
    MemorySleepTab { tab_id: TabId },
    #[serde(rename = "memory.sleepInactive")]
    MemorySleepInactive,
    #[serde(rename = "memory.setPolicy")]
    MemorySetPolicy { policy: MemoryPolicy },

    #[serde(rename = "blocker.getStats")]
    BlockerGetStats,
    #[serde(rename = "blocker.setEnabled")]
    BlockerSetEnabled { enabled: bool },
    #[serde(rename = "blocker.toggleSite")]
    BlockerToggleSite { domain: String },
    #[serde(rename = "blocker.updateLists")]
    BlockerUpdateLists,

    #[serde(rename = "history.list")]
    HistoryList {
        query: Option<String>,
        limit: Option<usize>,
    },
    #[serde(rename = "history.delete")]
    HistoryDelete { id: i64 },
    #[serde(rename = "history.clear")]
    HistoryClear,

    #[serde(rename = "bookmarks.list")]
    BookmarksList,
    #[serde(rename = "bookmarks.isBookmarked")]
    BookmarksIsBookmarked { url: String },
    #[serde(rename = "bookmarks.toggle")]
    BookmarksToggle {
        title: String,
        url: String,
        parent_id: Option<i64>,
    },
    #[serde(rename = "bookmarks.createFolder")]
    BookmarksCreateFolder {
        title: String,
        parent_id: Option<i64>,
    },
    #[serde(rename = "bookmarks.delete")]
    BookmarksDelete { id: i64 },
    #[serde(rename = "bookmarks.importNetscape")]
    BookmarksImportNetscape { html: String },

    #[serde(rename = "downloads.list")]
    DownloadsList,
    #[serde(rename = "downloads.action")]
    DownloadsAction { id: i64, action: String },
    #[serde(rename = "downloads.clearFinished")]
    DownloadsClearFinished,

    #[serde(rename = "permissions.list")]
    PermissionsList,
    #[serde(rename = "permissions.set")]
    PermissionsSet {
        origin: String,
        permission: String,
        decision: String,
    },
    #[serde(rename = "permissions.delete")]
    PermissionsDelete { origin: String, permission: String },
    #[serde(rename = "permissions.respondPrompt")]
    PermissionsRespondPrompt {
        window_id: WindowId,
        prompt_id: u64,
        allow: bool,
        remember: bool,
    },

    #[serde(rename = "settings.setRestoreSession")]
    SettingsSetRestoreSession { enabled: bool },

    #[serde(rename = "split.toggle")]
    SplitToggle {
        window_id: WindowId,
        tab_id: Option<TabId>,
    },
    #[serde(rename = "split.close")]
    SplitClose { window_id: WindowId },
    #[serde(rename = "split.setRatio")]
    SplitSetRatio { window_id: WindowId, ratio_pct: u8 },

    #[serde(rename = "devtools.toggle")]
    DevToolsToggle { window_id: WindowId },
    #[serde(rename = "devtools.close")]
    DevToolsClose { window_id: WindowId },
    #[serde(rename = "devtools.setDock")]
    DevToolsSetDock {
        window_id: WindowId,
        dock: DevToolsDock,
    },
    #[serde(rename = "devtools.setRatio")]
    DevToolsSetRatio { window_id: WindowId, ratio_pct: u8 },

    #[serde(rename = "studio.setMode")]
    StudioSetMode { window_id: WindowId, mode: String },
    #[serde(rename = "studio.pickFolder")]
    StudioPickFolder { window_id: WindowId },
    #[serde(rename = "studio.openWorkspace")]
    StudioOpenWorkspace { path: String },
    #[serde(rename = "studio.getWorkspace")]
    StudioGetWorkspace,
    #[serde(rename = "studio.listDir")]
    StudioListDir { rel_path: String },
    #[serde(rename = "studio.readFile")]
    StudioReadFile { rel_path: String },
    #[serde(rename = "studio.writeFile")]
    StudioWriteFile { rel_path: String, content: String },
    #[serde(rename = "studio.createEntry")]
    StudioCreateEntry { rel_path: String, is_dir: bool },
    #[serde(rename = "studio.search")]
    StudioSearch { query: String },
    #[serde(rename = "studio.gitStatus")]
    StudioGitStatus,
    #[serde(rename = "studio.gitDiff")]
    StudioGitDiff { rel_path: Option<String> },
    #[serde(rename = "studio.runCommand")]
    StudioRunCommand { command: String },
    #[serde(rename = "studio.killCommand")]
    StudioKillCommand { session_id: u64 },
    #[serde(rename = "studio.getTerminal")]
    StudioGetTerminal { session_id: u64 },

    #[serde(rename = "studio.detectClis")]
    StudioDetectClis,
    #[serde(rename = "studio.agentsConfigGet")]
    StudioAgentsConfigGet,
    #[serde(rename = "studio.agentsConfigSave")]
    StudioAgentsConfigSave { config_json: String },
    #[serde(rename = "studio.agentsListRuns")]
    StudioAgentsListRuns,
    #[serde(rename = "studio.agentsStart")]
    StudioAgentsStart {
        role: AgentRole,
        config: AgentProviderConfig,
        prompt: String,
    },
    #[serde(rename = "studio.agentsApprovePatch")]
    StudioAgentsApprovePatch { run_id: u64, patch_index: usize },
    #[serde(rename = "studio.qaInspectPairedTab")]
    StudioQaInspectPairedTab { window_id: WindowId },
    #[serde(rename = "studio.qaGetLastSnapshot")]
    StudioQaGetLastSnapshot,
    #[serde(rename = "studio.qaExecAction")]
    StudioQaExecAction {
        window_id: WindowId,
        action: String,
        selector: String,
        value: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data")]
pub enum BridgeResponse {
    Ok,
    WindowSnapshot(WindowSnapshot),
    WindowCreated {
        window_id: WindowId,
        tab_id: TabId,
    },
    TabCreated {
        tab_id: TabId,
        url: String,
    },
    TabNavigated {
        tab_id: TabId,
        url: String,
    },
    TabPinned {
        tab_id: TabId,
        pinned: bool,
    },
    GroupCreated {
        group_id: GroupId,
    },
    GroupCollapsed {
        group_id: GroupId,
        collapsed: bool,
    },
    Settings(CoreSettings),
    ThemesList(Vec<ThemeSummary>),
    ThemeActive(Box<LoadedTheme>),
    ImportProfiles(Vec<DetectedBrowserProfile>),
    ImportCompleted(ImportReport),
    DataSummary {
        bookmarks: Vec<BookmarkItem>,
        history: Vec<HistoryItem>,
    },
    MemoryStats(MemoryStats),
    TabsSlept {
        count: usize,
    },
    BlockerStats(BlockerStats),
    HistoryItems(Vec<HistoryItem>),
    BookmarkItems(Vec<BookmarkItem>),
    BookmarkToggled {
        bookmarked: bool,
    },
    BookmarksImported {
        count: usize,
    },
    DownloadItems(Vec<DownloadItem>),
    PermissionItems(Vec<SitePermissionItem>),
    SplitState {
        split_tab_id: Option<TabId>,
        ratio_pct: u8,
    },
    DevToolsState {
        open: bool,
        dock: DevToolsDock,
        ratio_pct: u8,
    },
    StudioWorkspace(Option<WorkspaceSummary>),
    StudioEntries(Vec<FileEntry>),
    StudioFile(FileContent),
    StudioSaved {
        bytes: usize,
    },
    StudioCreated {
        rel_path: String,
    },
    StudioSearchResults(Vec<SearchMatch>),
    StudioGit(GitSummary),
    StudioDiff {
        diff: String,
    },
    StudioTerminal(Option<TerminalSnapshot>),
    StudioClis(Vec<DetectedLocalCli>),
    StudioAgentsConfig {
        config_json: Option<String>,
    },
    StudioAgentRuns(Vec<AgentRunSnapshot>),
    StudioAgentRun(AgentRunSnapshot),
    StudioQaSnapshot(Option<BrowserQaSnapshot>),
}

/// Side-effect commands that the engine/window layer must execute after
/// `BrowserCore` state is updated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEffect {
    CreateNativeWindow {
        window_id: WindowId,
        incognito: bool,
        initial_tab_id: TabId,
        initial_url: String,
    },
    MinimizeWindow {
        window_id: WindowId,
    },
    MaximizeWindow {
        window_id: WindowId,
    },
    CloseWindow {
        window_id: WindowId,
    },
    StartWindowDrag {
        window_id: WindowId,
    },
    SetShellHeight {
        window_id: WindowId,
        height: u32,
    },
    CreateTabView {
        window_id: WindowId,
        tab_id: TabId,
        url: String,
    },
    ActivateTabView {
        window_id: WindowId,
        tab_id: TabId,
    },
    CloseTabView {
        window_id: WindowId,
        tab_id: TabId,
    },
    SleepTabView {
        window_id: WindowId,
        tab_id: TabId,
    },
    WakeTabView {
        window_id: WindowId,
        tab_id: TabId,
        url: String,
        scroll_x: i32,
        scroll_y: i32,
    },
    NavigateTabView {
        tab_id: TabId,
        url: String,
    },
    ReloadTabView {
        tab_id: TabId,
    },
    HardReloadTabView {
        tab_id: TabId,
    },
    GoBackTabView {
        tab_id: TabId,
    },
    GoForwardTabView {
        tab_id: TabId,
    },
    FocusOmnibox {
        window_id: WindowId,
    },
    ZoomTabView {
        tab_id: TabId,
        /// `+1` zoom in, `-1` zoom out, `0` reset to 100%.
        delta_steps: i32,
    },
    OpenOverlay {
        window_id: WindowId,
        panel: String,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    CloseOverlay {
        window_id: WindowId,
    },
    BroadcastState {
        window_id: WindowId,
    },
    BroadcastSettings,
    BroadcastTheme,
    BroadcastMemory,
    BroadcastBlocker,
    UpdateBlockerLists,
    RelayoutWindow {
        window_id: WindowId,
    },
    OpenDevTools {
        window_id: WindowId,
    },
    CloseDevTools {
        window_id: WindowId,
    },
    DownloadControl {
        id: i64,
        action: String,
    },
    ResolvePermissionPrompt {
        window_id: WindowId,
        prompt_id: u64,
        allow: bool,
        remember: bool,
    },
    BroadcastBookmarks,
    BroadcastDownloads,
    PersistSession,
    PickStudioFolder {
        window_id: WindowId,
    },
    StudioInspectPairedTab {
        window_id: WindowId,
    },
    StudioQaExecAction {
        window_id: WindowId,
        action: String,
        selector: String,
        value: Option<String>,
    },
}

pub struct DispatchOutput {
    pub response: BridgeResponse,
    pub effects: Vec<HostEffect>,
}

/// Parses, validates origin + size, mutates `BrowserCore`, and returns both the
/// JSON-serializable response and any native host side effects to run.
#[allow(clippy::too_many_arguments)]
pub fn handle_raw_request(
    origin_url: &str,
    raw_json: &str,
    core: &mut BrowserCore,
    themes: &ThemeRegistry,
    storage: Option<&Storage>,
    blocker: &mut BlockerService,
    studio: &mut StudioService,
    terminal_notify: Option<TerminalNotifyFn>,
    agent_notify: Option<AgentNotifyFn>,
) -> Result<DispatchOutput, BridgeError> {
    if !is_privileged_origin(origin_url) {
        return Err(BridgeError::OriginDenied);
    }
    if raw_json.len() > MAX_BRIDGE_PAYLOAD_BYTES {
        return Err(BridgeError::PayloadTooLarge);
    }

    let req: BridgeRequest =
        serde_json::from_str(raw_json).map_err(|e| BridgeError::InvalidJson(e.to_string()))?;
    dispatch_request(
        req,
        core,
        themes,
        storage,
        blocker,
        studio,
        terminal_notify,
        agent_notify,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn dispatch_request(
    req: BridgeRequest,
    core: &mut BrowserCore,
    themes: &ThemeRegistry,
    storage: Option<&Storage>,
    blocker: &mut BlockerService,
    studio: &mut StudioService,
    terminal_notify: Option<TerminalNotifyFn>,
    agent_notify: Option<AgentNotifyFn>,
) -> Result<DispatchOutput, BridgeError> {
    match req {
        BridgeRequest::WindowGetSnapshot { window_id } => {
            let snap = core.window_snapshot(window_id)?;
            Ok(DispatchOutput {
                response: BridgeResponse::WindowSnapshot(snap),
                effects: Vec::new(),
            })
        }
        BridgeRequest::WindowCreate { incognito, url } => {
            let (window_id, tab_id) = core.create_window(incognito, url.as_deref());
            let initial_url = core
                .get_tab(tab_id)
                .map(|t| t.url.clone())
                .unwrap_or_else(|| seu_core::NEWTAB_URL.to_string());
            Ok(DispatchOutput {
                response: BridgeResponse::WindowCreated { window_id, tab_id },
                effects: vec![HostEffect::CreateNativeWindow {
                    window_id,
                    incognito,
                    initial_tab_id: tab_id,
                    initial_url,
                }],
            })
        }
        BridgeRequest::WindowMinimize { window_id } => Ok(DispatchOutput {
            response: BridgeResponse::Ok,
            effects: vec![HostEffect::MinimizeWindow { window_id }],
        }),
        BridgeRequest::WindowMaximize { window_id } => Ok(DispatchOutput {
            response: BridgeResponse::Ok,
            effects: vec![HostEffect::MaximizeWindow { window_id }],
        }),
        BridgeRequest::WindowClose { window_id } => Ok(DispatchOutput {
            response: BridgeResponse::Ok,
            effects: vec![HostEffect::CloseWindow { window_id }],
        }),
        BridgeRequest::WindowStartDrag { window_id } => Ok(DispatchOutput {
            response: BridgeResponse::Ok,
            effects: vec![HostEffect::StartWindowDrag { window_id }],
        }),
        BridgeRequest::WindowSetShellBounds { window_id, height } => {
            let clamped = height.clamp(32, 640);
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects: vec![HostEffect::SetShellHeight {
                    window_id,
                    height: clamped,
                }],
            })
        }
        BridgeRequest::TabCreate {
            window_id,
            url,
            activate,
        } => {
            let should_activate = activate.unwrap_or(true);
            let tab_id = core.create_tab(window_id, url.as_deref(), should_activate)?;
            let resolved_url = core
                .get_tab(tab_id)
                .map(|t| t.url.clone())
                .unwrap_or_else(|| seu_core::NEWTAB_URL.to_string());
            let mut effects = vec![HostEffect::CreateTabView {
                window_id,
                tab_id,
                url: resolved_url.clone(),
            }];
            if should_activate {
                effects.push(HostEffect::ActivateTabView { window_id, tab_id });
            }
            effects.push(HostEffect::BroadcastState { window_id });
            Ok(DispatchOutput {
                response: BridgeResponse::TabCreated {
                    tab_id,
                    url: resolved_url,
                },
                effects,
            })
        }
        BridgeRequest::TabClose { tab_id } => {
            let (window_id, next_active) = core.close_tab(tab_id)?;
            let mut effects = vec![HostEffect::CloseTabView { window_id, tab_id }];
            if let Some(next_id) = next_active {
                effects.push(HostEffect::ActivateTabView {
                    window_id,
                    tab_id: next_id,
                });
                effects.push(HostEffect::BroadcastState { window_id });
            } else {
                effects.push(HostEffect::CloseWindow { window_id });
            }
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects,
            })
        }
        BridgeRequest::TabActivate { window_id, tab_id } => {
            let was_sleeping = core
                .get_tab(tab_id)
                .map(|t| t.state == TabLifecycleState::Sleeping)
                .unwrap_or(false);
            let wake_info = if was_sleeping {
                let url = core
                    .get_tab(tab_id)
                    .map(|t| t.url.clone())
                    .unwrap_or_else(|| seu_core::NEWTAB_URL.to_string());
                let (sx, sy) = core
                    .tab_memory_meta(tab_id)
                    .map(|m| (m.scroll_x, m.scroll_y))
                    .unwrap_or((0, 0));
                Some((url, sx, sy))
            } else {
                None
            };
            core.activate_tab(window_id, tab_id)?;
            let mut effects = Vec::new();
            if let Some((url, scroll_x, scroll_y)) = wake_info {
                effects.push(HostEffect::WakeTabView {
                    window_id,
                    tab_id,
                    url,
                    scroll_x,
                    scroll_y,
                });
            }
            effects.push(HostEffect::ActivateTabView { window_id, tab_id });
            effects.push(HostEffect::BroadcastState { window_id });
            effects.push(HostEffect::BroadcastMemory);
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects,
            })
        }
        BridgeRequest::TabNavigate { tab_id, input } => {
            let url = core.navigate_tab(tab_id, &input)?;
            let window_id = core
                .get_tab(tab_id)
                .map(|t| t.window_id)
                .ok_or(CoreError::TabNotFound(tab_id))?;
            Ok(DispatchOutput {
                response: BridgeResponse::TabNavigated {
                    tab_id,
                    url: url.clone(),
                },
                effects: vec![
                    HostEffect::NavigateTabView { tab_id, url },
                    HostEffect::BroadcastState { window_id },
                ],
            })
        }
        BridgeRequest::TabReload { tab_id } => {
            if core.get_tab(tab_id).is_none() {
                return Err(CoreError::TabNotFound(tab_id).into());
            }
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects: vec![HostEffect::ReloadTabView { tab_id }],
            })
        }
        BridgeRequest::TabBack { tab_id } => {
            if core.get_tab(tab_id).is_none() {
                return Err(CoreError::TabNotFound(tab_id).into());
            }
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects: vec![HostEffect::GoBackTabView { tab_id }],
            })
        }
        BridgeRequest::TabForward { tab_id } => {
            if core.get_tab(tab_id).is_none() {
                return Err(CoreError::TabNotFound(tab_id).into());
            }
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects: vec![HostEffect::GoForwardTabView { tab_id }],
            })
        }
        BridgeRequest::TabPin { tab_id } => {
            let pinned = core.toggle_pin_tab(tab_id)?;
            let window_id = core
                .get_tab(tab_id)
                .map(|t| t.window_id)
                .ok_or(CoreError::TabNotFound(tab_id))?;
            Ok(DispatchOutput {
                response: BridgeResponse::TabPinned { tab_id, pinned },
                effects: vec![HostEffect::BroadcastState { window_id }],
            })
        }
        BridgeRequest::TabDuplicate { tab_id } => {
            let new_tab_id = core.duplicate_tab(tab_id)?;
            let tab = core
                .get_tab(new_tab_id)
                .cloned()
                .ok_or(CoreError::TabNotFound(new_tab_id))?;
            Ok(DispatchOutput {
                response: BridgeResponse::TabCreated {
                    tab_id: new_tab_id,
                    url: tab.url.clone(),
                },
                effects: vec![
                    HostEffect::CreateTabView {
                        window_id: tab.window_id,
                        tab_id: new_tab_id,
                        url: tab.url,
                    },
                    HostEffect::ActivateTabView {
                        window_id: tab.window_id,
                        tab_id: new_tab_id,
                    },
                    HostEffect::BroadcastState {
                        window_id: tab.window_id,
                    },
                ],
            })
        }
        BridgeRequest::TabReopenClosed { window_id } => {
            let Some(tab_id) = core.reopen_closed_tab(window_id)? else {
                return Ok(DispatchOutput {
                    response: BridgeResponse::Ok,
                    effects: Vec::new(),
                });
            };
            let url = core
                .get_tab(tab_id)
                .map(|t| t.url.clone())
                .unwrap_or_else(|| seu_core::NEWTAB_URL.to_string());
            Ok(DispatchOutput {
                response: BridgeResponse::TabCreated {
                    tab_id,
                    url: url.clone(),
                },
                effects: vec![
                    HostEffect::CreateTabView {
                        window_id,
                        tab_id,
                        url,
                    },
                    HostEffect::ActivateTabView { window_id, tab_id },
                    HostEffect::BroadcastState { window_id },
                ],
            })
        }
        BridgeRequest::TabReorder {
            window_id,
            tab_id,
            to_index,
        } => {
            core.reorder_tab(window_id, tab_id, to_index)?;
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects: vec![HostEffect::BroadcastState { window_id }],
            })
        }
        BridgeRequest::TabDetachToWindow { tab_id } => {
            let url = core
                .get_tab(tab_id)
                .ok_or(CoreError::TabNotFound(tab_id))?
                .url
                .clone();
            let (source_window_id, next_active) = core.close_tab(tab_id)?;
            let (new_window_id, new_tab_id) = core.create_window(false, Some(&url));
            let mut effects = vec![
                HostEffect::CloseTabView {
                    window_id: source_window_id,
                    tab_id,
                },
                HostEffect::CreateNativeWindow {
                    window_id: new_window_id,
                    incognito: false,
                    initial_tab_id: new_tab_id,
                    initial_url: url,
                },
            ];
            if let Some(next_id) = next_active {
                effects.push(HostEffect::ActivateTabView {
                    window_id: source_window_id,
                    tab_id: next_id,
                });
                effects.push(HostEffect::BroadcastState {
                    window_id: source_window_id,
                });
            } else {
                effects.push(HostEffect::CloseWindow {
                    window_id: source_window_id,
                });
            }
            Ok(DispatchOutput {
                response: BridgeResponse::WindowCreated {
                    window_id: new_window_id,
                    tab_id: new_tab_id,
                },
                effects,
            })
        }
        BridgeRequest::TabGroupCreate {
            window_id,
            title,
            color,
            tab_ids,
        } => {
            if title.trim().is_empty() || title.len() > 64 {
                return Err(BridgeError::InvalidArgument(
                    "group title must be 1..=64 chars".to_string(),
                ));
            }
            let group_id =
                core.create_tab_group(window_id, title.trim().to_string(), color, &tab_ids)?;
            Ok(DispatchOutput {
                response: BridgeResponse::GroupCreated { group_id },
                effects: vec![HostEffect::BroadcastState { window_id }],
            })
        }
        BridgeRequest::TabGroupToggleCollapse {
            window_id,
            group_id,
        } => {
            let collapsed = core.toggle_group_collapse(window_id, group_id)?;
            Ok(DispatchOutput {
                response: BridgeResponse::GroupCollapsed {
                    group_id,
                    collapsed,
                },
                effects: vec![HostEffect::BroadcastState { window_id }],
            })
        }
        BridgeRequest::TabGroupAssignTab { tab_id, group_id } => {
            let window_id = core.assign_tab_to_group(tab_id, group_id)?;
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects: vec![HostEffect::BroadcastState { window_id }],
            })
        }
        BridgeRequest::TabGroupUngroup {
            window_id,
            group_id,
        } => {
            core.ungroup(window_id, group_id)?;
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects: vec![HostEffect::BroadcastState { window_id }],
            })
        }
        BridgeRequest::OverlayOpen {
            window_id,
            panel,
            x,
            y,
            width,
            height,
        } => Ok(DispatchOutput {
            response: BridgeResponse::Ok,
            effects: vec![HostEffect::OpenOverlay {
                window_id,
                panel,
                x,
                y,
                width: width.clamp(120, 960),
                height: height.clamp(60, 720),
            }],
        }),
        BridgeRequest::OverlayClose { window_id } => Ok(DispatchOutput {
            response: BridgeResponse::Ok,
            effects: vec![HostEffect::CloseOverlay { window_id }],
        }),
        BridgeRequest::SettingsGet => Ok(DispatchOutput {
            response: BridgeResponse::Settings(core.settings.clone()),
            effects: Vec::new(),
        }),
        BridgeRequest::SettingsSetLocale { locale } => {
            let parsed = Locale::from_code(&locale).ok_or_else(|| {
                BridgeError::InvalidArgument(format!("unsupported locale: {locale}"))
            })?;
            core.settings.locale = parsed;
            Ok(DispatchOutput {
                response: BridgeResponse::Settings(core.settings.clone()),
                effects: vec![HostEffect::BroadcastSettings],
            })
        }
        BridgeRequest::SettingsSetSearchEngine { engine } => {
            core.settings.search_engine = engine;
            Ok(DispatchOutput {
                response: BridgeResponse::Settings(core.settings.clone()),
                effects: vec![HostEffect::BroadcastSettings],
            })
        }
        BridgeRequest::SearchSubmit {
            tab_id,
            query,
            category,
        } => {
            let url = core.navigate_tab_category_search(tab_id, &query, category)?;
            let window_id = core
                .get_tab(tab_id)
                .map(|t| t.window_id)
                .ok_or(CoreError::TabNotFound(tab_id))?;
            Ok(DispatchOutput {
                response: BridgeResponse::TabNavigated {
                    tab_id,
                    url: url.clone(),
                },
                effects: vec![
                    HostEffect::NavigateTabView { tab_id, url },
                    HostEffect::BroadcastState { window_id },
                ],
            })
        }
        BridgeRequest::GameModeToggle => {
            core.toggle_game_mode();
            Ok(DispatchOutput {
                response: BridgeResponse::Settings(core.settings.clone()),
                effects: vec![HostEffect::BroadcastSettings],
            })
        }
        BridgeRequest::ThemesList => Ok(DispatchOutput {
            response: BridgeResponse::ThemesList(themes.list_summaries()),
            effects: Vec::new(),
        }),
        BridgeRequest::ThemesGetActive => {
            let active = themes
                .get(&core.settings.active_theme_id)
                .cloned()
                .ok_or_else(|| {
                    BridgeError::InvalidArgument(format!(
                        "theme not found: {}",
                        core.settings.active_theme_id
                    ))
                })?;
            Ok(DispatchOutput {
                response: BridgeResponse::ThemeActive(Box::new(active)),
                effects: Vec::new(),
            })
        }
        BridgeRequest::ThemesSetActive { id } => {
            if !themes.exists(&id) {
                return Err(BridgeError::InvalidArgument(format!(
                    "unknown theme id: {id}"
                )));
            }
            core.settings.active_theme_id = id.clone();
            let active = themes
                .get(&id)
                .cloned()
                .ok_or_else(|| BridgeError::InvalidArgument(format!("unknown theme id: {id}")))?;
            Ok(DispatchOutput {
                response: BridgeResponse::ThemeActive(Box::new(active)),
                effects: vec![HostEffect::BroadcastTheme, HostEffect::BroadcastSettings],
            })
        }
        BridgeRequest::ShortcutsSet { action, combo } => {
            core.set_shortcut(&action, &combo).map_err(|_| {
                BridgeError::InvalidArgument(format!("invalid shortcut '{combo}' for '{action}'"))
            })?;
            Ok(DispatchOutput {
                response: BridgeResponse::Settings(core.settings.clone()),
                effects: vec![HostEffect::BroadcastSettings],
            })
        }
        BridgeRequest::ShortcutsReset => {
            core.reset_shortcuts();
            Ok(DispatchOutput {
                response: BridgeResponse::Settings(core.settings.clone()),
                effects: vec![HostEffect::BroadcastSettings],
            })
        }
        BridgeRequest::QuickShortcutsSet { items } => {
            let cleaned: Vec<QuickShortcut> = items
                .into_iter()
                .filter(|item| {
                    let u = item.url.trim();
                    (u.starts_with("https://") || u.starts_with("http://"))
                        && !item.name.trim().is_empty()
                })
                .take(12)
                .collect();
            core.settings.quick_shortcuts = cleaned;
            Ok(DispatchOutput {
                response: BridgeResponse::Settings(core.settings.clone()),
                effects: vec![HostEffect::BroadcastSettings],
            })
        }
        BridgeRequest::ImportListProfiles => Ok(DispatchOutput {
            response: BridgeResponse::ImportProfiles(detect_browser_profiles()),
            effects: Vec::new(),
        }),
        BridgeRequest::ImportRun(options) => {
            let Some(db) = storage else {
                return Err(BridgeError::InvalidArgument(
                    "local database unavailable".into(),
                ));
            };
            let profiles = detect_browser_profiles();
            let Some(profile) = profiles.into_iter().find(|p| p.id == options.profile_id) else {
                return Err(BridgeError::InvalidArgument(format!(
                    "browser profile not found: {}",
                    options.profile_id
                )));
            };
            let report = run_profile_import(db, &profile, &options)
                .map_err(|e| BridgeError::InvalidArgument(format!("import failed: {e}")))?;

            let mut effects = Vec::new();
            if options.import_quick_shortcuts && !report.quick_shortcuts.is_empty() {
                core.settings.quick_shortcuts = report
                    .quick_shortcuts
                    .iter()
                    .map(|s| QuickShortcut {
                        id: s.id.clone(),
                        code: s.code.clone(),
                        name: s.name.clone(),
                        url: s.url.clone(),
                    })
                    .collect();
                effects.push(HostEffect::BroadcastSettings);
            }
            Ok(DispatchOutput {
                response: BridgeResponse::ImportCompleted(report),
                effects,
            })
        }
        BridgeRequest::DataSummary => {
            let (bookmarks, history) = match storage {
                Some(db) => (
                    db.list_bookmarks(80).unwrap_or_default(),
                    db.list_history(80).unwrap_or_default(),
                ),
                None => (Vec::new(), Vec::new()),
            };
            Ok(DispatchOutput {
                response: BridgeResponse::DataSummary { bookmarks, history },
                effects: Vec::new(),
            })
        }
        BridgeRequest::MemoryGetStats => Ok(DispatchOutput {
            response: BridgeResponse::MemoryStats(core.memory_stats.clone()),
            effects: Vec::new(),
        }),
        BridgeRequest::MemorySleepTab { tab_id } => {
            let window_id = core.mark_tab_sleeping(tab_id, true)?;
            Ok(DispatchOutput {
                response: BridgeResponse::TabsSlept { count: 1 },
                effects: vec![
                    HostEffect::SleepTabView { window_id, tab_id },
                    HostEffect::BroadcastState { window_id },
                    HostEffect::BroadcastMemory,
                ],
            })
        }
        BridgeRequest::MemorySleepInactive => {
            let candidates = core.select_due_sleep_tabs(u64::MAX, true);
            let mut effects = Vec::new();
            let mut touched_windows = Vec::new();
            let mut slept = 0usize;
            for tid in candidates {
                if let Ok(wid) = core.mark_tab_sleeping(tid, false) {
                    slept += 1;
                    effects.push(HostEffect::SleepTabView {
                        window_id: wid,
                        tab_id: tid,
                    });
                    if !touched_windows.contains(&wid) {
                        touched_windows.push(wid);
                    }
                }
            }
            for wid in touched_windows {
                effects.push(HostEffect::BroadcastState { window_id: wid });
            }
            if slept > 0 {
                effects.push(HostEffect::BroadcastMemory);
            }
            Ok(DispatchOutput {
                response: BridgeResponse::TabsSlept { count: slept },
                effects,
            })
        }
        BridgeRequest::MemorySetPolicy { mut policy } => {
            policy.sleep_timeout_secs = policy.sleep_timeout_secs.min(24 * 3600);
            if let Some(cap) = policy.max_live_tabs {
                policy.max_live_tabs = Some(cap.clamp(1, 200));
            }
            policy.never_sleep_domains = policy
                .never_sleep_domains
                .into_iter()
                .map(|d| d.trim().to_ascii_lowercase())
                .filter(|d| !d.is_empty())
                .take(100)
                .collect();
            core.settings.memory_policy = policy;
            Ok(DispatchOutput {
                response: BridgeResponse::Settings(core.settings.clone()),
                effects: vec![HostEffect::BroadcastSettings],
            })
        }
        BridgeRequest::BlockerGetStats => Ok(DispatchOutput {
            response: BridgeResponse::BlockerStats(blocker.stats()),
            effects: Vec::new(),
        }),
        BridgeRequest::BlockerSetEnabled { enabled } => {
            blocker.set_enabled(enabled);
            core.settings.blocker_enabled = enabled;
            Ok(DispatchOutput {
                response: BridgeResponse::BlockerStats(blocker.stats()),
                effects: vec![HostEffect::BroadcastBlocker, HostEffect::BroadcastSettings],
            })
        }
        BridgeRequest::BlockerToggleSite { domain } => {
            let _ = blocker.toggle_for_site(&domain);
            core.settings.blocker_allowlist = blocker.allowlisted_domains();
            Ok(DispatchOutput {
                response: BridgeResponse::BlockerStats(blocker.stats()),
                effects: vec![HostEffect::BroadcastBlocker, HostEffect::BroadcastSettings],
            })
        }
        BridgeRequest::BlockerUpdateLists => Ok(DispatchOutput {
            response: BridgeResponse::BlockerStats(blocker.stats()),
            effects: vec![HostEffect::UpdateBlockerLists],
        }),
        BridgeRequest::HistoryList { query, limit } => {
            let max = limit.unwrap_or(150).clamp(1, 500);
            let items = match storage {
                Some(db) => {
                    if let Some(q) = query.as_deref() {
                        db.search_history(q, max).unwrap_or_default()
                    } else {
                        db.list_history(max).unwrap_or_default()
                    }
                }
                None => Vec::new(),
            };
            Ok(DispatchOutput {
                response: BridgeResponse::HistoryItems(items),
                effects: Vec::new(),
            })
        }
        BridgeRequest::HistoryDelete { id } => {
            if let Some(db) = storage {
                let _ = db.delete_history_item(id);
            }
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects: Vec::new(),
            })
        }
        BridgeRequest::HistoryClear => {
            if let Some(db) = storage {
                let _ = db.clear_history();
            }
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects: Vec::new(),
            })
        }
        BridgeRequest::BookmarksList => {
            let items = storage
                .map(|db| db.list_bookmarks_all(500).unwrap_or_default())
                .unwrap_or_default();
            Ok(DispatchOutput {
                response: BridgeResponse::BookmarkItems(items),
                effects: Vec::new(),
            })
        }
        BridgeRequest::BookmarksIsBookmarked { url } => {
            let bookmarked = storage
                .and_then(|db| db.is_url_bookmarked(&url).ok())
                .unwrap_or(false);
            Ok(DispatchOutput {
                response: BridgeResponse::BookmarkToggled { bookmarked },
                effects: Vec::new(),
            })
        }
        BridgeRequest::BookmarksToggle {
            title,
            url,
            parent_id,
        } => {
            let Some(db) = storage else {
                return Err(BridgeError::InvalidArgument(
                    "local database unavailable".into(),
                ));
            };
            if url.trim().is_empty() || url.trim().starts_with("seu://") {
                return Err(BridgeError::InvalidArgument(
                    "cannot bookmark empty or internal seu:// URL".into(),
                ));
            }
            let bookmarked = db
                .toggle_bookmark(&title, &url, parent_id)
                .map_err(|e| BridgeError::InvalidArgument(e.to_string()))?;
            Ok(DispatchOutput {
                response: BridgeResponse::BookmarkToggled { bookmarked },
                effects: vec![HostEffect::BroadcastBookmarks],
            })
        }
        BridgeRequest::BookmarksCreateFolder { title, parent_id } => {
            let Some(db) = storage else {
                return Err(BridgeError::InvalidArgument(
                    "local database unavailable".into(),
                ));
            };
            if title.trim().is_empty() || title.len() > 80 {
                return Err(BridgeError::InvalidArgument(
                    "folder title must be 1..=80 chars".into(),
                ));
            }
            db.create_bookmark_folder(&title, parent_id)
                .map_err(|e| BridgeError::InvalidArgument(e.to_string()))?;
            let items = db.list_bookmarks_all(500).unwrap_or_default();
            Ok(DispatchOutput {
                response: BridgeResponse::BookmarkItems(items),
                effects: vec![HostEffect::BroadcastBookmarks],
            })
        }
        BridgeRequest::BookmarksDelete { id } => {
            if let Some(db) = storage {
                let _ = db.delete_bookmark(id);
            }
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects: vec![HostEffect::BroadcastBookmarks],
            })
        }
        BridgeRequest::BookmarksImportNetscape { html } => {
            let Some(db) = storage else {
                return Err(BridgeError::InvalidArgument(
                    "local database unavailable".into(),
                ));
            };
            let count = db
                .import_netscape_html(&html)
                .map_err(|e| BridgeError::InvalidArgument(e.to_string()))?;
            Ok(DispatchOutput {
                response: BridgeResponse::BookmarksImported { count },
                effects: vec![HostEffect::BroadcastBookmarks],
            })
        }
        BridgeRequest::DownloadsList => {
            let items = storage
                .map(|db| db.list_downloads(120).unwrap_or_default())
                .unwrap_or_default();
            Ok(DispatchOutput {
                response: BridgeResponse::DownloadItems(items),
                effects: Vec::new(),
            })
        }
        BridgeRequest::DownloadsAction { id, action } => {
            if action == "remove" {
                if let Some(db) = storage {
                    let _ = db.delete_download(id);
                }
                return Ok(DispatchOutput {
                    response: BridgeResponse::Ok,
                    effects: vec![HostEffect::BroadcastDownloads],
                });
            }
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects: vec![HostEffect::DownloadControl { id, action }],
            })
        }
        BridgeRequest::DownloadsClearFinished => {
            if let Some(db) = storage {
                let _ = db.clear_finished_downloads();
            }
            Ok(DispatchOutput {
                response: BridgeResponse::Ok,
                effects: vec![HostEffect::BroadcastDownloads],
            })
        }
        BridgeRequest::PermissionsList => {
            let items = storage
                .map(|db| db.list_permissions().unwrap_or_default())
                .unwrap_or_default();
            Ok(DispatchOutput {
                response: BridgeResponse::PermissionItems(items),
                effects: Vec::new(),
            })
        }
        BridgeRequest::PermissionsSet {
            origin,
            permission,
            decision,
        } => {
            if decision != "allow" && decision != "deny" {
                return Err(BridgeError::InvalidArgument(
                    "decision must be 'allow' or 'deny'".into(),
                ));
            }
            if let Some(db) = storage {
                let _ = db.set_permission(&origin, &permission, &decision);
            }
            let items = storage
                .map(|db| db.list_permissions().unwrap_or_default())
                .unwrap_or_default();
            Ok(DispatchOutput {
                response: BridgeResponse::PermissionItems(items),
                effects: Vec::new(),
            })
        }
        BridgeRequest::PermissionsDelete { origin, permission } => {
            if let Some(db) = storage {
                let _ = db.delete_permission(&origin, &permission);
            }
            let items = storage
                .map(|db| db.list_permissions().unwrap_or_default())
                .unwrap_or_default();
            Ok(DispatchOutput {
                response: BridgeResponse::PermissionItems(items),
                effects: Vec::new(),
            })
        }
        BridgeRequest::PermissionsRespondPrompt {
            window_id,
            prompt_id,
            allow,
            remember,
        } => Ok(DispatchOutput {
            response: BridgeResponse::Ok,
            effects: vec![
                HostEffect::ResolvePermissionPrompt {
                    window_id,
                    prompt_id,
                    allow,
                    remember,
                },
                HostEffect::CloseOverlay { window_id },
            ],
        }),
        BridgeRequest::SettingsSetRestoreSession { enabled } => {
            core.settings.restore_session_on_startup = enabled;
            Ok(DispatchOutput {
                response: BridgeResponse::Settings(core.settings.clone()),
                effects: vec![HostEffect::BroadcastSettings],
            })
        }
        BridgeRequest::SplitToggle { window_id, tab_id } => {
            let (paired, created_new) = core.toggle_split_view(window_id, tab_id)?;
            let mut effects = Vec::new();
            if let Some(pid) = paired {
                let url = core
                    .get_tab(pid)
                    .map(|t| t.url.clone())
                    .unwrap_or_else(|| seu_core::NEWTAB_URL.to_string());
                if created_new {
                    effects.push(HostEffect::CreateTabView {
                        window_id,
                        tab_id: pid,
                        url,
                    });
                } else {
                    let (sx, sy) = core
                        .tab_memory_meta(pid)
                        .map(|m| (m.scroll_x, m.scroll_y))
                        .unwrap_or((0, 0));
                    effects.push(HostEffect::WakeTabView {
                        window_id,
                        tab_id: pid,
                        url,
                        scroll_x: sx,
                        scroll_y: sy,
                    });
                }
            }
            effects.push(HostEffect::RelayoutWindow { window_id });
            effects.push(HostEffect::BroadcastState { window_id });
            effects.push(HostEffect::BroadcastMemory);
            let snap = core.window_snapshot(window_id)?;
            Ok(DispatchOutput {
                response: BridgeResponse::SplitState {
                    split_tab_id: snap.split_tab_id,
                    ratio_pct: snap.split_ratio_pct,
                },
                effects,
            })
        }
        BridgeRequest::SplitClose { window_id } => {
            core.close_split_view(window_id)?;
            let snap = core.window_snapshot(window_id)?;
            Ok(DispatchOutput {
                response: BridgeResponse::SplitState {
                    split_tab_id: snap.split_tab_id,
                    ratio_pct: snap.split_ratio_pct,
                },
                effects: vec![
                    HostEffect::RelayoutWindow { window_id },
                    HostEffect::BroadcastState { window_id },
                ],
            })
        }
        BridgeRequest::SplitSetRatio {
            window_id,
            ratio_pct,
        } => {
            let clamped = core.set_split_ratio(window_id, ratio_pct)?;
            let snap = core.window_snapshot(window_id)?;
            Ok(DispatchOutput {
                response: BridgeResponse::SplitState {
                    split_tab_id: snap.split_tab_id,
                    ratio_pct: clamped,
                },
                effects: vec![
                    HostEffect::RelayoutWindow { window_id },
                    HostEffect::BroadcastState { window_id },
                ],
            })
        }
        BridgeRequest::DevToolsToggle { window_id } => {
            let open = core.toggle_devtools(window_id)?;
            let snap = core.window_snapshot(window_id)?;
            let effects = if open {
                vec![
                    HostEffect::OpenDevTools { window_id },
                    HostEffect::RelayoutWindow { window_id },
                    HostEffect::BroadcastState { window_id },
                ]
            } else {
                vec![
                    HostEffect::CloseDevTools { window_id },
                    HostEffect::RelayoutWindow { window_id },
                    HostEffect::BroadcastState { window_id },
                ]
            };
            Ok(DispatchOutput {
                response: BridgeResponse::DevToolsState {
                    open,
                    dock: snap.devtools_dock,
                    ratio_pct: snap.devtools_ratio_pct,
                },
                effects,
            })
        }
        BridgeRequest::DevToolsClose { window_id } => {
            core.close_devtools(window_id)?;
            let snap = core.window_snapshot(window_id)?;
            Ok(DispatchOutput {
                response: BridgeResponse::DevToolsState {
                    open: false,
                    dock: snap.devtools_dock,
                    ratio_pct: snap.devtools_ratio_pct,
                },
                effects: vec![
                    HostEffect::CloseDevTools { window_id },
                    HostEffect::RelayoutWindow { window_id },
                    HostEffect::BroadcastState { window_id },
                ],
            })
        }
        BridgeRequest::DevToolsSetDock { window_id, dock } => {
            core.set_devtools_dock(window_id, dock)?;
            let snap = core.window_snapshot(window_id)?;
            Ok(DispatchOutput {
                response: BridgeResponse::DevToolsState {
                    open: snap.devtools_open,
                    dock,
                    ratio_pct: snap.devtools_ratio_pct,
                },
                effects: vec![
                    HostEffect::RelayoutWindow { window_id },
                    HostEffect::BroadcastState { window_id },
                ],
            })
        }
        BridgeRequest::DevToolsSetRatio {
            window_id,
            ratio_pct,
        } => {
            let clamped = core.set_devtools_ratio(window_id, ratio_pct)?;
            let snap = core.window_snapshot(window_id)?;
            Ok(DispatchOutput {
                response: BridgeResponse::DevToolsState {
                    open: snap.devtools_open,
                    dock: snap.devtools_dock,
                    ratio_pct: clamped,
                },
                effects: vec![
                    HostEffect::RelayoutWindow { window_id },
                    HostEffect::BroadcastState { window_id },
                ],
            })
        }
        BridgeRequest::StudioSetMode { window_id, mode } => {
            let effects = apply_studio_mode(core, window_id, &mode)?;
            let snap = core.window_snapshot(window_id)?;
            Ok(DispatchOutput {
                response: BridgeResponse::WindowSnapshot(snap),
                effects,
            })
        }
        BridgeRequest::StudioPickFolder { window_id } => Ok(DispatchOutput {
            response: BridgeResponse::Ok,
            effects: vec![HostEffect::PickStudioFolder { window_id }],
        }),
        BridgeRequest::StudioOpenWorkspace { path } => {
            let summary = studio
                .open_workspace(&path)
                .map_err(|e| BridgeError::InvalidArgument(e.to_string()))?;
            core.settings.studio_last_workspace = Some(summary.root_path.clone());
            Ok(DispatchOutput {
                response: BridgeResponse::StudioWorkspace(Some(summary)),
                effects: vec![HostEffect::BroadcastSettings],
            })
        }
        BridgeRequest::StudioGetWorkspace => {
            if studio.workspace_summary().is_err() {
                if let Some(last) = core.settings.studio_last_workspace.clone() {
                    let _ = studio.open_workspace(&last);
                }
            }
            let summary = studio.workspace_summary().ok();
            Ok(DispatchOutput {
                response: BridgeResponse::StudioWorkspace(summary),
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioListDir { rel_path } => {
            let entries = studio
                .list_dir(&rel_path)
                .map_err(|e| BridgeError::InvalidArgument(e.to_string()))?;
            Ok(DispatchOutput {
                response: BridgeResponse::StudioEntries(entries),
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioReadFile { rel_path } => {
            let file = studio
                .read_file(&rel_path)
                .map_err(|e| BridgeError::InvalidArgument(e.to_string()))?;
            Ok(DispatchOutput {
                response: BridgeResponse::StudioFile(file),
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioWriteFile { rel_path, content } => {
            let bytes = studio
                .write_file(&rel_path, &content)
                .map_err(|e| BridgeError::InvalidArgument(e.to_string()))?;
            Ok(DispatchOutput {
                response: BridgeResponse::StudioSaved { bytes },
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioCreateEntry { rel_path, is_dir } => {
            let created = studio
                .create_entry(&rel_path, is_dir)
                .map_err(|e| BridgeError::InvalidArgument(e.to_string()))?;
            Ok(DispatchOutput {
                response: BridgeResponse::StudioCreated { rel_path: created },
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioSearch { query } => {
            let results = studio
                .search(&query)
                .map_err(|e| BridgeError::InvalidArgument(e.to_string()))?;
            Ok(DispatchOutput {
                response: BridgeResponse::StudioSearchResults(results),
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioGitStatus => {
            let git = studio
                .git_status()
                .map_err(|e| BridgeError::InvalidArgument(e.to_string()))?;
            Ok(DispatchOutput {
                response: BridgeResponse::StudioGit(git),
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioGitDiff { rel_path } => {
            let diff = studio
                .git_diff(rel_path.as_deref())
                .map_err(|e| BridgeError::InvalidArgument(e.to_string()))?;
            Ok(DispatchOutput {
                response: BridgeResponse::StudioDiff { diff },
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioRunCommand { command } => {
            let snap = studio
                .run_command(&command, terminal_notify)
                .map_err(BridgeError::InvalidArgument)?;
            Ok(DispatchOutput {
                response: BridgeResponse::StudioTerminal(Some(snap)),
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioKillCommand { session_id } => {
            let snap = studio.kill_command(session_id);
            Ok(DispatchOutput {
                response: BridgeResponse::StudioTerminal(snap),
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioGetTerminal { session_id } => {
            let snap = studio.get_terminal(session_id);
            Ok(DispatchOutput {
                response: BridgeResponse::StudioTerminal(snap),
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioDetectClis => Ok(DispatchOutput {
            response: BridgeResponse::StudioClis(studio.detect_local_clis()),
            effects: Vec::new(),
        }),
        BridgeRequest::StudioAgentsConfigGet => {
            let config_json =
                storage.and_then(|db| db.get_setting("studio_agent_config_v1").ok().flatten());
            Ok(DispatchOutput {
                response: BridgeResponse::StudioAgentsConfig { config_json },
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioAgentsConfigSave { config_json } => {
            if let Some(db) = storage {
                let _ = db.set_setting("studio_agent_config_v1", &config_json);
            }
            Ok(DispatchOutput {
                response: BridgeResponse::StudioAgentsConfig {
                    config_json: Some(config_json),
                },
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioAgentsListRuns => Ok(DispatchOutput {
            response: BridgeResponse::StudioAgentRuns(studio.list_agent_runs()),
            effects: Vec::new(),
        }),
        BridgeRequest::StudioAgentsStart {
            role,
            config,
            prompt,
        } => {
            let run = studio
                .start_agent_run(role, config, prompt, agent_notify)
                .map_err(BridgeError::InvalidArgument)?;
            Ok(DispatchOutput {
                response: BridgeResponse::StudioAgentRun(run),
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioAgentsApprovePatch {
            run_id,
            patch_index,
        } => {
            let run = studio
                .approve_agent_patch(run_id, patch_index)
                .map_err(BridgeError::InvalidArgument)?;
            Ok(DispatchOutput {
                response: BridgeResponse::StudioAgentRun(run),
                effects: Vec::new(),
            })
        }
        BridgeRequest::StudioQaInspectPairedTab { window_id } => Ok(DispatchOutput {
            response: BridgeResponse::Ok,
            effects: vec![HostEffect::StudioInspectPairedTab { window_id }],
        }),
        BridgeRequest::StudioQaGetLastSnapshot => Ok(DispatchOutput {
            response: BridgeResponse::StudioQaSnapshot(studio.last_qa_snapshot()),
            effects: Vec::new(),
        }),
        BridgeRequest::StudioQaExecAction {
            window_id,
            action,
            selector,
            value,
        } => Ok(DispatchOutput {
            response: BridgeResponse::Ok,
            effects: vec![HostEffect::StudioQaExecAction {
                window_id,
                action,
                selector,
                value,
            }],
        }),
    }
}

/// Applies the 3-way Browser / Split / Studio mode transition cleanly on `BrowserCore`.
pub fn apply_studio_mode(
    core: &mut BrowserCore,
    window_id: WindowId,
    mode: &str,
) -> Result<Vec<HostEffect>, BridgeError> {
    let snap = core.window_snapshot(window_id)?;
    let existing_studio = snap
        .tabs
        .iter()
        .find(|t| t.url.starts_with(seu_core::STUDIO_URL))
        .map(|t| t.id);

    let mut effects = Vec::new();
    match mode {
        "browser" => {
            let _ = core.close_split_view(window_id);
            let active_is_studio =
                snap.active_tab_id == existing_studio && existing_studio.is_some();
            if active_is_studio {
                let web_tab = snap
                    .tabs
                    .iter()
                    .find(|t| Some(t.id) != existing_studio)
                    .map(|t| t.id);
                let target_id = if let Some(wid_tab) = web_tab {
                    let was_sleeping = core
                        .get_tab(wid_tab)
                        .map(|t| t.state == TabLifecycleState::Sleeping)
                        .unwrap_or(false);
                    core.activate_tab(window_id, wid_tab)?;
                    if was_sleeping {
                        let url = core
                            .get_tab(wid_tab)
                            .map(|t| t.url.clone())
                            .unwrap_or_else(|| seu_core::NEWTAB_URL.to_string());
                        effects.push(HostEffect::WakeTabView {
                            window_id,
                            tab_id: wid_tab,
                            url,
                            scroll_x: 0,
                            scroll_y: 0,
                        });
                    }
                    wid_tab
                } else {
                    let new_id = core.create_tab(window_id, None, true)?;
                    effects.push(HostEffect::CreateTabView {
                        window_id,
                        tab_id: new_id,
                        url: seu_core::NEWTAB_URL.to_string(),
                    });
                    new_id
                };
                effects.push(HostEffect::ActivateTabView {
                    window_id,
                    tab_id: target_id,
                });
            }
            // Put Studio tab to sleep when leaving Studio so pure browsing uses 0 MB extra.
            if let Some(sid) = existing_studio {
                if core.mark_tab_sleeping(sid, true).is_ok() {
                    effects.push(HostEffect::SleepTabView {
                        window_id,
                        tab_id: sid,
                    });
                    effects.push(HostEffect::BroadcastMemory);
                }
            }
            effects.push(HostEffect::RelayoutWindow { window_id });
            effects.push(HostEffect::BroadcastState { window_id });
            Ok(effects)
        }
        "studio" => {
            let _ = core.close_split_view(window_id);
            let studio_id = if let Some(sid) = existing_studio {
                let was_sleeping = core
                    .get_tab(sid)
                    .map(|t| t.state == TabLifecycleState::Sleeping)
                    .unwrap_or(false);
                core.activate_tab(window_id, sid)?;
                if was_sleeping {
                    effects.push(HostEffect::WakeTabView {
                        window_id,
                        tab_id: sid,
                        url: seu_core::STUDIO_URL.to_string(),
                        scroll_x: 0,
                        scroll_y: 0,
                    });
                }
                sid
            } else {
                let sid = core.create_tab(window_id, Some(seu_core::STUDIO_URL), true)?;
                effects.push(HostEffect::CreateTabView {
                    window_id,
                    tab_id: sid,
                    url: seu_core::STUDIO_URL.to_string(),
                });
                sid
            };
            effects.push(HostEffect::ActivateTabView {
                window_id,
                tab_id: studio_id,
            });
            effects.push(HostEffect::RelayoutWindow { window_id });
            effects.push(HostEffect::BroadcastState { window_id });
            effects.push(HostEffect::BroadcastMemory);
            Ok(effects)
        }
        "split" => {
            // Pick the paired web tab before activating Studio (prefer current active web tab).
            let preferred_web = snap
                .active_tab_id
                .filter(|&id| Some(id) != existing_studio)
                .or_else(|| {
                    snap.tabs
                        .iter()
                        .find(|t| Some(t.id) != existing_studio)
                        .map(|t| t.id)
                });

            let studio_id = if let Some(sid) = existing_studio {
                let was_sleeping = core
                    .get_tab(sid)
                    .map(|t| t.state == TabLifecycleState::Sleeping)
                    .unwrap_or(false);
                core.activate_tab(window_id, sid)?;
                if was_sleeping {
                    effects.push(HostEffect::WakeTabView {
                        window_id,
                        tab_id: sid,
                        url: seu_core::STUDIO_URL.to_string(),
                        scroll_x: 0,
                        scroll_y: 0,
                    });
                }
                sid
            } else {
                let sid = core.create_tab(window_id, Some(seu_core::STUDIO_URL), true)?;
                effects.push(HostEffect::CreateTabView {
                    window_id,
                    tab_id: sid,
                    url: seu_core::STUDIO_URL.to_string(),
                });
                sid
            };

            let (paired, created_web) = core.toggle_split_view(window_id, preferred_web)?;
            if let Some(pid) = paired {
                let url = core
                    .get_tab(pid)
                    .map(|t| t.url.clone())
                    .unwrap_or_else(|| seu_core::NEWTAB_URL.to_string());
                if created_web {
                    effects.push(HostEffect::CreateTabView {
                        window_id,
                        tab_id: pid,
                        url,
                    });
                } else {
                    effects.push(HostEffect::WakeTabView {
                        window_id,
                        tab_id: pid,
                        url,
                        scroll_x: 0,
                        scroll_y: 0,
                    });
                }
            }
            let _ = studio_id;
            effects.push(HostEffect::RelayoutWindow { window_id });
            effects.push(HostEffect::BroadcastState { window_id });
            effects.push(HostEffect::BroadcastMemory);
            Ok(effects)
        }
        _ => Err(BridgeError::InvalidArgument(format!(
            "unknown studio mode: {mode}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_registry() -> ThemeRegistry {
        ThemeRegistry::load_builtin().unwrap_or_else(|_| {
            // Unreachable when builtin y2k-maximal exists; keeps tests unwrap-free.
            panic!("builtin theme registry failed to load")
        })
    }

    #[test]
    fn denies_non_seu_origins() {
        let mut core = BrowserCore::new();
        let themes = test_registry();
        let mut blocker = BlockerService::new();
        let mut studio = StudioService::new();
        let err = handle_raw_request(
            "https://evil.example/",
            r#"{"method":"settings.get"}"#,
            &mut core,
            &themes,
            None,
            &mut blocker,
            &mut studio,
            None,
            None,
        )
        .err();
        assert_eq!(err, Some(BridgeError::OriginDenied));
    }

    #[test]
    fn allows_seu_origin_and_dispatches_requests() {
        let mut core = BrowserCore::new();
        let themes = test_registry();
        let mut blocker = BlockerService::new();
        let mut studio = StudioService::new();
        let (win_id, _) = core.create_window(false, None);

        let payload = format!(
            r#"{{"method":"tabs.create","params":{{"window_id":{win_id},"url":"docs.rs"}}}}"#
        );
        let out = handle_raw_request(
            "seu://shell/",
            &payload,
            &mut core,
            &themes,
            None,
            &mut blocker,
            &mut studio,
            None,
            None,
        );
        assert!(out.is_ok());
        if let Ok(output) = out {
            assert!(matches!(
                output.response,
                BridgeResponse::TabCreated { ref url, .. } if url == "https://docs.rs"
            ));
            assert!(!output.effects.is_empty());
        }

        let theme_out = handle_raw_request(
            "seu://shell/",
            r#"{"method":"themes.getActive"}"#,
            &mut core,
            &themes,
            None,
            &mut blocker,
            &mut studio,
            None,
            None,
        );
        assert!(matches!(
            theme_out.map(|o| o.response),
            Ok(BridgeResponse::ThemeActive(ref t)) if t.manifest.id == "y2k-maximal"
        ));

        let rebind = handle_raw_request(
            "seu://settings/",
            r#"{"method":"shortcuts.set","params":{"action":"tab.new","combo":"Ctrl+Q"}}"#,
            &mut core,
            &themes,
            None,
            &mut blocker,
            &mut studio,
            None,
            None,
        );
        assert!(rebind.is_ok());
        assert_eq!(
            core.settings.shortcuts.get("tab.new").map(String::as_str),
            Some("Ctrl+Q")
        );

        let allow = handle_raw_request(
            "seu://overlay/",
            r#"{"method":"blocker.toggleSite","params":{"domain":"example.com"}}"#,
            &mut core,
            &themes,
            None,
            &mut blocker,
            &mut studio,
            None,
            None,
        );
        assert!(allow.is_ok());
        assert!(core
            .settings
            .blocker_allowlist
            .contains(&"example.com".to_string()));

        // Verify 3-way studio mode switching: split -> studio -> browser (sleeps studio tab).
        let split_res = handle_raw_request(
            "seu://shell/",
            &format!(
                r#"{{"method":"studio.setMode","params":{{"window_id":{win_id},"mode":"split"}}}}"#
            ),
            &mut core,
            &themes,
            None,
            &mut blocker,
            &mut studio,
            None,
            None,
        );
        assert!(split_res.is_ok());
        if let Ok(o) = split_res {
            assert!(matches!(
                o.response,
                BridgeResponse::WindowSnapshot(ref s) if s.split_tab_id.is_some()
            ));
        }

        let browser_res = handle_raw_request(
            "seu://shell/",
            &format!(
                r#"{{"method":"studio.setMode","params":{{"window_id":{win_id},"mode":"browser"}}}}"#
            ),
            &mut core,
            &themes,
            None,
            &mut blocker,
            &mut studio,
            None,
            None,
        );
        assert!(browser_res.is_ok());
        if let Ok(o) = browser_res {
            assert!(o
                .effects
                .iter()
                .any(|e| matches!(e, HostEffect::SleepTabView { .. })));
        }
    }

    #[test]
    fn rejects_invalid_json_and_invalid_locale() {
        let mut core = BrowserCore::new();
        let themes = test_registry();
        let mut blocker = BlockerService::new();
        let mut studio = StudioService::new();
        assert!(matches!(
            handle_raw_request(
                "seu://shell/",
                "{not-json",
                &mut core,
                &themes,
                None,
                &mut blocker,
                &mut studio,
                None,
            ),
            Err(BridgeError::InvalidJson(_))
        ));
        assert!(matches!(
            handle_raw_request(
                "seu://shell/",
                r#"{"method":"settings.setLocale","params":{"locale":"xx-YY"}}"#,
                &mut core,
                &themes,
                None,
                &mut blocker,
                &mut studio,
                None,
            ),
            Err(BridgeError::InvalidArgument(_))
        ));
    }
}
