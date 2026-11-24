//! Ad & tracker blocker foundation (Phase 1 skeleton; full Brave `adblock`
//! engine + EasyList/EasyPrivacy/uBlock Filters + scriptlets added in Phase 4).

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockerStats {
    pub enabled: bool,
    pub total_blocked: u64,
    pub allowlisted_domains: Vec<String>,
}

#[derive(Debug, Default)]
pub struct BlockerService {
    enabled: bool,
    total_blocked: u64,
    allowlist: HashSet<String>,
}

impl BlockerService {
    pub fn new() -> Self {
        Self {
            enabled: true,
            total_blocked: 0,
            allowlist: HashSet::new(),
        }
    }

    pub fn is_enabled_for(&self, domain: &str) -> bool {
        self.enabled && !self.allowlist.contains(&domain.to_ascii_lowercase())
    }

    pub fn toggle_for_site(&mut self, domain: &str) -> bool {
        let key = domain.to_ascii_lowercase();
        if self.allowlist.contains(&key) {
            self.allowlist.remove(&key);
            true
        } else {
            self.allowlist.insert(key);
            false
        }
    }

    pub fn record_blocked(&mut self) {
        self.total_blocked += 1;
    }

    pub fn stats(&self) -> BlockerStats {
        let mut domains: Vec<String> = self.allowlist.iter().cloned().collect();
        domains.sort();
        BlockerStats {
            enabled: self.enabled,
            total_blocked: self.total_blocked,
            allowlisted_domains: domains,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_site_allowlist() {
        let mut blocker = BlockerService::new();
        assert!(blocker.is_enabled_for("youtube.com"));
        assert!(!blocker.toggle_for_site("YouTube.com"));
        assert!(!blocker.is_enabled_for("youtube.com"));
        assert!(blocker.toggle_for_site("youtube.com"));
        assert!(blocker.is_enabled_for("youtube.com"));
    }
}
