//! Platform-agnostic tab memory lifecycle, sleep policies, Back/Forward stack
//! preservation across sleep/wake, and realistic multi-process RAM accounting.
//!
//! Uses injectable timestamps (`now_ms: u64`) so all timeout and LRU policies
//! are deterministically unit-testable without real wall-clock sleeps.

use crate::{TabId, TabInfo, TabLifecycleState};
use serde::{Deserialize, Serialize};

pub const DEFAULT_SLEEP_TIMEOUT_SECS: u64 = 10 * 60; // 10 minutes (SPEC §5.2)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryPolicy {
    /// Seconds a background tab remains idle before sleeping (`0` disables auto-sleep).
    pub sleep_timeout_secs: u64,
    /// Optional cap on simultaneously live tabs (evicts LRU eligible background tabs).
    pub max_live_tabs: Option<usize>,
    /// When true, pinned tabs are exempt from automatic sleeping.
    pub never_sleep_pinned: bool,
    /// Hostnames (exact or parent domain) that never sleep automatically.
    pub never_sleep_domains: Vec<String>,
}

impl Default for MemoryPolicy {
    fn default() -> Self {
        Self {
            sleep_timeout_secs: DEFAULT_SLEEP_TIMEOUT_SECS,
            max_live_tabs: None,
            never_sleep_pinned: true,
            never_sleep_domains: vec![
                "meet.google.com".into(),
                "discord.com".into(),
                "web.whatsapp.com".into(),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabHistoryEntry {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TabMemoryMeta {
    pub last_active_ms: u64,
    pub dirty_form: bool,
    pub has_beforeunload: bool,
    pub media_capture_active: bool,
    pub active_transfer: bool,
    pub scroll_x: i32,
    pub scroll_y: i32,
    pub has_thumbnail: bool,
    pub history_stack: Vec<TabHistoryEntry>,
    pub history_index: usize,
    pub slept_at_ms: Option<u64>,
    pub saved_mb: u32,
}

impl TabMemoryMeta {
    pub fn record_navigation(&mut self, url: String, title: String) {
        if url.trim().is_empty() {
            return;
        }
        if let Some(cur) = self.history_stack.get_mut(self.history_index) {
            if cur.url == url {
                if !title.is_empty() {
                    cur.title = title;
                }
                return;
            }
            // Discard forward entries on new navigation (standard browser stack).
            self.history_stack.truncate(self.history_index + 1);
        }
        self.history_stack.push(TabHistoryEntry { url, title });
        self.history_index = self.history_stack.len().saturating_sub(1);
    }

    pub fn update_current_title(&mut self, title: &str) {
        if title.is_empty() {
            return;
        }
        if let Some(cur) = self.history_stack.get_mut(self.history_index) {
            cur.title = title.to_string();
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.history_index > 0 && !self.history_stack.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        self.history_index + 1 < self.history_stack.len()
    }

    pub fn step_back(&mut self) -> Option<&TabHistoryEntry> {
        if self.history_index == 0 || self.history_stack.is_empty() {
            return None;
        }
        self.history_index -= 1;
        self.history_stack.get(self.history_index)
    }

    pub fn step_forward(&mut self) -> Option<&TabHistoryEntry> {
        if self.history_index + 1 >= self.history_stack.len() {
            return None;
        }
        self.history_index += 1;
        self.history_stack.get(self.history_index)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SleepBlockReason {
    Active,
    AlreadySleeping,
    InternalPage,
    Audible,
    PinnedProtected,
    DirtyForm,
    BeforeUnload,
    ActiveMediaOrTransfer,
    NeverSleepDomain,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MemoryStats {
    /// Total Private Bytes across ALL Seu Navegador processes (browser + GPU + utility + renderers).
    pub total_private_mb: u32,
    /// Total Working Set across ALL Seu Navegador processes.
    pub total_working_set_mb: u32,
    /// Total OS processes belonging to this Seu Navegador instance.
    pub process_count: usize,
    pub live_tabs_count: usize,
    pub sleeping_tabs_count: usize,
    /// Cumulative MB returned to the OS measured from real before/after sleep deltas.
    pub saved_mb: u32,
    /// True when the last tab sleep shared a renderer process with another live tab,
    /// so OS Private Bytes did not drop immediately until all tabs in that renderer sleep.
    pub shared_renderer_detected: bool,
}

/// Evaluates whether a tab is eligible to sleep under SPEC §5.2 protection rules.
pub fn evaluate_sleep_eligibility(
    tab: &TabInfo,
    meta: &TabMemoryMeta,
    policy: &MemoryPolicy,
) -> Result<(), SleepBlockReason> {
    if tab.state == TabLifecycleState::Active {
        return Err(SleepBlockReason::Active);
    }
    if tab.state == TabLifecycleState::Sleeping {
        return Err(SleepBlockReason::AlreadySleeping);
    }
    if tab.url.starts_with("seu://") || tab.url.starts_with("about:") {
        return Err(SleepBlockReason::InternalPage);
    }
    if tab.audible {
        return Err(SleepBlockReason::Audible);
    }
    if tab.pinned && policy.never_sleep_pinned {
        return Err(SleepBlockReason::PinnedProtected);
    }
    if meta.dirty_form {
        return Err(SleepBlockReason::DirtyForm);
    }
    if meta.has_beforeunload {
        return Err(SleepBlockReason::BeforeUnload);
    }
    if meta.media_capture_active || meta.active_transfer {
        return Err(SleepBlockReason::ActiveMediaOrTransfer);
    }
    if is_domain_protected(&tab.url, &policy.never_sleep_domains) {
        return Err(SleepBlockReason::NeverSleepDomain);
    }
    Ok(())
}

/// Selects tabs that should be put to sleep at `now_ms`.
///
/// - When `force_all_eligible` is true (manual "Sleep inactive tabs now" or Game Mode),
///   every eligible background tab is returned, oldest first.
/// - Otherwise returns tabs exceeding `policy.sleep_timeout_secs`, plus any additional
///   LRU eligible background tabs required to stay within `policy.max_live_tabs`.
pub fn select_tabs_to_sleep(
    tabs: &[(&TabInfo, &TabMemoryMeta)],
    policy: &MemoryPolicy,
    now_ms: u64,
    force_all_eligible: bool,
) -> Vec<TabId> {
    let mut eligible: Vec<(TabId, u64)> = tabs
        .iter()
        .filter(|(tab, meta)| evaluate_sleep_eligibility(tab, meta, policy).is_ok())
        .map(|(tab, meta)| (tab.id, meta.last_active_ms))
        .collect();

    // Sort least-recently-used (oldest `last_active_ms`) first.
    eligible.sort_by_key(|&(_, last_active)| last_active);

    if force_all_eligible {
        return eligible.into_iter().map(|(id, _)| id).collect();
    }

    let mut to_sleep: Vec<TabId> = Vec::new();
    let timeout_ms = policy.sleep_timeout_secs.saturating_mul(1000);

    if timeout_ms > 0 {
        for &(id, last_active) in &eligible {
            if now_ms.saturating_sub(last_active) >= timeout_ms {
                to_sleep.push(id);
            }
        }
    }

    if let Some(cap) = policy.max_live_tabs {
        let live_count = tabs
            .iter()
            .filter(|(t, _)| t.state != TabLifecycleState::Sleeping)
            .count();
        if live_count.saturating_sub(to_sleep.len()) > cap {
            for &(id, _) in &eligible {
                if !to_sleep.contains(&id) {
                    to_sleep.push(id);
                    if live_count.saturating_sub(to_sleep.len()) <= cap {
                        break;
                    }
                }
            }
        }
    }

    to_sleep
}

pub fn is_domain_protected(url: &str, protected_domains: &[String]) -> bool {
    let Some(host) = extract_hostname(url) else {
        return false;
    };
    protected_domains.iter().any(|entry| {
        let clean = entry.trim().to_ascii_lowercase();
        if clean.is_empty() {
            return false;
        }
        host == clean || host.ends_with(&format!(".{clean}"))
    })
}

fn extract_hostname(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = after_scheme.split(['/', '?', '#']).next()?;
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    let host = host_port.split(':').next()?.trim().to_ascii_lowercase();
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tab(id: TabId, url: &str, state: TabLifecycleState) -> TabInfo {
        TabInfo {
            id,
            window_id: 1,
            url: url.into(),
            title: format!("Tab {id}"),
            favicon: None,
            loading: false,
            can_go_back: false,
            can_go_forward: false,
            pinned: false,
            audible: false,
            state,
            group_id: None,
            has_thumbnail: false,
            sleep_protected_reason: None,
        }
    }

    #[test]
    fn enforces_all_never_sleep_protections() {
        let policy = MemoryPolicy::default();
        let meta = TabMemoryMeta::default();

        let active = sample_tab(1, "https://example.com", TabLifecycleState::Active);
        assert_eq!(
            evaluate_sleep_eligibility(&active, &meta, &policy),
            Err(SleepBlockReason::Active)
        );

        let mut audible = sample_tab(
            2,
            "https://youtube.com/watch?v=1",
            TabLifecycleState::Background,
        );
        audible.audible = true;
        assert_eq!(
            evaluate_sleep_eligibility(&audible, &meta, &policy),
            Err(SleepBlockReason::Audible)
        );

        let mut pinned = sample_tab(3, "https://github.com", TabLifecycleState::Background);
        pinned.pinned = true;
        assert_eq!(
            evaluate_sleep_eligibility(&pinned, &meta, &policy),
            Err(SleepBlockReason::PinnedProtected)
        );

        let bg = sample_tab(4, "https://docs.rs", TabLifecycleState::Background);
        let dirty_meta = TabMemoryMeta {
            dirty_form: true,
            ..Default::default()
        };
        assert_eq!(
            evaluate_sleep_eligibility(&bg, &dirty_meta, &policy),
            Err(SleepBlockReason::DirtyForm)
        );

        let beforeunload_meta = TabMemoryMeta {
            has_beforeunload: true,
            ..Default::default()
        };
        assert_eq!(
            evaluate_sleep_eligibility(&bg, &beforeunload_meta, &policy),
            Err(SleepBlockReason::BeforeUnload)
        );

        let call_meta = TabMemoryMeta {
            media_capture_active: true,
            ..Default::default()
        };
        assert_eq!(
            evaluate_sleep_eligibility(&bg, &call_meta, &policy),
            Err(SleepBlockReason::ActiveMediaOrTransfer)
        );

        let discord = sample_tab(
            5,
            "https://ptb.discord.com/channels/1",
            TabLifecycleState::Background,
        );
        assert_eq!(
            evaluate_sleep_eligibility(&discord, &meta, &policy),
            Err(SleepBlockReason::NeverSleepDomain)
        );

        assert_eq!(evaluate_sleep_eligibility(&bg, &meta, &policy), Ok(()));
    }

    #[test]
    fn selects_idle_and_lru_tabs_deterministically_with_injectable_clock() {
        let policy = MemoryPolicy {
            sleep_timeout_secs: 600,
            max_live_tabs: Some(2),
            never_sleep_pinned: true,
            never_sleep_domains: vec![],
        };

        let t1 = sample_tab(1, "https://a.example", TabLifecycleState::Active);
        let m1 = TabMemoryMeta {
            last_active_ms: 1_000_000,
            ..Default::default()
        };
        let t2 = sample_tab(2, "https://b.example", TabLifecycleState::Background);
        let m2 = TabMemoryMeta {
            last_active_ms: 200_000, // 800s idle -> past 600s timeout
            ..Default::default()
        };
        let t3 = sample_tab(3, "https://c.example", TabLifecycleState::Background);
        let m3 = TabMemoryMeta {
            last_active_ms: 850_000, // 150s idle -> evicted by max_live_tabs=2 (LRU)
            ..Default::default()
        };
        let t4 = sample_tab(4, "https://d.example", TabLifecycleState::Background);
        let m4 = TabMemoryMeta {
            last_active_ms: 950_000, // 50s idle -> kept live
            ..Default::default()
        };

        let list = [(&t1, &m1), (&t2, &m2), (&t3, &m3), (&t4, &m4)];
        let sleep_ids = select_tabs_to_sleep(&list, &policy, 1_000_000, false);
        assert_eq!(sleep_ids, vec![2, 3]);
    }

    #[test]
    fn preserves_back_forward_history_stack_across_sleep() {
        let mut meta = TabMemoryMeta::default();
        meta.record_navigation("https://one.test".into(), "One".into());
        meta.record_navigation("https://two.test".into(), "Two".into());
        meta.record_navigation("https://three.test".into(), "Three".into());
        assert!(meta.can_go_back());
        assert!(!meta.can_go_forward());

        assert_eq!(
            meta.step_back().map(|e| e.url.as_str()),
            Some("https://two.test")
        );
        assert!(meta.can_go_back());
        assert!(meta.can_go_forward());

        // Navigating to a new page truncates forward history.
        meta.record_navigation("https://four.test".into(), "Four".into());
        assert!(!meta.can_go_forward());
        assert_eq!(meta.history_stack.len(), 3);
        assert_eq!(
            meta.step_back().map(|e| e.url.as_str()),
            Some("https://two.test")
        );
    }
}
