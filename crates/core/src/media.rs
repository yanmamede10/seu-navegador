//! Platform-agnostic "Now Playing" media session state & control commands (SPEC §10).

use crate::{TabId, WindowId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaCommand {
    PlayPause,
    Next,
    Prev,
    FocusTab,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaSessionInfo {
    pub tab_id: TabId,
    pub window_id: WindowId,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artwork: Option<String>,
    pub source: String,
    pub playing: bool,
    pub has_prev: bool,
    pub has_next: bool,
    /// Progress percentage `0..=100` (integer so struct cleanly derives `Eq`).
    pub progress_pct: u8,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NowPlayingState {
    pub active: Option<MediaSessionInfo>,
    pub sessions: Vec<MediaSessionInfo>,
}

impl NowPlayingState {
    pub fn from_sessions(mut sessions: Vec<MediaSessionInfo>) -> Self {
        // Prefer currently playing sessions first, then most recently updated.
        sessions.sort_by(|a, b| {
            b.playing
                .cmp(&a.playing)
                .then_with(|| b.updated_at_ms.cmp(&a.updated_at_ms))
        });
        let active = sessions.first().cloned();
        Self { active, sessions }
    }
}

/// Sanitizes an artwork URL so only safe `https://`, `http://`, or `data:image/` URIs
/// under 8 KB are stored.
pub fn sanitize_artwork_url(raw: Option<&str>) -> Option<String> {
    let s = raw?.trim();
    if s.is_empty() || s.len() > 8192 {
        return None;
    }
    if s.starts_with("https://") || s.starts_with("http://") || s.starts_with("data:image/") {
        Some(s.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prioritizes_playing_session_over_paused() {
        let paused = MediaSessionInfo {
            tab_id: 1,
            window_id: 1,
            title: "Paused Video".into(),
            artist: "Channel A".into(),
            album: String::new(),
            artwork: None,
            source: "youtube.com".into(),
            playing: false,
            has_prev: false,
            has_next: true,
            progress_pct: 50,
            updated_at_ms: 9_000,
        };
        let playing = MediaSessionInfo {
            tab_id: 2,
            window_id: 1,
            title: "Live Track".into(),
            artist: "Channel B".into(),
            album: String::new(),
            artwork: sanitize_artwork_url(Some("https://i.ytimg.com/vi/abc/default.jpg")),
            source: "youtube.com".into(),
            playing: true,
            has_prev: true,
            has_next: true,
            progress_pct: 25,
            updated_at_ms: 5_000,
        };

        let np = NowPlayingState::from_sessions(vec![paused, playing]);
        assert_eq!(np.active.as_ref().map(|s| s.tab_id), Some(2));
        assert_eq!(np.sessions.len(), 2);
        assert_eq!(sanitize_artwork_url(Some("javascript:alert(1)")), None);
    }
}
