//! Customizable Chrome-compatible keyboard shortcuts and New Tab quick shortcuts.
//!
//! Platform-agnostic (no CEF/Win32 dependencies) so the same shortcut definitions
//! and canonical key parsing can be shared across desktop and future platforms.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickShortcut {
    pub id: String,
    pub code: String,
    pub name: String,
    pub url: String,
}

pub fn default_quick_shortcuts() -> Vec<QuickShortcut> {
    vec![
        QuickShortcut {
            id: "yt".into(),
            code: "YT".into(),
            name: "YouTube".into(),
            url: "https://www.youtube.com".into(),
        },
        QuickShortcut {
            id: "gh".into(),
            code: "GH".into(),
            name: "GitHub".into(),
            url: "https://github.com".into(),
        },
        QuickShortcut {
            id: "rs".into(),
            code: "RS".into(),
            name: "docs.rs".into(),
            url: "https://docs.rs".into(),
        },
        QuickShortcut {
            id: "gd".into(),
            code: "GD".into(),
            name: "Godot".into(),
            url: "https://godotengine.org".into(),
        },
        QuickShortcut {
            id: "io".into(),
            code: "IO".into(),
            name: "itch.io".into(),
            url: "https://itch.io".into(),
        },
        QuickShortcut {
            id: "st".into(),
            code: "ST".into(),
            name: "Steam".into(),
            url: "https://store.steampowered.com".into(),
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShortcutAction {
    NewTab,
    CloseTab,
    ReopenClosedTab,
    NextTab,
    PrevTab,
    Reload,
    HardReload,
    Back,
    Forward,
    FocusOmnibox,
    NewWindow,
    NewIncognitoWindow,
    PinTab,
    DuplicateTab,
    ToggleMenu,
    OpenSettings,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    SwitchTab1,
    SwitchTab2,
    SwitchTab3,
    SwitchTab4,
    SwitchTab5,
    SwitchTab6,
    SwitchTab7,
    SwitchTab8,
    SwitchTabLast,
    ToggleDevTools,
    BookmarkPage,
    OpenHistory,
    OpenDownloads,
    ToggleSplitView,
    ToggleStudio,
}

impl ShortcutAction {
    pub fn as_id(self) -> &'static str {
        match self {
            Self::NewTab => "tab.new",
            Self::CloseTab => "tab.close",
            Self::ReopenClosedTab => "tab.reopenClosed",
            Self::NextTab => "tab.next",
            Self::PrevTab => "tab.prev",
            Self::Reload => "tab.reload",
            Self::HardReload => "tab.hardReload",
            Self::Back => "nav.back",
            Self::Forward => "nav.forward",
            Self::FocusOmnibox => "nav.focusOmnibox",
            Self::NewWindow => "window.new",
            Self::NewIncognitoWindow => "window.newIncognito",
            Self::PinTab => "tab.pin",
            Self::DuplicateTab => "tab.duplicate",
            Self::ToggleMenu => "overlay.toggleMenu",
            Self::OpenSettings => "nav.openSettings",
            Self::ZoomIn => "zoom.in",
            Self::ZoomOut => "zoom.out",
            Self::ZoomReset => "zoom.reset",
            Self::SwitchTab1 => "tab.switch1",
            Self::SwitchTab2 => "tab.switch2",
            Self::SwitchTab3 => "tab.switch3",
            Self::SwitchTab4 => "tab.switch4",
            Self::SwitchTab5 => "tab.switch5",
            Self::SwitchTab6 => "tab.switch6",
            Self::SwitchTab7 => "tab.switch7",
            Self::SwitchTab8 => "tab.switch8",
            Self::SwitchTabLast => "tab.switchLast",
            Self::ToggleDevTools => "devtools.toggle",
            Self::BookmarkPage => "bookmarks.toggle",
            Self::OpenHistory => "nav.openHistory",
            Self::OpenDownloads => "nav.openDownloads",
            Self::ToggleSplitView => "split.toggle",
            Self::ToggleStudio => "studio.toggle",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "tab.new" => Some(Self::NewTab),
            "tab.close" => Some(Self::CloseTab),
            "tab.reopenClosed" => Some(Self::ReopenClosedTab),
            "tab.next" => Some(Self::NextTab),
            "tab.prev" => Some(Self::PrevTab),
            "tab.reload" => Some(Self::Reload),
            "tab.hardReload" => Some(Self::HardReload),
            "nav.back" => Some(Self::Back),
            "nav.forward" => Some(Self::Forward),
            "nav.focusOmnibox" => Some(Self::FocusOmnibox),
            "window.new" => Some(Self::NewWindow),
            "window.newIncognito" => Some(Self::NewIncognitoWindow),
            "tab.pin" => Some(Self::PinTab),
            "tab.duplicate" => Some(Self::DuplicateTab),
            "overlay.toggleMenu" => Some(Self::ToggleMenu),
            "nav.openSettings" => Some(Self::OpenSettings),
            "zoom.in" => Some(Self::ZoomIn),
            "zoom.out" => Some(Self::ZoomOut),
            "zoom.reset" => Some(Self::ZoomReset),
            "tab.switch1" => Some(Self::SwitchTab1),
            "tab.switch2" => Some(Self::SwitchTab2),
            "tab.switch3" => Some(Self::SwitchTab3),
            "tab.switch4" => Some(Self::SwitchTab4),
            "tab.switch5" => Some(Self::SwitchTab5),
            "tab.switch6" => Some(Self::SwitchTab6),
            "tab.switch7" => Some(Self::SwitchTab7),
            "tab.switch8" => Some(Self::SwitchTab8),
            "tab.switchLast" => Some(Self::SwitchTabLast),
            "devtools.toggle" => Some(Self::ToggleDevTools),
            "bookmarks.toggle" => Some(Self::BookmarkPage),
            "nav.openHistory" => Some(Self::OpenHistory),
            "nav.openDownloads" => Some(Self::OpenDownloads),
            "split.toggle" => Some(Self::ToggleSplitView),
            "studio.toggle" => Some(Self::ToggleStudio),
            _ => None,
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::NewTab,
            Self::CloseTab,
            Self::ReopenClosedTab,
            Self::NextTab,
            Self::PrevTab,
            Self::Reload,
            Self::HardReload,
            Self::Back,
            Self::Forward,
            Self::FocusOmnibox,
            Self::NewWindow,
            Self::NewIncognitoWindow,
            Self::PinTab,
            Self::DuplicateTab,
            Self::ToggleMenu,
            Self::OpenSettings,
            Self::ZoomIn,
            Self::ZoomOut,
            Self::ZoomReset,
            Self::SwitchTab1,
            Self::SwitchTab2,
            Self::SwitchTab3,
            Self::SwitchTab4,
            Self::SwitchTab5,
            Self::SwitchTab6,
            Self::SwitchTab7,
            Self::SwitchTab8,
            Self::SwitchTabLast,
            Self::ToggleDevTools,
            Self::BookmarkPage,
            Self::OpenHistory,
            Self::OpenDownloads,
            Self::ToggleSplitView,
            Self::ToggleStudio,
        ]
    }
}

/// Default Google Chrome-compatible keyboard shortcuts.
pub fn default_chrome_shortcuts() -> BTreeMap<String, String> {
    let entries: [(&str, &str); 34] = [
        ("tab.new", "Ctrl+T"),
        ("tab.close", "Ctrl+W"),
        ("tab.reopenClosed", "Ctrl+Shift+T"),
        ("tab.next", "Ctrl+Tab"),
        ("tab.prev", "Ctrl+Shift+Tab"),
        ("tab.reload", "Ctrl+R"),
        ("tab.hardReload", "Ctrl+Shift+R"),
        ("nav.back", "Alt+ArrowLeft"),
        ("nav.forward", "Alt+ArrowRight"),
        ("nav.focusOmnibox", "Ctrl+L"),
        ("window.new", "Ctrl+N"),
        ("window.newIncognito", "Ctrl+Shift+N"),
        ("tab.pin", "Ctrl+Shift+P"),
        ("tab.duplicate", "Ctrl+Shift+K"),
        ("overlay.toggleMenu", "Alt+E"),
        ("nav.openSettings", "Ctrl+,"),
        ("zoom.in", "Ctrl+="),
        ("zoom.out", "Ctrl+-"),
        ("zoom.reset", "Ctrl+0"),
        ("tab.switch1", "Ctrl+1"),
        ("tab.switch2", "Ctrl+2"),
        ("tab.switch3", "Ctrl+3"),
        ("tab.switch4", "Ctrl+4"),
        ("tab.switch5", "Ctrl+5"),
        ("tab.switch6", "Ctrl+6"),
        ("tab.switch7", "Ctrl+7"),
        ("tab.switch8", "Ctrl+8"),
        ("tab.switchLast", "Ctrl+9"),
        ("devtools.toggle", "F12"),
        ("bookmarks.toggle", "Ctrl+D"),
        ("nav.openHistory", "Ctrl+H"),
        ("nav.openDownloads", "Ctrl+J"),
        ("split.toggle", "Ctrl+Shift+S"),
        ("studio.toggle", "Ctrl+Shift+E"),
    ];
    entries
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Canonical normalized keyboard combination (e.g. `Ctrl+Shift+T`, `Alt+ArrowLeft`, `F5`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: String,
}

impl KeyCombo {
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut main_key: Option<String> = None;

        // Handle literal '+' key like "Ctrl++" or "Ctrl+=" cleanly.
        let normalized_raw = trimmed.replace("++", "+Plus");

        for part in normalized_raw.split('+') {
            let token = part.trim();
            if token.is_empty() {
                continue;
            }
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "shift" => shift = true,
                "alt" => alt = true,
                other => {
                    let canon = normalize_key_token(other)?;
                    main_key = Some(canon);
                }
            }
        }

        let key = main_key?;
        // Require either a modifier or a function key (F1..F12) so regular typing is never hijacked.
        let is_function_key =
            key.starts_with('F') && key.len() >= 2 && key[1..].chars().all(|c| c.is_ascii_digit());
        if !ctrl && !alt && !is_function_key {
            return None;
        }

        Some(Self {
            ctrl,
            shift,
            alt,
            key,
        })
    }

    pub fn canonical(&self) -> String {
        let mut parts = Vec::with_capacity(4);
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        parts.push(self.key.as_str());
        parts.join("+")
    }

    /// Maps a Windows virtual-key code + modifier state into a canonical `KeyCombo`.
    pub fn from_windows_vk(vk: i32, ctrl: bool, shift: bool, alt: bool) -> Option<Self> {
        let key = match vk {
            0x09 => "Tab".to_string(),
            0x0D => "Enter".to_string(),
            0x1B => "Escape".to_string(),
            0x21 => "PageUp".to_string(),
            0x22 => "PageDown".to_string(),
            0x23 => "End".to_string(),
            0x24 => "Home".to_string(),
            0x25 => "ArrowLeft".to_string(),
            0x26 => "ArrowUp".to_string(),
            0x27 => "ArrowRight".to_string(),
            0x28 => "ArrowDown".to_string(),
            0x2E => "Delete".to_string(),
            0x30..=0x39 => ((vk as u8) as char).to_string(),
            0x41..=0x5A => ((vk as u8) as char).to_string(),
            0x60..=0x69 => char::from_digit((vk - 0x60) as u32, 10)?.to_string(),
            0x6B | 0xBB => "=".to_string(), // VK_ADD / VK_OEM_PLUS
            0x6D | 0xBD => "-".to_string(), // VK_SUBTRACT / VK_OEM_MINUS
            0xBC => ",".to_string(),        // VK_OEM_COMMA
            0xBE => ".".to_string(),        // VK_OEM_PERIOD
            0xBF => "/".to_string(),        // VK_OEM_2
            0x70..=0x7B => format!("F{}", vk - 0x70 + 1), // F1..F12
            _ => return None,
        };

        let is_function_key =
            key.starts_with('F') && key.len() >= 2 && key[1..].chars().all(|c| c.is_ascii_digit());
        if !ctrl && !alt && !is_function_key {
            return None;
        }

        Some(Self {
            ctrl,
            shift,
            alt,
            key,
        })
    }
}

fn normalize_key_token(lower: &str) -> Option<String> {
    match lower {
        "tab" => Some("Tab".into()),
        "enter" | "return" => Some("Enter".into()),
        "esc" | "escape" => Some("Escape".into()),
        "pageup" | "pgup" => Some("PageUp".into()),
        "pagedown" | "pgdn" => Some("PageDown".into()),
        "home" => Some("Home".into()),
        "end" => Some("End".into()),
        "arrowleft" | "left" => Some("ArrowLeft".into()),
        "arrowright" | "right" => Some("ArrowRight".into()),
        "arrowup" | "up" => Some("ArrowUp".into()),
        "arrowdown" | "down" => Some("ArrowDown".into()),
        "delete" | "del" => Some("Delete".into()),
        "plus" | "=" | "equal" => Some("=".into()),
        "minus" | "-" => Some("-".into()),
        "," | "comma" => Some(",".into()),
        "." | "period" => Some(".".into()),
        "/" | "slash" => Some("/".into()),
        s if s.len() == 1 => {
            let c = s.chars().next()?;
            if c.is_ascii_alphanumeric() {
                Some(c.to_ascii_uppercase().to_string())
            } else {
                None
            }
        }
        s if s.starts_with('f') && s.len() >= 2 && s.len() <= 3 => {
            let num: u8 = s[1..].parse().ok()?;
            if (1..=12).contains(&num) {
                Some(format!("F{num}"))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Ensures every known `ShortcutAction` has a binding, filling any missing entries
/// with Chrome defaults.
pub fn merge_with_defaults(custom: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut merged = default_chrome_shortcuts();
    for (action_id, combo) in custom {
        if ShortcutAction::from_id(action_id).is_some() {
            if let Some(parsed) = KeyCombo::parse(combo) {
                merged.insert(action_id.clone(), parsed.canonical());
            }
        }
    }
    merged
}

/// Matches a Windows keystroke against the active shortcut map (plus standard Chrome
/// secondary aliases like `F5` for reload, `Alt+D` for focus omnibox, and
/// `Ctrl+PageUp`/`Ctrl+PageDown` for tab cycling).
pub fn match_shortcut_vk(
    shortcuts: &BTreeMap<String, String>,
    vk: i32,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> Option<ShortcutAction> {
    let combo = KeyCombo::from_windows_vk(vk, ctrl, shift, alt)?;
    let canon = combo.canonical();

    for (action_id, bound) in shortcuts {
        if bound.eq_ignore_ascii_case(&canon) {
            if let Some(action) = ShortcutAction::from_id(action_id) {
                return Some(action);
            }
        }
    }

    // Standard Chrome secondary aliases if not explicitly bound elsewhere.
    match canon.as_str() {
        "F5" => Some(ShortcutAction::Reload),
        "Ctrl+F5" => Some(ShortcutAction::HardReload),
        "Alt+D" => Some(ShortcutAction::FocusOmnibox),
        "Ctrl+PageDown" => Some(ShortcutAction::NextTab),
        "Ctrl+PageUp" => Some(ShortcutAction::PrevTab),
        "Ctrl+F4" => Some(ShortcutAction::CloseTab),
        "Ctrl+Shift+I" => Some(ShortcutAction::ToggleDevTools),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalizes_chrome_shortcuts() {
        let parsed = KeyCombo::parse("shift+ctrl+t");
        assert_eq!(
            parsed.as_ref().map(|c| c.canonical()),
            Some("Ctrl+Shift+T".into())
        );

        assert_eq!(
            KeyCombo::parse("ctrl++").map(|c| c.canonical()),
            Some("Ctrl+=".into())
        );
        assert_eq!(KeyCombo::parse("a"), None);
        assert_eq!(
            KeyCombo::parse("f5").map(|c| c.canonical()),
            Some("F5".into())
        );
    }

    #[test]
    fn matches_windows_vk_and_custom_rebinds() {
        let mut map = default_chrome_shortcuts();
        // Ctrl+T -> NewTab (0x54 = 'T')
        assert_eq!(
            match_shortcut_vk(&map, 0x54, true, false, false),
            Some(ShortcutAction::NewTab)
        );
        // Ctrl+Shift+T -> ReopenClosedTab
        assert_eq!(
            match_shortcut_vk(&map, 0x54, true, true, false),
            Some(ShortcutAction::ReopenClosedTab)
        );
        // Alt+ArrowLeft -> Back (0x25)
        assert_eq!(
            match_shortcut_vk(&map, 0x25, false, false, true),
            Some(ShortcutAction::Back)
        );
        // F5 -> Reload secondary alias (0x74)
        assert_eq!(
            match_shortcut_vk(&map, 0x74, false, false, false),
            Some(ShortcutAction::Reload)
        );

        // Rebind NewTab to Ctrl+Q (0x51)
        map.insert("tab.new".into(), "Ctrl+Q".into());
        assert_eq!(
            match_shortcut_vk(&map, 0x51, true, false, false),
            Some(ShortcutAction::NewTab)
        );
        assert_eq!(match_shortcut_vk(&map, 0x54, true, false, false), None);
    }
}
