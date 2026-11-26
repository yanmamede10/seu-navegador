//! Core platform-agnostic browser state: windows, tabs, tab groups,
//! omnibox URL resolution, and locale/search settings.
//!
//! By architectural contract (SPEC §12.1), this crate MUST NOT depend on CEF
//! or Win32 so it can be shared with Android via UniFFI in the future.

pub mod game_mode;
pub mod media;
pub mod memory;
pub mod shortcuts;

use game_mode::{
    GameModeConfig, GameModeDetector, GameModeStatus, GameModeTransition, GameModeTrigger,
    GameProbeSample,
};
use media::{MediaSessionInfo, NowPlayingState};
use memory::{
    evaluate_sleep_eligibility, select_tabs_to_sleep, MemoryPolicy, MemoryStats, SleepBlockReason,
    TabMemoryMeta,
};
use serde::{Deserialize, Serialize};
use shortcuts::{
    default_chrome_shortcuts, default_quick_shortcuts, merge_with_defaults, KeyCombo,
    QuickShortcut, ShortcutAction,
};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const NEWTAB_URL: &str = "seu://newtab/";
pub const SHELL_URL: &str = "seu://shell/";
pub const OVERLAY_URL: &str = "seu://overlay/";
pub const STUDIO_URL: &str = "seu://studio/";

pub type WindowId = u64;
pub type TabId = u64;
pub type GroupId = u64;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoreError {
    #[error("window not found: {0}")]
    WindowNotFound(WindowId),
    #[error("tab not found: {0}")]
    TabNotFound(TabId),
    #[error("tab group not found: {0}")]
    GroupNotFound(GroupId),
    #[error("invalid index: {0}")]
    InvalidIndex(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Locale {
    #[default]
    PtBr,
    En,
}

impl Locale {
    pub fn as_code(self) -> &'static str {
        match self {
            Self::PtBr => "pt-BR",
            Self::En => "en",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code.to_ascii_lowercase().as_str() {
            "pt-br" | "pt" => Some(Self::PtBr),
            "en" | "en-us" | "en-gb" => Some(Self::En),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SearchEngine {
    #[default]
    DuckDuckGo,
    Google,
    Brave,
    Startpage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SearchCategory {
    #[default]
    Web,
    Videos,
    Images,
    Code,
    Docs,
}

impl SearchEngine {
    pub fn search_url(self, query: &str) -> String {
        let encoded = percent_encode_query(query);
        match self {
            Self::DuckDuckGo => format!("https://duckduckgo.com/?q={encoded}"),
            Self::Google => format!("https://www.google.com/search?q={encoded}"),
            Self::Brave => format!("https://search.brave.com/search?q={encoded}"),
            Self::Startpage => format!("https://www.startpage.com/do/search?q={encoded}"),
        }
    }

    pub fn category_search_url(self, query: &str, category: SearchCategory) -> String {
        let trimmed = query.trim();
        if matches!(category, SearchCategory::Web) {
            return resolve_omnibox_input(trimmed, self);
        }
        let encoded = percent_encode_query(trimmed);
        match category {
            SearchCategory::Web => resolve_omnibox_input(trimmed, self),
            SearchCategory::Videos => {
                format!("https://www.youtube.com/results?search_query={encoded}")
            }
            SearchCategory::Images => match self {
                Self::Google => format!("https://www.google.com/search?tbm=isch&q={encoded}"),
                _ => format!("https://duckduckgo.com/?q={encoded}&iax=images&ia=images"),
            },
            SearchCategory::Code => format!("https://github.com/search?q={encoded}"),
            SearchCategory::Docs => format!("https://docs.rs/releases/search?query={encoded}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabLifecycleState {
    Active,
    Background,
    Sleeping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabGroup {
    pub id: GroupId,
    pub title: String,
    pub color: String,
    pub collapsed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabInfo {
    pub id: TabId,
    pub window_id: WindowId,
    pub url: String,
    pub title: String,
    pub favicon: Option<String>,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub pinned: bool,
    pub audible: bool,
    pub state: TabLifecycleState,
    pub group_id: Option<GroupId>,
    #[serde(default)]
    pub has_thumbnail: bool,
    #[serde(default)]
    pub sleep_protected_reason: Option<SleepBlockReason>,
    #[serde(default)]
    pub blocked_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedTabRecord {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub group_id: Option<GroupId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DevToolsDock {
    #[default]
    Right,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedGroup {
    pub title: String,
    pub color: String,
    pub collapsed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTab {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub group_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWindow {
    pub tabs: Vec<SavedTab>,
    pub groups: Vec<SavedGroup>,
    pub active_tab_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SavedSession {
    pub windows: Vec<SavedWindow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSnapshot {
    pub id: WindowId,
    pub incognito: bool,
    pub active_tab_id: Option<TabId>,
    pub tabs: Vec<TabInfo>,
    pub groups: Vec<TabGroup>,
    #[serde(default)]
    pub split_tab_id: Option<TabId>,
    #[serde(default = "default_split_ratio")]
    pub split_ratio_pct: u8,
    #[serde(default)]
    pub devtools_open: bool,
    #[serde(default)]
    pub devtools_dock: DevToolsDock,
    #[serde(default = "default_devtools_ratio")]
    pub devtools_ratio_pct: u8,
}

fn default_split_ratio() -> u8 {
    55
}

fn default_devtools_ratio() -> u8 {
    38
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreSettings {
    pub locale: Locale,
    pub search_engine: SearchEngine,
    pub active_theme_id: String,
    pub game_mode_enabled: bool,
    #[serde(default = "default_chrome_shortcuts")]
    pub shortcuts: BTreeMap<String, String>,
    #[serde(default = "default_quick_shortcuts")]
    pub quick_shortcuts: Vec<QuickShortcut>,
    #[serde(default)]
    pub memory_policy: MemoryPolicy,
    #[serde(default = "default_true")]
    pub blocker_enabled: bool,
    #[serde(default)]
    pub blocker_allowlist: Vec<String>,
    #[serde(default = "default_true")]
    pub restore_session_on_startup: bool,
    #[serde(default)]
    pub studio_last_workspace: Option<String>,
    #[serde(default = "default_true")]
    pub site_skins_enabled: bool,
    #[serde(default)]
    pub allowed_site_script_themes: Vec<String>,
    #[serde(default)]
    pub game_mode_config: GameModeConfig,
}

fn default_true() -> bool {
    true
}

impl Default for CoreSettings {
    fn default() -> Self {
        Self {
            locale: Locale::PtBr,
            search_engine: SearchEngine::DuckDuckGo,
            active_theme_id: "y2k-maximal".to_string(),
            game_mode_enabled: false,
            shortcuts: default_chrome_shortcuts(),
            quick_shortcuts: default_quick_shortcuts(),
            memory_policy: MemoryPolicy::default(),
            blocker_enabled: true,
            blocker_allowlist: Vec::new(),
            restore_session_on_startup: true,
            studio_last_workspace: None,
            site_skins_enabled: true,
            allowed_site_script_themes: Vec::new(),
            game_mode_config: GameModeConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
struct WindowEntry {
    id: WindowId,
    incognito: bool,
    tab_ids: Vec<TabId>,
    active_tab_id: Option<TabId>,
    groups: Vec<TabGroup>,
    closed_tabs: VecDeque<ClosedTabRecord>,
    split_tab_id: Option<TabId>,
    split_ratio_pct: u8,
    devtools_open: bool,
    devtools_dock: DevToolsDock,
    devtools_ratio_pct: u8,
}

const MAX_CLOSED_TABS_PER_WINDOW: usize = 25;

#[derive(Debug, Default)]
pub struct BrowserCore {
    next_window_id: u64,
    next_tab_id: u64,
    next_group_id: u64,
    windows: HashMap<WindowId, WindowEntry>,
    window_order: Vec<WindowId>,
    tabs: HashMap<TabId, TabInfo>,
    tab_meta: HashMap<TabId, TabMemoryMeta>,
    media_sessions: HashMap<TabId, MediaSessionInfo>,
    game_mode_detector: GameModeDetector,
    pub game_mode_status: GameModeStatus,
    pub memory_stats: MemoryStats,
    pub settings: CoreSettings,
}

impl BrowserCore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_window(
        &mut self,
        incognito: bool,
        initial_url: Option<&str>,
    ) -> (WindowId, TabId) {
        self.next_window_id += 1;
        let window_id = self.next_window_id;

        let entry = WindowEntry {
            id: window_id,
            incognito,
            tab_ids: Vec::new(),
            active_tab_id: None,
            groups: Vec::new(),
            closed_tabs: VecDeque::new(),
            split_tab_id: None,
            split_ratio_pct: default_split_ratio(),
            devtools_open: false,
            devtools_dock: DevToolsDock::Right,
            devtools_ratio_pct: default_devtools_ratio(),
        };
        self.windows.insert(window_id, entry);
        self.window_order.push(window_id);

        let url = initial_url.unwrap_or(NEWTAB_URL);
        let tab_id = self
            .create_tab(window_id, Some(url), true)
            .unwrap_or_default();
        (window_id, tab_id)
    }

    pub fn close_window(&mut self, window_id: WindowId) -> Result<Vec<TabId>, CoreError> {
        let entry = self
            .windows
            .remove(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;
        self.window_order.retain(|&id| id != window_id);
        for tab_id in &entry.tab_ids {
            self.tabs.remove(tab_id);
            self.tab_meta.remove(tab_id);
            self.media_sessions.remove(tab_id);
        }
        self.recompute_tab_counts();
        Ok(entry.tab_ids)
    }

    pub fn window_ids(&self) -> &[WindowId] {
        &self.window_order
    }

    pub fn window_snapshot(&self, window_id: WindowId) -> Result<WindowSnapshot, CoreError> {
        let win = self
            .windows
            .get(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;
        let tabs = win
            .tab_ids
            .iter()
            .filter_map(|id| {
                let mut tab = self.tabs.get(id)?.clone();
                if let Some(meta) = self.tab_meta.get(id) {
                    tab.has_thumbnail = meta.has_thumbnail;
                    tab.sleep_protected_reason =
                        evaluate_sleep_eligibility(&tab, meta, &self.settings.memory_policy).err();
                }
                Some(tab)
            })
            .collect();
        Ok(WindowSnapshot {
            id: win.id,
            incognito: win.incognito,
            active_tab_id: win.active_tab_id,
            tabs,
            groups: win.groups.clone(),
            split_tab_id: win.split_tab_id,
            split_ratio_pct: win.split_ratio_pct,
            devtools_open: win.devtools_open,
            devtools_dock: win.devtools_dock,
            devtools_ratio_pct: win.devtools_ratio_pct,
        })
    }

    pub fn get_tab(&self, tab_id: TabId) -> Option<&TabInfo> {
        self.tabs.get(&tab_id)
    }

    pub fn create_tab(
        &mut self,
        window_id: WindowId,
        url: Option<&str>,
        activate: bool,
    ) -> Result<TabId, CoreError> {
        if !self.windows.contains_key(&window_id) {
            return Err(CoreError::WindowNotFound(window_id));
        }

        self.next_tab_id += 1;
        let tab_id = self.next_tab_id;
        let resolved_url = url
            .map(|u| resolve_omnibox_input(u, self.settings.search_engine))
            .unwrap_or_else(|| NEWTAB_URL.to_string());

        let title = if resolved_url == NEWTAB_URL {
            match self.settings.locale {
                Locale::PtBr => "Nova aba".to_string(),
                Locale::En => "New tab".to_string(),
            }
        } else {
            resolved_url.clone()
        };

        let tab = TabInfo {
            id: tab_id,
            window_id,
            url: resolved_url.clone(),
            title: title.clone(),
            favicon: None,
            loading: false,
            can_go_back: false,
            can_go_forward: false,
            pinned: false,
            audible: false,
            state: TabLifecycleState::Background,
            group_id: None,
            has_thumbnail: false,
            sleep_protected_reason: None,
            blocked_count: 0,
        };
        self.tabs.insert(tab_id, tab);

        let mut meta = TabMemoryMeta {
            last_active_ms: unix_now_ms(),
            ..Default::default()
        };
        meta.record_navigation(resolved_url, title);
        self.tab_meta.insert(tab_id, meta);

        if let Some(win) = self.windows.get_mut(&window_id) {
            win.tab_ids.push(tab_id);
        }

        if activate
            || self
                .windows
                .get(&window_id)
                .and_then(|w| w.active_tab_id)
                .is_none()
        {
            self.activate_tab(window_id, tab_id)?;
        }

        self.recompute_tab_counts();
        Ok(tab_id)
    }

    pub fn activate_tab(&mut self, window_id: WindowId, tab_id: TabId) -> Result<(), CoreError> {
        let now = unix_now_ms();
        let win = self
            .windows
            .get_mut(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;

        if !win.tab_ids.contains(&tab_id) {
            return Err(CoreError::TabNotFound(tab_id));
        }

        let prev_active = win.active_tab_id;
        // If clicking the tab currently docked in the right split pane, smoothly swap panes.
        if win.split_tab_id == Some(tab_id) {
            win.split_tab_id = prev_active.filter(|&id| id != tab_id);
        }
        let split_id = win.split_tab_id;
        win.active_tab_id = Some(tab_id);

        if let Some(prev_id) = prev_active {
            if prev_id != tab_id && split_id != Some(prev_id) {
                if let Some(prev_tab) = self.tabs.get_mut(&prev_id) {
                    if prev_tab.state == TabLifecycleState::Active {
                        prev_tab.state = TabLifecycleState::Background;
                    }
                }
                if let Some(prev_meta) = self.tab_meta.get_mut(&prev_id) {
                    prev_meta.last_active_ms = now;
                }
            }
        }

        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.state = TabLifecycleState::Active;
        }
        if let Some(meta) = self.tab_meta.get_mut(&tab_id) {
            meta.last_active_ms = now;
            meta.slept_at_ms = None;
        }
        self.recompute_tab_counts();
        Ok(())
    }

    pub fn close_tab(&mut self, tab_id: TabId) -> Result<(WindowId, Option<TabId>), CoreError> {
        let tab = self
            .tabs
            .remove(&tab_id)
            .ok_or(CoreError::TabNotFound(tab_id))?;
        self.tab_meta.remove(&tab_id);
        self.media_sessions.remove(&tab_id);
        let window_id = tab.window_id;
        let win = self
            .windows
            .get_mut(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;

        let pos = win
            .tab_ids
            .iter()
            .position(|&id| id == tab_id)
            .ok_or(CoreError::TabNotFound(tab_id))?;
        win.tab_ids.remove(pos);

        if !win.incognito {
            win.closed_tabs.push_front(ClosedTabRecord {
                url: tab.url,
                title: tab.title,
                pinned: tab.pinned,
                group_id: tab.group_id,
            });
            while win.closed_tabs.len() > MAX_CLOSED_TABS_PER_WINDOW {
                win.closed_tabs.pop_back();
            }
        }

        let next_active = if win.active_tab_id == Some(tab_id) {
            if win.tab_ids.is_empty() {
                win.active_tab_id = None;
                win.split_tab_id = None;
                None
            } else {
                let next_idx = pos.min(win.tab_ids.len() - 1);
                let next_id = win.tab_ids[next_idx];
                win.active_tab_id = Some(next_id);
                if win.split_tab_id == Some(next_id) {
                    win.split_tab_id = None;
                }
                if let Some(next_tab) = self.tabs.get_mut(&next_id) {
                    next_tab.state = TabLifecycleState::Active;
                }
                Some(next_id)
            }
        } else {
            if win.split_tab_id == Some(tab_id) {
                win.split_tab_id = None;
            }
            win.active_tab_id
        };

        self.recompute_tab_counts();
        Ok((window_id, next_active))
    }

    pub fn reopen_closed_tab(&mut self, window_id: WindowId) -> Result<Option<TabId>, CoreError> {
        let record = {
            let win = self
                .windows
                .get_mut(&window_id)
                .ok_or(CoreError::WindowNotFound(window_id))?;
            win.closed_tabs.pop_front()
        };

        let Some(rec) = record else {
            return Ok(None);
        };

        let tab_id = self.create_tab(window_id, Some(&rec.url), true)?;
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.title = rec.title;
            tab.pinned = rec.pinned;
            let group_exists = self
                .windows
                .get(&window_id)
                .map(|w| w.groups.iter().any(|g| Some(g.id) == rec.group_id))
                .unwrap_or(false);
            if group_exists {
                tab.group_id = rec.group_id;
            }
        }
        Ok(Some(tab_id))
    }

    pub fn duplicate_tab(&mut self, tab_id: TabId) -> Result<TabId, CoreError> {
        let (window_id, url, group_id) = {
            let tab = self
                .tabs
                .get(&tab_id)
                .ok_or(CoreError::TabNotFound(tab_id))?;
            (tab.window_id, tab.url.clone(), tab.group_id)
        };
        let new_id = self.create_tab(window_id, Some(&url), true)?;
        if let Some(tab) = self.tabs.get_mut(&new_id) {
            tab.group_id = group_id;
        }
        Ok(new_id)
    }

    pub fn toggle_pin_tab(&mut self, tab_id: TabId) -> Result<bool, CoreError> {
        let (window_id, pinned) = {
            let tab = self
                .tabs
                .get_mut(&tab_id)
                .ok_or(CoreError::TabNotFound(tab_id))?;
            tab.pinned = !tab.pinned;
            (tab.window_id, tab.pinned)
        };

        // Keep pinned tabs grouped at the start of the tab strip.
        if let Some(win) = self.windows.get_mut(&window_id) {
            if let Some(pos) = win.tab_ids.iter().position(|&id| id == tab_id) {
                win.tab_ids.remove(pos);
                let target_idx = win
                    .tab_ids
                    .iter()
                    .take_while(|id| self.tabs.get(id).map(|t| t.pinned).unwrap_or(false))
                    .count();
                win.tab_ids.insert(target_idx, tab_id);
            }
        }

        Ok(pinned)
    }

    pub fn reorder_tab(
        &mut self,
        window_id: WindowId,
        tab_id: TabId,
        to_index: usize,
    ) -> Result<(), CoreError> {
        let win = self
            .windows
            .get_mut(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;

        let from_idx = win
            .tab_ids
            .iter()
            .position(|&id| id == tab_id)
            .ok_or(CoreError::TabNotFound(tab_id))?;

        if to_index >= win.tab_ids.len() {
            return Err(CoreError::InvalidIndex(to_index));
        }

        win.tab_ids.remove(from_idx);
        win.tab_ids.insert(to_index, tab_id);
        Ok(())
    }

    pub fn move_tab_to_window(
        &mut self,
        tab_id: TabId,
        target_window_id: WindowId,
    ) -> Result<WindowId, CoreError> {
        if !self.windows.contains_key(&target_window_id) {
            return Err(CoreError::WindowNotFound(target_window_id));
        }

        let source_window_id = self
            .tabs
            .get(&tab_id)
            .ok_or(CoreError::TabNotFound(tab_id))?
            .window_id;

        if source_window_id == target_window_id {
            return Ok(source_window_id);
        }

        if let Some(src) = self.windows.get_mut(&source_window_id) {
            if let Some(pos) = src.tab_ids.iter().position(|&id| id == tab_id) {
                src.tab_ids.remove(pos);
                if src.active_tab_id == Some(tab_id) {
                    src.active_tab_id = if src.tab_ids.is_empty() {
                        None
                    } else {
                        let next_id = src.tab_ids[pos.min(src.tab_ids.len() - 1)];
                        if let Some(t) = self.tabs.get_mut(&next_id) {
                            t.state = TabLifecycleState::Active;
                        }
                        Some(next_id)
                    };
                }
            }
        }

        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.window_id = target_window_id;
            tab.group_id = None;
        }

        if let Some(dst) = self.windows.get_mut(&target_window_id) {
            dst.tab_ids.push(tab_id);
        }
        self.activate_tab(target_window_id, tab_id)?;

        Ok(source_window_id)
    }

    pub fn update_tab_navigation(
        &mut self,
        tab_id: TabId,
        url: Option<String>,
        title: Option<String>,
        loading: Option<bool>,
        can_go_back: Option<bool>,
        can_go_forward: Option<bool>,
    ) -> Result<WindowId, CoreError> {
        let tab = self
            .tabs
            .get_mut(&tab_id)
            .ok_or(CoreError::TabNotFound(tab_id))?;
        let meta = self.tab_meta.entry(tab_id).or_default();

        if let Some(u) = url {
            if u != tab.url {
                // Reset per-document dirty/scroll/blocker counters on new URL load.
                meta.dirty_form = false;
                meta.has_beforeunload = false;
                meta.scroll_x = 0;
                meta.scroll_y = 0;
                tab.blocked_count = 0;
            }
            tab.url = u.clone();
            meta.record_navigation(u, tab.title.clone());
        }
        if let Some(t) = title {
            if !t.is_empty() {
                tab.title = t.clone();
                meta.update_current_title(&t);
            }
        }
        if let Some(l) = loading {
            tab.loading = l;
        }
        if let Some(b) = can_go_back {
            tab.can_go_back = b || meta.can_go_back();
        }
        if let Some(f) = can_go_forward {
            tab.can_go_forward = f || meta.can_go_forward();
        }
        Ok(tab.window_id)
    }

    pub fn tab_memory_meta(&self, tab_id: TabId) -> Option<&TabMemoryMeta> {
        self.tab_meta.get(&tab_id)
    }

    pub fn update_tab_probe(
        &mut self,
        tab_id: TabId,
        dirty_form: Option<bool>,
        has_beforeunload: Option<bool>,
        scroll_x: Option<i32>,
        scroll_y: Option<i32>,
        audible: Option<bool>,
    ) -> Option<WindowId> {
        let tab = self.tabs.get_mut(&tab_id)?;
        let meta = self.tab_meta.entry(tab_id).or_default();
        if let Some(d) = dirty_form {
            meta.dirty_form = d;
        }
        if let Some(b) = has_beforeunload {
            meta.has_beforeunload = b;
        }
        if let Some(x) = scroll_x {
            meta.scroll_x = x;
        }
        if let Some(y) = scroll_y {
            meta.scroll_y = y;
        }
        let mut audible_changed = false;
        if let Some(a) = audible {
            if tab.audible != a {
                tab.audible = a;
                audible_changed = true;
            }
        }
        if audible_changed {
            Some(tab.window_id)
        } else {
            None
        }
    }

    pub fn set_tab_media_capture(&mut self, tab_id: TabId, active: bool) {
        if let Some(meta) = self.tab_meta.get_mut(&tab_id) {
            meta.media_capture_active = active;
        }
    }

    pub fn set_tab_thumbnail_ready(&mut self, tab_id: TabId, ready: bool) {
        if let Some(meta) = self.tab_meta.get_mut(&tab_id) {
            meta.has_thumbnail = ready;
        }
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.has_thumbnail = ready;
        }
    }

    /// Marks an eligible background tab as `Sleeping`. Returns its `WindowId` on success.
    pub fn mark_tab_sleeping(&mut self, tab_id: TabId, force: bool) -> Result<WindowId, CoreError> {
        let tab = self
            .tabs
            .get(&tab_id)
            .ok_or(CoreError::TabNotFound(tab_id))?;
        let meta = self.tab_meta.get(&tab_id).cloned().unwrap_or_default();

        if !force {
            if evaluate_sleep_eligibility(tab, &meta, &self.settings.memory_policy).is_err() {
                return Err(CoreError::InvalidIndex(0));
            }
        } else if tab.state != TabLifecycleState::Background {
            return Err(CoreError::InvalidIndex(0));
        }

        let window_id = tab.window_id;
        if let Some(t) = self.tabs.get_mut(&tab_id) {
            t.state = TabLifecycleState::Sleeping;
            t.loading = false;
            t.audible = false;
            t.can_go_back = meta.can_go_back();
            t.can_go_forward = meta.can_go_forward();
        }
        if let Some(m) = self.tab_meta.get_mut(&tab_id) {
            m.slept_at_ms = Some(unix_now_ms());
        }
        self.recompute_tab_counts();
        Ok(window_id)
    }

    /// Selects background tabs eligible to sleep right now under the active `MemoryPolicy`.
    pub fn select_due_sleep_tabs(&self, now_ms: u64, force_all_eligible: bool) -> Vec<TabId> {
        let default_meta = TabMemoryMeta::default();
        let pairs: Vec<(&TabInfo, &TabMemoryMeta)> = self
            .tabs
            .values()
            .map(|t| (t, self.tab_meta.get(&t.id).unwrap_or(&default_meta)))
            .collect();
        select_tabs_to_sleep(
            &pairs,
            &self.settings.memory_policy,
            now_ms,
            force_all_eligible,
        )
    }

    pub fn record_sleep_ram_delta(&mut self, tab_id: TabId, delta_mb: u32, shared_renderer: bool) {
        if let Some(m) = self.tab_meta.get_mut(&tab_id) {
            m.saved_mb = delta_mb;
        }
        self.memory_stats.saved_mb = self.memory_stats.saved_mb.saturating_add(delta_mb);
        self.memory_stats.shared_renderer_detected = shared_renderer;
    }

    pub fn record_batch_sleep_ram_delta(
        &mut self,
        tab_ids: &[TabId],
        total_delta_mb: u32,
        shared_renderer: bool,
    ) {
        let sleeping_ids: Vec<TabId> = tab_ids
            .iter()
            .copied()
            .filter(|id| {
                self.tabs
                    .get(id)
                    .map(|t| t.state == TabLifecycleState::Sleeping)
                    .unwrap_or(false)
            })
            .collect();
        if sleeping_ids.is_empty() {
            return;
        }
        let per_tab = total_delta_mb / (sleeping_ids.len() as u32);
        for tid in sleeping_ids {
            if let Some(m) = self.tab_meta.get_mut(&tid) {
                m.saved_mb = per_tab;
            }
        }
        self.memory_stats.saved_mb = self.memory_stats.saved_mb.saturating_add(total_delta_mb);
        self.memory_stats.shared_renderer_detected = shared_renderer;
    }

    pub fn increment_tab_blocked(&mut self, tab_id: TabId) -> Option<WindowId> {
        let tab = self.tabs.get_mut(&tab_id)?;
        tab.blocked_count = tab.blocked_count.saturating_add(1);
        Some(tab.window_id)
    }

    pub fn update_os_memory_metrics(
        &mut self,
        total_private_mb: u32,
        total_working_set_mb: u32,
        process_count: usize,
    ) {
        self.memory_stats.total_private_mb = total_private_mb;
        self.memory_stats.total_working_set_mb = total_working_set_mb;
        self.memory_stats.process_count = process_count;
        self.recompute_tab_counts();
    }

    pub fn step_tab_history_back(&mut self, tab_id: TabId) -> Option<(WindowId, String)> {
        let wid = self.tabs.get(&tab_id)?.window_id;
        let meta = self.tab_meta.get_mut(&tab_id)?;
        let entry = meta.step_back()?.clone();
        let can_back = meta.can_go_back();
        let can_fwd = meta.can_go_forward();
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.url = entry.url.clone();
            tab.title = entry.title;
            tab.can_go_back = can_back;
            tab.can_go_forward = can_fwd;
        }
        Some((wid, entry.url))
    }

    pub fn step_tab_history_forward(&mut self, tab_id: TabId) -> Option<(WindowId, String)> {
        let wid = self.tabs.get(&tab_id)?.window_id;
        let meta = self.tab_meta.get_mut(&tab_id)?;
        let entry = meta.step_forward()?.clone();
        let can_back = meta.can_go_back();
        let can_fwd = meta.can_go_forward();
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.url = entry.url.clone();
            tab.title = entry.title;
            tab.can_go_back = can_back;
            tab.can_go_forward = can_fwd;
        }
        Some((wid, entry.url))
    }

    fn recompute_tab_counts(&mut self) {
        let sleeping = self
            .tabs
            .values()
            .filter(|t| t.state == TabLifecycleState::Sleeping)
            .count();
        let live = self.tabs.len().saturating_sub(sleeping);
        self.memory_stats.sleeping_tabs_count = sleeping;
        self.memory_stats.live_tabs_count = live;
    }

    pub fn navigate_tab(&mut self, tab_id: TabId, raw_input: &str) -> Result<String, CoreError> {
        let resolved = resolve_omnibox_input(raw_input, self.settings.search_engine);
        let tab = self
            .tabs
            .get_mut(&tab_id)
            .ok_or(CoreError::TabNotFound(tab_id))?;
        tab.url = resolved.clone();
        Ok(resolved)
    }

    pub fn navigate_tab_category_search(
        &mut self,
        tab_id: TabId,
        query: &str,
        category: SearchCategory,
    ) -> Result<String, CoreError> {
        let resolved = self
            .settings
            .search_engine
            .category_search_url(query, category);
        let tab = self
            .tabs
            .get_mut(&tab_id)
            .ok_or(CoreError::TabNotFound(tab_id))?;
        tab.url = resolved.clone();
        Ok(resolved)
    }

    pub fn toggle_game_mode(&mut self) -> bool {
        let now_ms = unix_now_ms();
        if self.settings.game_mode_enabled {
            self.exit_game_mode(now_ms);
            false
        } else {
            let _ = self.enter_game_mode(GameModeTrigger::Manual, None, now_ms);
            true
        }
    }

    pub fn enter_game_mode(
        &mut self,
        trigger: GameModeTrigger,
        process: Option<String>,
        now_ms: u64,
    ) -> Vec<(WindowId, TabId)> {
        self.settings.game_mode_enabled = true;
        let ram_before = self.memory_stats.total_private_mb;
        let due = self.select_due_sleep_tabs(now_ms, true);
        let mut slept = Vec::new();
        for tid in due {
            if let Ok(wid) = self.mark_tab_sleeping(tid, false) {
                self.media_sessions.remove(&tid);
                slept.push((wid, tid));
            }
        }
        self.game_mode_status = GameModeStatus {
            active: true,
            trigger: Some(trigger),
            detected_process: process,
            tabs_slept: slept.len(),
            ram_before_mb: ram_before,
            ram_saved_mb: 0,
            last_exit_saved_mb: self.game_mode_status.last_exit_saved_mb,
            last_exit_at_ms: self.game_mode_status.last_exit_at_ms,
        };
        slept
    }

    pub fn exit_game_mode(&mut self, now_ms: u64) {
        self.settings.game_mode_enabled = false;
        let computed = self
            .game_mode_status
            .ram_before_mb
            .saturating_sub(self.memory_stats.total_private_mb);
        let total_saved = self.game_mode_status.ram_saved_mb.max(computed);
        self.game_mode_status.active = false;
        self.game_mode_status.trigger = None;
        self.game_mode_status.detected_process = None;
        self.game_mode_status.ram_saved_mb = total_saved;
        self.game_mode_status.last_exit_saved_mb = total_saved;
        self.game_mode_status.last_exit_at_ms = now_ms;
        self.game_mode_detector.reset();
    }

    pub fn set_game_mode_config(&mut self, config: GameModeConfig) {
        self.settings.game_mode_config = config.sanitized();
        if !self.settings.game_mode_config.auto_detect {
            self.game_mode_detector.reset();
        }
    }

    pub fn record_game_mode_ram_delta(&mut self, delta_mb: u32) {
        if self.game_mode_status.active {
            self.game_mode_status.ram_saved_mb =
                self.game_mode_status.ram_saved_mb.saturating_add(delta_mb);
        }
    }

    pub fn evaluate_game_probe(&mut self, sample: &GameProbeSample) -> GameModeTransition {
        self.game_mode_detector.evaluate(
            sample,
            &self.settings.game_mode_config,
            &self.game_mode_status,
        )
    }

    pub fn update_tab_media(
        &mut self,
        tab_id: TabId,
        info: Option<MediaSessionInfo>,
    ) -> Option<WindowId> {
        let wid = self.tabs.get(&tab_id)?.window_id;
        let is_playing = info.as_ref().map(|m| m.playing).unwrap_or(false);
        let changed = match info {
            Some(session) => {
                let prev = self.media_sessions.get(&tab_id);
                let diff = prev != Some(&session);
                self.media_sessions.insert(tab_id, session);
                diff
            }
            None => self.media_sessions.remove(&tab_id).is_some(),
        };
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            if tab.audible != is_playing {
                tab.audible = is_playing;
                return Some(wid);
            }
        }
        if changed {
            Some(wid)
        } else {
            None
        }
    }

    pub fn clear_tab_media(&mut self, tab_id: TabId) {
        self.media_sessions.remove(&tab_id);
    }

    pub fn now_playing(&self) -> NowPlayingState {
        let live: Vec<MediaSessionInfo> = self
            .media_sessions
            .values()
            .filter(|m| {
                self.tabs
                    .get(&m.tab_id)
                    .map(|t| t.state != TabLifecycleState::Sleeping)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        NowPlayingState::from_sessions(live)
    }

    pub fn set_shortcut(&mut self, action_id: &str, combo: &str) -> Result<String, CoreError> {
        if ShortcutAction::from_id(action_id).is_none() {
            return Err(CoreError::InvalidIndex(0));
        }
        let parsed = KeyCombo::parse(combo).ok_or(CoreError::InvalidIndex(1))?;
        let canonical = parsed.canonical();
        // Clear any conflicting action bound to the exact same key combo.
        for (other_id, bound) in self.settings.shortcuts.iter_mut() {
            if other_id != action_id && bound.eq_ignore_ascii_case(&canonical) {
                bound.clear();
            }
        }
        self.settings
            .shortcuts
            .insert(action_id.to_string(), canonical.clone());
        Ok(canonical)
    }

    pub fn reset_shortcuts(&mut self) {
        self.settings.shortcuts = default_chrome_shortcuts();
    }

    pub fn hydrate_shortcuts(&mut self, raw: &BTreeMap<String, String>) {
        self.settings.shortcuts = merge_with_defaults(raw);
    }

    pub fn activate_tab_by_index(
        &mut self,
        window_id: WindowId,
        index: usize,
    ) -> Result<Option<TabId>, CoreError> {
        let target = {
            let win = self
                .windows
                .get(&window_id)
                .ok_or(CoreError::WindowNotFound(window_id))?;
            win.tab_ids.get(index).copied()
        };
        if let Some(tid) = target {
            self.activate_tab(window_id, tid)?;
        }
        Ok(target)
    }

    pub fn activate_last_tab(&mut self, window_id: WindowId) -> Result<Option<TabId>, CoreError> {
        let target = {
            let win = self
                .windows
                .get(&window_id)
                .ok_or(CoreError::WindowNotFound(window_id))?;
            win.tab_ids.last().copied()
        };
        if let Some(tid) = target {
            self.activate_tab(window_id, tid)?;
        }
        Ok(target)
    }

    pub fn create_tab_group(
        &mut self,
        window_id: WindowId,
        title: String,
        color: String,
        initial_tab_ids: &[TabId],
    ) -> Result<GroupId, CoreError> {
        let win = self
            .windows
            .get_mut(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;

        self.next_group_id += 1;
        let group_id = self.next_group_id;
        win.groups.push(TabGroup {
            id: group_id,
            title,
            color,
            collapsed: false,
        });

        for &tab_id in initial_tab_ids {
            if let Some(tab) = self.tabs.get_mut(&tab_id) {
                if tab.window_id == window_id {
                    tab.group_id = Some(group_id);
                }
            }
        }

        Ok(group_id)
    }

    pub fn toggle_group_collapse(
        &mut self,
        window_id: WindowId,
        group_id: GroupId,
    ) -> Result<bool, CoreError> {
        let win = self
            .windows
            .get_mut(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;
        let group = win
            .groups
            .iter_mut()
            .find(|g| g.id == group_id)
            .ok_or(CoreError::GroupNotFound(group_id))?;
        group.collapsed = !group.collapsed;
        Ok(group.collapsed)
    }

    pub fn assign_tab_to_group(
        &mut self,
        tab_id: TabId,
        group_id: Option<GroupId>,
    ) -> Result<WindowId, CoreError> {
        let window_id = self
            .tabs
            .get(&tab_id)
            .ok_or(CoreError::TabNotFound(tab_id))?
            .window_id;

        if let Some(gid) = group_id {
            let exists = self
                .windows
                .get(&window_id)
                .map(|w| w.groups.iter().any(|g| g.id == gid))
                .unwrap_or(false);
            if !exists {
                return Err(CoreError::GroupNotFound(gid));
            }
        }

        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.group_id = group_id;
        }
        Ok(window_id)
    }

    pub fn ungroup(&mut self, window_id: WindowId, group_id: GroupId) -> Result<(), CoreError> {
        let win = self
            .windows
            .get_mut(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;
        let before = win.groups.len();
        win.groups.retain(|g| g.id != group_id);
        if win.groups.len() == before {
            return Err(CoreError::GroupNotFound(group_id));
        }

        for tab in self.tabs.values_mut() {
            if tab.window_id == window_id && tab.group_id == Some(group_id) {
                tab.group_id = None;
            }
        }
        Ok(())
    }

    pub fn cycle_tab(
        &mut self,
        window_id: WindowId,
        forward: bool,
    ) -> Result<Option<TabId>, CoreError> {
        let next_id = {
            let win = self
                .windows
                .get(&window_id)
                .ok_or(CoreError::WindowNotFound(window_id))?;
            if win.tab_ids.len() <= 1 {
                return Ok(win.active_tab_id);
            }
            let current_idx = win
                .active_tab_id
                .and_then(|aid| win.tab_ids.iter().position(|&id| id == aid))
                .unwrap_or(0);
            let len = win.tab_ids.len();
            let next_idx = if forward {
                (current_idx + 1) % len
            } else {
                (current_idx + len - 1) % len
            };
            win.tab_ids[next_idx]
        };

        self.activate_tab(window_id, next_id)?;
        Ok(Some(next_id))
    }

    // -------------------------------------------------------------------------
    // Split View & Docked DevTools
    // -------------------------------------------------------------------------

    /// Toggles Split View for `window_id`. If closing, returns `Ok((None, false))`.
    /// If opening, pairs `secondary_tab_id` (or picks another tab / creates a new one)
    /// and returns `Ok((Some(paired_tab_id), created_new_tab))`.
    pub fn toggle_split_view(
        &mut self,
        window_id: WindowId,
        secondary_tab_id: Option<TabId>,
    ) -> Result<(Option<TabId>, bool), CoreError> {
        let (already_open, active_id, candidate) = {
            let win = self
                .windows
                .get(&window_id)
                .ok_or(CoreError::WindowNotFound(window_id))?;
            let other = secondary_tab_id
                .filter(|id| win.tab_ids.contains(id) && Some(*id) != win.active_tab_id)
                .or_else(|| {
                    win.tab_ids
                        .iter()
                        .rev()
                        .copied()
                        .find(|&id| Some(id) != win.active_tab_id)
                });
            (win.split_tab_id.is_some(), win.active_tab_id, other)
        };

        if already_open && secondary_tab_id.is_none() {
            self.close_split_view(window_id)?;
            return Ok((None, false));
        }

        let (paired_id, created_new) = if let Some(tid) = candidate {
            (tid, false)
        } else {
            let new_id = self.create_tab(window_id, None, false)?;
            if let Some(aid) = active_id {
                let _ = self.activate_tab(window_id, aid);
            }
            (new_id, true)
        };

        if let Some(win) = self.windows.get_mut(&window_id) {
            win.split_tab_id = Some(paired_id);
        }
        if let Some(tab) = self.tabs.get_mut(&paired_id) {
            tab.state = TabLifecycleState::Active;
        }
        if let Some(meta) = self.tab_meta.get_mut(&paired_id) {
            meta.last_active_ms = unix_now_ms();
            meta.slept_at_ms = None;
        }
        self.recompute_tab_counts();
        Ok((Some(paired_id), created_new))
    }

    pub fn close_split_view(&mut self, window_id: WindowId) -> Result<(), CoreError> {
        let prev_split = {
            let win = self
                .windows
                .get_mut(&window_id)
                .ok_or(CoreError::WindowNotFound(window_id))?;
            win.split_tab_id.take()
        };
        if let Some(tid) = prev_split {
            let is_active = self.windows.get(&window_id).and_then(|w| w.active_tab_id) == Some(tid);
            if !is_active {
                if let Some(tab) = self.tabs.get_mut(&tid) {
                    if tab.state == TabLifecycleState::Active {
                        tab.state = TabLifecycleState::Background;
                    }
                }
            }
        }
        self.recompute_tab_counts();
        Ok(())
    }

    pub fn set_split_ratio(&mut self, window_id: WindowId, ratio_pct: u8) -> Result<u8, CoreError> {
        let win = self
            .windows
            .get_mut(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;
        win.split_ratio_pct = ratio_pct.clamp(25, 75);
        Ok(win.split_ratio_pct)
    }

    pub fn toggle_devtools(&mut self, window_id: WindowId) -> Result<bool, CoreError> {
        let win = self
            .windows
            .get_mut(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;
        win.devtools_open = !win.devtools_open;
        Ok(win.devtools_open)
    }

    pub fn close_devtools(&mut self, window_id: WindowId) -> Result<(), CoreError> {
        let win = self
            .windows
            .get_mut(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;
        win.devtools_open = false;
        Ok(())
    }

    pub fn set_devtools_dock(
        &mut self,
        window_id: WindowId,
        dock: DevToolsDock,
    ) -> Result<(), CoreError> {
        let win = self
            .windows
            .get_mut(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;
        win.devtools_dock = dock;
        Ok(())
    }

    pub fn set_devtools_ratio(
        &mut self,
        window_id: WindowId,
        ratio_pct: u8,
    ) -> Result<u8, CoreError> {
        let win = self
            .windows
            .get_mut(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;
        win.devtools_ratio_pct = ratio_pct.clamp(20, 70);
        Ok(win.devtools_ratio_pct)
    }

    // -------------------------------------------------------------------------
    // Session Export & Restore (non-active tabs restore in Sleeping state)
    // -------------------------------------------------------------------------

    pub fn export_session(&self) -> SavedSession {
        let mut windows = Vec::new();
        for &wid in &self.window_order {
            let Some(win) = self.windows.get(&wid) else {
                continue;
            };
            if win.incognito || win.tab_ids.is_empty() {
                continue;
            }
            let group_index_map: HashMap<GroupId, usize> = win
                .groups
                .iter()
                .enumerate()
                .map(|(i, g)| (g.id, i))
                .collect();
            let groups = win
                .groups
                .iter()
                .map(|g| SavedGroup {
                    title: g.title.clone(),
                    color: g.color.clone(),
                    collapsed: g.collapsed,
                })
                .collect();

            let mut tabs = Vec::new();
            let mut active_tab_index = 0usize;
            for (idx, &tid) in win.tab_ids.iter().enumerate() {
                let Some(tab) = self.tabs.get(&tid) else {
                    continue;
                };
                if win.active_tab_id == Some(tid) {
                    active_tab_index = idx;
                }
                tabs.push(SavedTab {
                    url: tab.url.clone(),
                    title: tab.title.clone(),
                    pinned: tab.pinned,
                    group_index: tab
                        .group_id
                        .and_then(|gid| group_index_map.get(&gid).copied()),
                });
            }
            if !tabs.is_empty() {
                windows.push(SavedWindow {
                    tabs,
                    groups,
                    active_tab_index: active_tab_index.min(win.tab_ids.len().saturating_sub(1)),
                });
            }
        }
        SavedSession { windows }
    }

    /// Restores a previously exported session. Per SPEC §11, only each window's active tab
    /// starts `Active`; all other restored tabs start `Sleeping` so restored sessions consume
    /// minimal baseline RAM. Returns `(window_id, active_tab_id, active_url)` to spawn live views.
    pub fn restore_session(&mut self, session: &SavedSession) -> Vec<(WindowId, TabId, String)> {
        let mut spawned = Vec::new();
        let now = unix_now_ms();

        for saved_win in &session.windows {
            if saved_win.tabs.is_empty() {
                continue;
            }
            self.next_window_id += 1;
            let window_id = self.next_window_id;

            let mut restored_groups = Vec::new();
            let mut group_id_by_idx = Vec::new();
            for g in &saved_win.groups {
                self.next_group_id += 1;
                let gid = self.next_group_id;
                group_id_by_idx.push(gid);
                restored_groups.push(TabGroup {
                    id: gid,
                    title: g.title.clone(),
                    color: g.color.clone(),
                    collapsed: g.collapsed,
                });
            }

            let active_idx = saved_win
                .active_tab_index
                .min(saved_win.tabs.len().saturating_sub(1));
            let mut tab_ids = Vec::with_capacity(saved_win.tabs.len());
            let mut active_tab_id = 0;
            let mut active_url = NEWTAB_URL.to_string();

            for (idx, st) in saved_win.tabs.iter().enumerate() {
                self.next_tab_id += 1;
                let tab_id = self.next_tab_id;
                let is_active = idx == active_idx;
                let group_id = st
                    .group_index
                    .and_then(|gi| group_id_by_idx.get(gi).copied());
                let state = if is_active {
                    TabLifecycleState::Active
                } else {
                    TabLifecycleState::Sleeping
                };

                let tab = TabInfo {
                    id: tab_id,
                    window_id,
                    url: st.url.clone(),
                    title: if st.title.is_empty() {
                        st.url.clone()
                    } else {
                        st.title.clone()
                    },
                    favicon: None,
                    loading: false,
                    can_go_back: false,
                    can_go_forward: false,
                    pinned: st.pinned,
                    audible: false,
                    state,
                    group_id,
                    has_thumbnail: false,
                    sleep_protected_reason: None,
                    blocked_count: 0,
                };
                self.tabs.insert(tab_id, tab);

                let mut meta = TabMemoryMeta {
                    last_active_ms: now,
                    slept_at_ms: if is_active { None } else { Some(now) },
                    ..Default::default()
                };
                meta.record_navigation(st.url.clone(), st.title.clone());
                self.tab_meta.insert(tab_id, meta);

                tab_ids.push(tab_id);
                if is_active {
                    active_tab_id = tab_id;
                    active_url = st.url.clone();
                }
            }

            self.windows.insert(
                window_id,
                WindowEntry {
                    id: window_id,
                    incognito: false,
                    tab_ids,
                    active_tab_id: Some(active_tab_id),
                    groups: restored_groups,
                    closed_tabs: VecDeque::new(),
                    split_tab_id: None,
                    split_ratio_pct: default_split_ratio(),
                    devtools_open: false,
                    devtools_dock: DevToolsDock::Right,
                    devtools_ratio_pct: default_devtools_ratio(),
                },
            );
            self.window_order.push(window_id);
            spawned.push((window_id, active_tab_id, active_url));
        }

        self.recompute_tab_counts();
        spawned
    }
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Resolves raw user input from the omnibox into a navigable URL or search query URL.
pub fn resolve_omnibox_input(raw: &str, engine: SearchEngine) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return NEWTAB_URL.to_string();
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("seu://")
        || lower.starts_with("about:")
        || lower.starts_with("file://")
    {
        return trimmed.to_string();
    }

    // Localhost with optional port/path
    if !trimmed.contains(' ')
        && (lower == "localhost"
            || lower.starts_with("localhost:")
            || lower.starts_with("localhost/")
            || lower.starts_with("127.0.0.1"))
    {
        return format!("http://{trimmed}");
    }

    // Looks like a domain (has dot, no spaces, non-empty TLD)
    if !trimmed.contains(' ') {
        let host_part = trimmed.split('/').next().unwrap_or("");
        if let Some((_, tld)) = host_part.rsplit_once('.') {
            if tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphanumeric() || c == ':') {
                return format!("https://{trimmed}");
            }
        }
    }

    engine.search_url(trimmed)
}

fn percent_encode_query(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            b => {
                out.push('%');
                out.push(hex_digit(b >> 4));
                out.push(hex_digit(b & 0x0F));
            }
        }
    }
    out
}

fn hex_digit(v: u8) -> char {
    match v {
        0..=9 => (b'0' + v) as char,
        10..=15 => (b'A' + (v - 10)) as char,
        _ => '0',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_urls_and_search_queries() {
        assert_eq!(
            resolve_omnibox_input("https://github.com", SearchEngine::DuckDuckGo),
            "https://github.com"
        );
        assert_eq!(
            resolve_omnibox_input("seu://settings", SearchEngine::DuckDuckGo),
            "seu://settings"
        );
        assert_eq!(
            resolve_omnibox_input("docs.rs/cef", SearchEngine::DuckDuckGo),
            "https://docs.rs/cef"
        );
        assert_eq!(
            resolve_omnibox_input("localhost:5173", SearchEngine::DuckDuckGo),
            "http://localhost:5173"
        );
        assert_eq!(
            resolve_omnibox_input("rust cef alloy", SearchEngine::DuckDuckGo),
            "https://duckduckgo.com/?q=rust+cef+alloy"
        );
    }

    #[test]
    fn manages_tabs_groups_and_reopen_closed() {
        let mut core = BrowserCore::new();
        let (win, tab1) = core.create_window(false, None);
        let tab2 = core
            .create_tab(win, Some("github.com"), true)
            .ok()
            .unwrap_or(0);
        assert_ne!(tab2, 0);

        let gid = core
            .create_tab_group(win, "Dev".to_string(), "#FF2E93".to_string(), &[tab2])
            .ok()
            .unwrap_or(0);
        assert_ne!(gid, 0);
        assert_eq!(core.get_tab(tab2).and_then(|t| t.group_id), Some(gid));

        let collapsed = core.toggle_group_collapse(win, gid).ok().unwrap_or(false);
        assert!(collapsed);

        let (_, next) = core.close_tab(tab2).ok().unwrap_or((0, None));
        assert_eq!(next, Some(tab1));

        let reopened = core.reopen_closed_tab(win).ok().flatten();
        assert!(reopened.is_some());
        let reopened_id = reopened.unwrap_or(0);
        assert_eq!(
            core.get_tab(reopened_id).map(|t| t.url.as_str()),
            Some("https://github.com")
        );
        assert_eq!(
            core.get_tab(reopened_id).and_then(|t| t.group_id),
            Some(gid)
        );
    }

    #[test]
    fn moves_tab_between_windows_and_reorders() {
        let mut core = BrowserCore::new();
        let (w1, t1) = core.create_window(false, None);
        let t2 = core
            .create_tab(w1, Some("youtube.com"), true)
            .ok()
            .unwrap_or(0);
        assert!(core.reorder_tab(w1, t2, 0).is_ok());

        let snap1 = core.window_snapshot(w1).ok();
        assert_eq!(
            snap1
                .as_ref()
                .map(|s| s.tabs.iter().map(|t| t.id).collect::<Vec<_>>()),
            Some(vec![t2, t1])
        );

        let (w2, _) = core.create_window(false, None);
        assert_eq!(core.move_tab_to_window(t2, w2), Ok(w1));
        assert_eq!(core.get_tab(t2).map(|t| t.window_id), Some(w2));
    }

    #[test]
    fn manages_split_view_devtools_and_sleeping_session_restore() {
        let mut core = BrowserCore::new();
        let (w1, t1) = core.create_window(false, Some("https://example.com/"));
        let t2 = core
            .create_tab(w1, Some("https://rust-lang.org/"), false)
            .ok()
            .unwrap_or(0);
        let gid = core
            .create_tab_group(w1, "Work".into(), "#4FD8FF".into(), &[t2])
            .ok()
            .unwrap_or(0);
        assert_ne!(gid, 0);

        // Toggle Split View pairs t2 beside t1 without creating an extra tab.
        let (paired, created_new) = core
            .toggle_split_view(w1, None)
            .ok()
            .unwrap_or((None, true));
        assert_eq!(paired, Some(t2));
        assert!(!created_new);
        assert_eq!(core.set_split_ratio(w1, 62), Ok(62));

        // DevTools dock and ratio
        assert_eq!(core.toggle_devtools(w1), Ok(true));
        assert!(core.set_devtools_dock(w1, DevToolsDock::Bottom).is_ok());
        assert_eq!(core.set_devtools_ratio(w1, 45), Ok(45));

        // Export & restore session: active tab stays Active, inactive tab starts Sleeping!
        let saved = core.export_session();
        assert_eq!(saved.windows.len(), 1);
        assert_eq!(saved.windows[0].tabs.len(), 2);
        assert_eq!(saved.windows[0].groups.len(), 1);

        let mut restored_core = BrowserCore::new();
        let spawned = restored_core.restore_session(&saved);
        assert_eq!(spawned.len(), 1);
        let (rw, r_active, r_url) = &spawned[0];
        assert_eq!(r_url, "https://example.com/");
        let snap = restored_core.window_snapshot(*rw).ok();
        assert!(snap.is_some());
        if let Some(s) = snap {
            assert_eq!(s.tabs.len(), 2);
            assert_eq!(s.groups.len(), 1);
            let active_tab = s.tabs.iter().find(|t| t.id == *r_active);
            let sleeping_tab = s.tabs.iter().find(|t| t.id != *r_active);
            assert_eq!(active_tab.map(|t| t.state), Some(TabLifecycleState::Active));
            assert_eq!(
                sleeping_tab.map(|t| t.state),
                Some(TabLifecycleState::Sleeping)
            );
            assert_eq!(t1, 1);
        }
    }
}
