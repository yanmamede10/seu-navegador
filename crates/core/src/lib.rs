//! Core platform-agnostic browser state: windows, tabs, tab groups,
//! omnibox URL resolution, and locale/search settings.
//!
//! By architectural contract (SPEC §12.1), this crate MUST NOT depend on CEF
//! or Win32 so it can be shared with Android via UniFFI in the future.

pub mod shortcuts;

use serde::{Deserialize, Serialize};
use shortcuts::{
    default_chrome_shortcuts, default_quick_shortcuts, merge_with_defaults, KeyCombo,
    QuickShortcut, ShortcutAction,
};
use std::collections::{BTreeMap, HashMap, VecDeque};
use thiserror::Error;

pub const NEWTAB_URL: &str = "seu://newtab/";
pub const SHELL_URL: &str = "seu://shell/";
pub const OVERLAY_URL: &str = "seu://overlay/";

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedTabRecord {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub group_id: Option<GroupId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSnapshot {
    pub id: WindowId,
    pub incognito: bool,
    pub active_tab_id: Option<TabId>,
    pub tabs: Vec<TabInfo>,
    pub groups: Vec<TabGroup>,
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
        }
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
            .filter_map(|id| self.tabs.get(id).cloned())
            .collect();
        Ok(WindowSnapshot {
            id: win.id,
            incognito: win.incognito,
            active_tab_id: win.active_tab_id,
            tabs,
            groups: win.groups.clone(),
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
            url: resolved_url,
            title,
            favicon: None,
            loading: false,
            can_go_back: false,
            can_go_forward: false,
            pinned: false,
            audible: false,
            state: TabLifecycleState::Background,
            group_id: None,
        };
        self.tabs.insert(tab_id, tab);

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

        Ok(tab_id)
    }

    pub fn activate_tab(&mut self, window_id: WindowId, tab_id: TabId) -> Result<(), CoreError> {
        let win = self
            .windows
            .get_mut(&window_id)
            .ok_or(CoreError::WindowNotFound(window_id))?;

        if !win.tab_ids.contains(&tab_id) {
            return Err(CoreError::TabNotFound(tab_id));
        }

        let prev_active = win.active_tab_id;
        win.active_tab_id = Some(tab_id);

        if let Some(prev_id) = prev_active {
            if prev_id != tab_id {
                if let Some(prev_tab) = self.tabs.get_mut(&prev_id) {
                    if prev_tab.state == TabLifecycleState::Active {
                        prev_tab.state = TabLifecycleState::Background;
                    }
                }
            }
        }

        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.state = TabLifecycleState::Active;
        }
        Ok(())
    }

    pub fn close_tab(&mut self, tab_id: TabId) -> Result<(WindowId, Option<TabId>), CoreError> {
        let tab = self
            .tabs
            .remove(&tab_id)
            .ok_or(CoreError::TabNotFound(tab_id))?;
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
                None
            } else {
                let next_idx = pos.min(win.tab_ids.len() - 1);
                let next_id = win.tab_ids[next_idx];
                win.active_tab_id = Some(next_id);
                if let Some(next_tab) = self.tabs.get_mut(&next_id) {
                    next_tab.state = TabLifecycleState::Active;
                }
                Some(next_id)
            }
        } else {
            win.active_tab_id
        };

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
        if let Some(u) = url {
            tab.url = u;
        }
        if let Some(t) = title {
            if !t.is_empty() {
                tab.title = t;
            }
        }
        if let Some(l) = loading {
            tab.loading = l;
        }
        if let Some(b) = can_go_back {
            tab.can_go_back = b;
        }
        if let Some(f) = can_go_forward {
            tab.can_go_forward = f;
        }
        Ok(tab.window_id)
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
        self.settings.game_mode_enabled = !self.settings.game_mode_enabled;
        self.settings.game_mode_enabled
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
}
