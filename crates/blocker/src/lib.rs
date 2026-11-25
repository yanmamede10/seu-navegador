//! Ad & tracker blocker powered by Brave's `adblock` Rust engine.
//!
//! Ships with an embedded offline baseline (EasyList + EasyPrivacy + uBlock Filters
//! + YouTube scriptlets) and supports loading cached upstream filter lists updated
//! in the background (max 1x/day, SPEC §8).

pub mod scriptlets;

use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request as AdblockRequest;
use adblock::Engine as AdblockEngine;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const EMBEDDED_FILTER_LIST: &str = include_str!("../lists/embedded-filters.txt");
const MAX_RECENT_BLOCKED: usize = 30;
const UPDATE_INTERVAL_SECS: u64 = 24 * 60 * 60;

pub const UPSTREAM_LIST_URLS: &[(&str, &str)] = &[
    ("easylist.txt", "https://easylist.to/easylist/easylist.txt"),
    (
        "easyprivacy.txt",
        "https://easylist.to/easylist/easyprivacy.txt",
    ),
    (
        "ublock-filters.txt",
        "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters.txt",
    ),
    (
        "ublock-quick-fixes.txt",
        "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/quick-fixes.txt",
    ),
];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockerStats {
    pub enabled: bool,
    pub total_blocked: u64,
    pub allowlisted_domains: Vec<String>,
    pub recent_blocked: Vec<String>,
    pub lists_loaded: usize,
    pub last_updated_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageCosmeticPayload {
    pub css: String,
    pub script: String,
}

pub struct BlockerService {
    engine: AdblockEngine,
    enabled: bool,
    total_blocked: u64,
    allowlist: HashSet<String>,
    recent_blocked: VecDeque<String>,
    lists_loaded: usize,
    last_updated_secs: Option<u64>,
}

impl Default for BlockerService {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockerService {
    pub fn new() -> Self {
        let engine = build_engine(&[EMBEDDED_FILTER_LIST]);
        Self {
            engine,
            enabled: true,
            total_blocked: 0,
            allowlist: HashSet::new(),
            recent_blocked: VecDeque::new(),
            lists_loaded: 1,
            last_updated_secs: None,
        }
    }

    pub fn configure(&mut self, enabled: bool, allowlisted_domains: &[String]) {
        self.enabled = enabled;
        self.allowlist = allowlisted_domains
            .iter()
            .map(|d| normalize_host(d))
            .filter(|d| !d.is_empty())
            .collect();
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_enabled_for(&self, domain_or_url: &str) -> bool {
        if !self.enabled {
            return false;
        }
        let host = extract_host(domain_or_url);
        if host.is_empty() {
            return true;
        }
        !self
            .allowlist
            .iter()
            .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
    }

    /// Toggles site protection (returns `true` if protection is now ON for this site,
    /// `false` if the site was added to the allowlist).
    pub fn toggle_for_site(&mut self, domain_or_url: &str) -> bool {
        let key = extract_host(domain_or_url);
        if key.is_empty() {
            return self.enabled;
        }
        if self.allowlist.contains(&key) {
            self.allowlist.remove(&key);
            true
        } else {
            self.allowlist.insert(key);
            false
        }
    }

    pub fn allowlisted_domains(&self) -> Vec<String> {
        let mut domains: Vec<String> = self.allowlist.iter().cloned().collect();
        domains.sort();
        domains
    }

    /// Evaluates whether a network request should be blocked.
    pub fn should_block_request(
        &self,
        url: &str,
        source_url: &str,
        request_type: &str,
        method: &str,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        // Never block internal `seu://` pages or main document top-level navigations.
        if url.starts_with("seu://") || source_url.starts_with("seu://") {
            return false;
        }
        if request_type == "main_frame" || request_type == "document" {
            return false;
        }
        if !source_url.is_empty() && !self.is_enabled_for(source_url) {
            return false;
        }

        let effective_source = if source_url.is_empty() {
            url
        } else {
            source_url
        };

        let Ok(req) = AdblockRequest::new(url, effective_source, request_type, method) else {
            return false;
        };
        self.engine.check_network_request(&req).should_block()
    }

    pub fn record_blocked(&mut self, blocked_url: &str) {
        self.total_blocked = self.total_blocked.saturating_add(1);
        let host = extract_host(blocked_url);
        if !host.is_empty() {
            if self.recent_blocked.front().map(String::as_str) != Some(host.as_str()) {
                self.recent_blocked.push_front(host);
                while self.recent_blocked.len() > MAX_RECENT_BLOCKED {
                    self.recent_blocked.pop_back();
                }
            }
        }
    }

    /// Computes cosmetic hide CSS + injected scriptlets (`##+js`) for a page URL.
    pub fn cosmetic_resources_for_url(&self, page_url: &str) -> PageCosmeticPayload {
        if !self.enabled || page_url.starts_with("seu://") || !self.is_enabled_for(page_url) {
            return PageCosmeticPayload::default();
        }
        let res = self.engine.url_cosmetic_resources(page_url);
        let mut css = String::new();
        if !res.hide_selectors.is_empty() {
            let selectors: Vec<&str> = res.hide_selectors.iter().map(String::as_str).collect();
            for chunk in selectors.chunks(64) {
                css.push_str(&chunk.join(","));
                css.push_str("{display:none!important;}\n");
            }
        }
        PageCosmeticPayload {
            css,
            script: res.injected_script,
        }
    }

    /// Loads any cached upstream filter lists from `lists_dir` alongside the embedded baseline.
    pub fn reload_from_cache_dir(&mut self, lists_dir: &Path) -> usize {
        let mut loaded_texts: Vec<String> = Vec::new();
        for (filename, _) in UPSTREAM_LIST_URLS {
            let p = lists_dir.join(filename);
            if let Ok(text) = std::fs::read_to_string(&p) {
                if !text.trim().is_empty() {
                    loaded_texts.push(text);
                }
            }
        }

        let mut refs: Vec<&str> = vec![EMBEDDED_FILTER_LIST];
        for t in &loaded_texts {
            refs.push(t.as_str());
        }
        self.engine = build_engine(&refs);
        self.lists_loaded = refs.len();
        self.last_updated_secs = read_last_updated(lists_dir);
        self.lists_loaded
    }

    pub fn stats(&self) -> BlockerStats {
        BlockerStats {
            enabled: self.enabled,
            total_blocked: self.total_blocked,
            allowlisted_domains: self.allowlisted_domains(),
            recent_blocked: self.recent_blocked.iter().cloned().collect(),
            lists_loaded: self.lists_loaded,
            last_updated_secs: self.last_updated_secs,
        }
    }
}

fn build_engine(lists: &[&str]) -> AdblockEngine {
    let mut filter_set = FilterSet::new(false);
    for list in lists {
        filter_set.add_filter_list(*list, ParseOptions::default());
    }
    let mut engine = AdblockEngine::new_with_filter_set(filter_set);
    engine.use_resources(scriptlets::builtin_scriptlet_resources());
    engine
}

pub fn filter_lists_dir() -> PathBuf {
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = root.join("SeuNavegador").join("filter-lists");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn needs_background_update(lists_dir: &Path) -> bool {
    let Some(last) = read_last_updated(lists_dir) else {
        return true;
    };
    let now = unix_now_secs();
    now.saturating_sub(last) >= UPDATE_INTERVAL_SECS
}

pub fn write_last_updated(lists_dir: &Path) {
    let _ = std::fs::create_dir_all(lists_dir);
    let _ = std::fs::write(
        lists_dir.join("last_updated.txt"),
        unix_now_secs().to_string(),
    );
}

fn read_last_updated(lists_dir: &Path) -> Option<u64> {
    std::fs::read_to_string(lists_dir.join("last_updated.txt"))
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

pub fn extract_host(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let after_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    let host = host_port.split(':').next().unwrap_or("");
    normalize_host(host)
}

fn normalize_host(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .trim_start_matches("www.")
        .to_string()
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_ads_trackers_and_youtube_ads_while_allowing_content() {
        let blocker = BlockerService::new();

        // EasyList / EasyPrivacy network blocking
        assert!(blocker.should_block_request(
            "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js",
            "https://news.example.com/",
            "script",
            "get"
        ));
        assert!(blocker.should_block_request(
            "https://www.google-analytics.com/analytics.js",
            "https://news.example.com/",
            "script",
            "get"
        ));
        assert!(blocker.should_block_request(
            "https://www.youtube.com/pagead/adview?ai=123",
            "https://www.youtube.com/watch?v=abc",
            "xmlhttprequest",
            "get"
        ));

        // Legitimate content & internal pages are never blocked
        assert!(!blocker.should_block_request(
            "https://github.com/yanmamede10/seu-navegador",
            "https://github.com/",
            "document",
            "get"
        ));
        assert!(!blocker.should_block_request(
            "https://www.youtube.com/s/player/main.js",
            "https://www.youtube.com/watch?v=abc",
            "script",
            "get"
        ));
        assert!(!blocker.should_block_request(
            "seu://shell/assets/shell.js",
            "seu://shell/",
            "script",
            "get"
        ));
    }

    #[test]
    fn generates_youtube_cosmetic_css_and_scriptlets() {
        let blocker = BlockerService::new();
        let cos = blocker.cosmetic_resources_for_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        assert!(cos.css.contains("ytd-ad-slot-renderer"));
        assert!(cos.css.contains("#player-ads"));
        assert!(
            cos.script.contains("adPlacements") || cos.script.contains("__seuYtAdSkipInstalled")
        );
    }

    #[test]
    fn toggles_site_allowlist_and_disables_blocking_for_allowlisted_domain() {
        let mut blocker = BlockerService::new();
        assert!(blocker.is_enabled_for("https://www.youtube.com/watch?v=1"));
        assert!(!blocker.toggle_for_site("https://www.youtube.com/watch?v=1"));
        assert!(!blocker.is_enabled_for("https://www.youtube.com/watch?v=1"));
        assert!(!blocker.should_block_request(
            "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js",
            "https://www.youtube.com/watch?v=1",
            "script",
            "get"
        ));
        assert!(blocker.toggle_for_site("youtube.com"));
        assert!(blocker.is_enabled_for("https://www.youtube.com/watch?v=1"));
    }
}
