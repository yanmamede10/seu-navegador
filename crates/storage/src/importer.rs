//! Browser data importer for Chromium-based browsers (Chrome, Edge, Brave,
//! Vivaldi, Opera / Opera GX) and Mozilla Firefox.
//!
//! Reads only user-accessible local profile data (Bookmarks JSON, History SQLite,
//! Firefox `places.sqlite`) by copying SQLite databases to a temporary file first
//! so imports work safely even while the source browser is currently open.
//! Never attempts to bypass OS/browser credential encryption.

use crate::{BookmarkItem, HistoryItem, Storage, StorageError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserFamily {
    Chromium,
    Firefox,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedBrowserProfile {
    pub id: String,
    pub browser_name: String,
    pub profile_name: String,
    pub family: BrowserFamily,
    pub has_bookmarks: bool,
    pub has_history: bool,
    pub bookmarks_path: Option<String>,
    pub history_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportOptions {
    pub profile_id: String,
    pub import_bookmarks: bool,
    pub import_history: bool,
    pub import_quick_shortcuts: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedQuickShortcut {
    pub id: String,
    pub code: String,
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    pub browser_name: String,
    pub profile_name: String,
    pub bookmarks_imported: usize,
    pub history_imported: usize,
    pub quick_shortcuts: Vec<ImportedQuickShortcut>,
}

/// Scans standard local browser profile directories (Windows `%LOCALAPPDATA%` &
/// `%APPDATA%`, plus `$HOME` fallback on Unix for portability) and lists every
/// detected browser profile with bookmarks and/or history available.
pub fn detect_browser_profiles() -> Vec<DetectedBrowserProfile> {
    let mut profiles = Vec::new();
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let roaming_app_data = std::env::var_os("APPDATA").map(PathBuf::from);

    if let Some(ref local) = local_app_data {
        let chromium_roots = [
            ("Google Chrome", local.join("Google/Chrome/User Data")),
            ("Microsoft Edge", local.join("Microsoft/Edge/User Data")),
            ("Brave", local.join("BraveSoftware/Brave-Browser/User Data")),
            ("Vivaldi", local.join("Vivaldi/User Data")),
        ];
        for (browser_name, user_data) in chromium_roots {
            scan_chromium_user_data(browser_name, &user_data, &mut profiles);
        }
    }

    if let Some(ref roaming) = roaming_app_data {
        // Opera & Opera GX store profile files directly under their stable directory.
        let opera_dirs = [
            (
                "Opera",
                "Stable",
                roaming.join("Opera Software/Opera Stable"),
            ),
            (
                "Opera GX",
                "GX",
                roaming.join("Opera Software/Opera GX Stable"),
            ),
        ];
        for (browser_name, profile_name, dir) in opera_dirs {
            if let Some(p) = inspect_chromium_profile_dir(browser_name, profile_name, &dir) {
                profiles.push(p);
            }
        }

        // Mozilla Firefox profiles.
        let ff_root = roaming.join("Mozilla/Firefox/Profiles");
        scan_firefox_profiles(&ff_root, &mut profiles);
    }

    profiles
}

fn scan_chromium_user_data(
    browser_name: &str,
    user_data_dir: &Path,
    out: &mut Vec<DetectedBrowserProfile>,
) {
    let Ok(entries) = std::fs::read_dir(user_data_dir) else {
        return;
    };

    let mut candidate_dirs: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name == "Default" || name.starts_with("Profile ") {
            candidate_dirs.push((name.to_string(), path));
        }
    }
    candidate_dirs.sort_by(|a, b| a.0.cmp(&b.0));

    for (dir_name, dir_path) in candidate_dirs {
        let display_name =
            read_chromium_profile_display_name(&dir_path).unwrap_or_else(|| dir_name.clone());
        if let Some(profile) = inspect_chromium_profile_dir(browser_name, &display_name, &dir_path)
        {
            out.push(profile);
        }
    }
}

fn read_chromium_profile_display_name(profile_dir: &Path) -> Option<String> {
    let prefs_path = profile_dir.join("Preferences");
    let raw = std::fs::read_to_string(prefs_path).ok()?;
    let json: Value = serde_json::from_str(&raw).ok()?;
    json.get("profile")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn inspect_chromium_profile_dir(
    browser_name: &str,
    profile_name: &str,
    dir: &Path,
) -> Option<DetectedBrowserProfile> {
    let bookmarks = dir.join("Bookmarks");
    let history = dir.join("History");
    let has_bookmarks = bookmarks.is_file();
    let has_history = history.is_file();

    if !has_bookmarks && !has_history {
        return None;
    }

    let id = format!(
        "chromium:{}:{}",
        slugify(browser_name),
        slugify(&dir.to_string_lossy())
    );

    Some(DetectedBrowserProfile {
        id,
        browser_name: browser_name.to_string(),
        profile_name: profile_name.to_string(),
        family: BrowserFamily::Chromium,
        has_bookmarks,
        has_history,
        bookmarks_path: has_bookmarks.then(|| bookmarks.to_string_lossy().to_string()),
        history_path: has_history.then(|| history.to_string_lossy().to_string()),
    })
}

fn scan_firefox_profiles(profiles_root: &Path, out: &mut Vec<DetectedBrowserProfile>) {
    let Ok(entries) = std::fs::read_dir(profiles_root) else {
        return;
    };

    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    for dir in dirs {
        let places = dir.join("places.sqlite");
        if !places.is_file() {
            continue;
        }
        let profile_name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("default")
            .to_string();
        let places_str = places.to_string_lossy().to_string();
        out.push(DetectedBrowserProfile {
            id: format!("firefox:{}", slugify(&places_str)),
            browser_name: "Mozilla Firefox".to_string(),
            profile_name,
            family: BrowserFamily::Firefox,
            has_bookmarks: true,
            has_history: true,
            bookmarks_path: Some(places_str.clone()),
            history_path: Some(places_str),
        });
    }
}

/// Imports the selected data categories from a detected browser profile into
/// Seu Navegador's SQLite database.
pub fn run_profile_import(
    storage: &Storage,
    profile: &DetectedBrowserProfile,
    options: &ImportOptions,
) -> Result<ImportReport, StorageError> {
    let mut bookmarks_imported = 0usize;
    let mut history_imported = 0usize;
    let mut candidate_shortcuts: Vec<(String, String)> = Vec::new();

    match profile.family {
        BrowserFamily::Chromium => {
            if options.import_bookmarks {
                if let Some(ref path_str) = profile.bookmarks_path {
                    let parsed = parse_chromium_bookmarks_file(Path::new(path_str));
                    for (title, url) in &parsed {
                        if candidate_shortcuts.len() < 12 {
                            candidate_shortcuts.push((title.clone(), url.clone()));
                        }
                    }
                    bookmarks_imported = storage.insert_bookmarks_dedup(&parsed)?;
                }
            }
            if options.import_history || options.import_quick_shortcuts {
                if let Some(ref path_str) = profile.history_path {
                    let items = read_chromium_history_sqlite(Path::new(path_str), 2000);
                    if options.import_quick_shortcuts {
                        for item in items.iter().take(12) {
                            candidate_shortcuts.push((item.title.clone(), item.url.clone()));
                        }
                    }
                    if options.import_history {
                        history_imported = storage.insert_history_dedup(&items)?;
                    }
                }
            }
        }
        BrowserFamily::Firefox => {
            if let Some(ref path_str) = profile.history_path {
                let (ff_bookmarks, ff_history) =
                    read_firefox_places_sqlite(Path::new(path_str), 2000);
                if options.import_bookmarks {
                    bookmarks_imported = storage.insert_bookmarks_dedup(&ff_bookmarks)?;
                }
                if options.import_history {
                    history_imported = storage.insert_history_dedup(&ff_history)?;
                }
                if options.import_quick_shortcuts {
                    for item in ff_history.iter().take(8) {
                        candidate_shortcuts.push((item.title.clone(), item.url.clone()));
                    }
                    for (title, url) in ff_bookmarks.iter().take(8) {
                        candidate_shortcuts.push((title.clone(), url.clone()));
                    }
                }
            }
        }
    }

    let quick_shortcuts = if options.import_quick_shortcuts {
        build_quick_shortcuts_from_pairs(&candidate_shortcuts, 6)
    } else {
        Vec::new()
    };

    Ok(ImportReport {
        browser_name: profile.browser_name.clone(),
        profile_name: profile.profile_name.clone(),
        bookmarks_imported,
        history_imported,
        quick_shortcuts,
    })
}

/// Parses a Chromium `Bookmarks` JSON file into `(title, url)` pairs.
pub fn parse_chromium_bookmarks_file(path: &Path) -> Vec<(String, String)> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_chromium_bookmarks_str(&raw)
}

pub fn parse_chromium_bookmarks_str(raw_json: &str) -> Vec<(String, String)> {
    let Ok(val) = serde_json::from_str::<Value>(raw_json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(roots) = val.get("roots").and_then(|r| r.as_object()) {
        for root_node in roots.values() {
            collect_chromium_bookmark_node(root_node, &mut out);
        }
    }
    out
}

fn collect_chromium_bookmark_node(node: &Value, out: &mut Vec<(String, String)>) {
    let node_type = node.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if node_type == "url" {
        if let Some(url) = node.get("url").and_then(|u| u.as_str()) {
            let trimmed_url = url.trim();
            if trimmed_url.starts_with("http://") || trimmed_url.starts_with("https://") {
                let title = node
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(trimmed_url)
                    .to_string();
                out.push((title, trimmed_url.to_string()));
            }
        }
    }
    if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
        for child in children {
            collect_chromium_bookmark_node(child, out);
        }
    }
}

/// Copies a Chromium `History` SQLite DB to a temporary file (so it succeeds
/// even if Chrome/Edge/Brave holds a lock) and reads the most recent/frequent URLs.
pub fn read_chromium_history_sqlite(path: &Path, limit: usize) -> Vec<HistoryItem> {
    let Some(temp_path) = copy_to_temp_sqlite(path, "chromium_history") else {
        return Vec::new();
    };
    let result = read_chromium_history_from_path(&temp_path, limit);
    let _ = std::fs::remove_file(&temp_path);
    result
}

fn read_chromium_history_from_path(path: &Path, limit: usize) -> Vec<HistoryItem> {
    let Ok(conn) =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return Vec::new();
    };

    let Ok(mut stmt) = conn.prepare(
        "SELECT url, title, last_visit_time
         FROM urls
         WHERE url LIKE 'http%'
         ORDER BY visit_count DESC, last_visit_time DESC
         LIMIT ?1",
    ) else {
        return Vec::new();
    };

    let Ok(rows) = stmt.query_map([limit as i64], |row| {
        let url: String = row.get(0)?;
        let title: String = row.get::<_, Option<String>>(1)?.unwrap_or_default();
        let webkit_us: i64 = row.get::<_, Option<i64>>(2)?.unwrap_or(0);
        // Chromium WebKit timestamps are microseconds since 1601-01-01 UTC.
        let unix_secs = (webkit_us / 1_000_000)
            .saturating_sub(11_644_473_600)
            .max(0);
        Ok(HistoryItem {
            id: 0,
            url: url.clone(),
            title: if title.trim().is_empty() { url } else { title },
            visited_at: unix_secs,
        })
    }) else {
        return Vec::new();
    };

    rows.flatten().collect()
}

/// Copies a Firefox `places.sqlite` DB to a temporary file and extracts both
/// bookmarks and history.
pub fn read_firefox_places_sqlite(
    path: &Path,
    limit: usize,
) -> (Vec<(String, String)>, Vec<HistoryItem>) {
    let Some(temp_path) = copy_to_temp_sqlite(path, "firefox_places") else {
        return (Vec::new(), Vec::new());
    };
    let result = read_firefox_places_from_path(&temp_path, limit);
    let _ = std::fs::remove_file(&temp_path);
    result
}

fn read_firefox_places_from_path(
    path: &Path,
    limit: usize,
) -> (Vec<(String, String)>, Vec<HistoryItem>) {
    let Ok(conn) =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return (Vec::new(), Vec::new());
    };

    let mut bookmarks = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT COALESCE(b.title, p.title, p.url), p.url
         FROM moz_bookmarks b
         INNER JOIN moz_places p ON p.id = b.fk
         WHERE b.type = 1 AND p.url LIKE 'http%'
         LIMIT 2000",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            let title: String = row.get(0)?;
            let url: String = row.get(1)?;
            Ok((title, url))
        }) {
            for item in rows.flatten() {
                bookmarks.push(item);
            }
        }
    }

    let mut history = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT url, COALESCE(title, url), COALESCE(last_visit_date, 0)
         FROM moz_places
         WHERE url LIKE 'http%' AND visit_count > 0
         ORDER BY frecency DESC, last_visit_date DESC
         LIMIT ?1",
    ) {
        if let Ok(rows) = stmt.query_map([limit as i64], |row| {
            let url: String = row.get(0)?;
            let title: String = row.get(1)?;
            let pr_time_us: i64 = row.get(2)?;
            Ok(HistoryItem {
                id: 0,
                url,
                title,
                visited_at: (pr_time_us / 1_000_000).max(0),
            })
        }) {
            for item in rows.flatten() {
                history.push(item);
            }
        }
    }

    (bookmarks, history)
}

fn copy_to_temp_sqlite(source: &Path, prefix: &str) -> Option<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_path = std::env::temp_dir().join(format!("seu_{prefix}_{nonce}.sqlite"));
    std::fs::copy(source, &temp_path).ok()?;
    Some(temp_path)
}

fn build_quick_shortcuts_from_pairs(
    pairs: &[(String, String)],
    max_count: usize,
) -> Vec<ImportedQuickShortcut> {
    let mut seen_hosts = HashSet::new();
    let mut out = Vec::new();

    for (title, url) in pairs {
        if out.len() >= max_count {
            break;
        }
        let Some((host, origin)) = extract_host_and_origin(url) else {
            continue;
        };
        if !seen_hosts.insert(host.clone()) {
            continue;
        }
        let clean_name = short_site_name(&host, title);
        let code = two_letter_code(&clean_name);
        out.push(ImportedQuickShortcut {
            id: format!("imp-{}", slugify(&host)),
            code,
            name: clean_name,
            url: origin,
        });
    }
    out
}

fn extract_host_and_origin(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
        ("https://", r)
    } else {
        ("http://", url.strip_prefix("http://")?)
    };
    let host = rest.split(['/', '?', '#']).next()?.trim();
    if host.is_empty() || host.contains("localhost") || host.starts_with("127.") {
        return None;
    }
    let bare = host
        .strip_prefix("www.")
        .unwrap_or(host)
        .to_ascii_lowercase();
    Some((bare, format!("{scheme}{host}")))
}

fn short_site_name(host: &str, fallback_title: &str) -> String {
    let first_label = host.split('.').next().unwrap_or(fallback_title);
    let mut chars = first_label.chars();
    match chars.next() {
        Some(c) => format!("{}{}", c.to_ascii_uppercase(), chars.as_str()),
        None => fallback_title.chars().take(12).collect(),
    }
}

fn two_letter_code(name: &str) -> String {
    let letters: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(2)
        .collect();
    if letters.is_empty() {
        "WW".into()
    } else {
        letters.to_ascii_uppercase()
    }
}

fn slugify(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

pub fn bookmark_items_summary(items: &[BookmarkItem]) -> usize {
    items.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chromium_bookmarks_and_imports_dedup() {
        let sample = r#"{
            "roots": {
                "bookmark_bar": {
                    "type": "folder",
                    "name": "Bookmarks bar",
                    "children": [
                        { "type": "url", "name": "YouTube", "url": "https://www.youtube.com/" },
                        { "type": "url", "name": "GitHub", "url": "https://github.com/" },
                        { "type": "url", "name": "JS Evil", "url": "javascript:alert(1)" }
                    ]
                }
            }
        }"#;

        let parsed = parse_chromium_bookmarks_str(sample);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "YouTube");
        assert_eq!(parsed[1].1, "https://github.com/");

        let db = Storage::open_in_memory();
        assert!(db.is_ok());
        if let Ok(storage) = db {
            assert_eq!(storage.insert_bookmarks_dedup(&parsed).ok(), Some(2));
            // Second import should skip duplicates.
            assert_eq!(storage.insert_bookmarks_dedup(&parsed).ok(), Some(0));
            assert_eq!(storage.list_bookmarks(50).ok().map(|b| b.len()), Some(2));
        }
    }

    #[test]
    fn imports_chromium_history_sqlite_safely() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp_src = std::env::temp_dir().join(format!("seu_test_chrome_hist_{nonce}.sqlite"));

        if let Ok(conn) = rusqlite::Connection::open(&temp_src) {
            let _ = conn.execute_batch(
                "CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT, title TEXT, visit_count INTEGER, last_visit_time INTEGER);
                 INSERT INTO urls (url, title, visit_count, last_visit_time) VALUES
                   ('https://github.com/yanmamede10', 'Yan GitHub', 12, 13370000000000000),
                   ('https://docs.rs/', 'Docs.rs', 5, 13370000001000000);",
            );
        }

        let items = read_chromium_history_sqlite(&temp_src, 100);
        let _ = std::fs::remove_file(&temp_src);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].url, "https://github.com/yanmamede10");

        let shortcuts = build_quick_shortcuts_from_pairs(
            &items
                .iter()
                .map(|i| (i.title.clone(), i.url.clone()))
                .collect::<Vec<_>>(),
            6,
        );
        assert_eq!(shortcuts.len(), 2);
        assert_eq!(shortcuts[0].code, "GI");
        assert_eq!(shortcuts[1].code, "DO");
    }
}
