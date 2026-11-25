//! SQLite persistence layer for Seu Navegador.
//!
//! Platform-agnostic (no CEF/Win32 dependencies) so it can be shared with
//! Android in future versions.

pub mod importer;
pub mod netscape;

use netscape::parse_netscape_bookmarks_html;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkItem {
    pub id: i64,
    #[serde(default)]
    pub parent_id: Option<i64>,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub is_folder: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryItem {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub visited_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadItem {
    pub id: i64,
    pub url: String,
    pub filename: String,
    pub target_path: String,
    pub total_bytes: i64,
    pub received_bytes: i64,
    /// One of: `"in_progress"`, `"paused"`, `"complete"`, `"canceled"`.
    pub state: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SitePermissionItem {
    pub origin: String,
    pub permission: String,
    /// `"allow"` or `"deny"`.
    pub decision: String,
    pub updated_at: i64,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let conn = Connection::open(path)?;
        let storage = Self { conn };
        storage.init_schema()?;
        Ok(storage)
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        let storage = Self { conn };
        storage.init_schema()?;
        Ok(storage)
    }

    fn init_schema(&self) -> Result<(), StorageError> {
        self.conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL,
                title TEXT NOT NULL,
                visited_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS bookmarks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                parent_id INTEGER,
                title TEXT NOT NULL,
                url TEXT,
                position INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS downloads (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL,
                target_path TEXT NOT NULL,
                total_bytes INTEGER NOT NULL DEFAULT 0,
                received_bytes INTEGER NOT NULL DEFAULT 0,
                state TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS permissions (
                origin TEXT NOT NULL,
                permission TEXT NOT NULL,
                decision TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (origin, permission)
            );
            ",
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        let value = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()?;
        Ok(value)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Bookmarks (including folders + Netscape HTML import)
    // -------------------------------------------------------------------------

    pub fn insert_bookmarks_dedup(
        &self,
        items: &[(String, String)],
    ) -> Result<usize, StorageError> {
        let now = unix_now_secs();
        let mut inserted = 0usize;
        for (title, url) in items {
            let exists: bool = self
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM bookmarks WHERE url = ?1)",
                    [url],
                    |r| r.get::<_, i64>(0),
                )
                .map(|v| v != 0)?;
            if !exists {
                self.conn.execute(
                    "INSERT INTO bookmarks (parent_id, title, url, position, created_at)
                     VALUES (NULL, ?1, ?2, 0, ?3)",
                    rusqlite::params![title, url, now],
                )?;
                inserted += 1;
            }
        }
        Ok(inserted)
    }

    pub fn add_bookmark(
        &self,
        title: &str,
        url: &str,
        parent_id: Option<i64>,
    ) -> Result<BookmarkItem, StorageError> {
        let now = unix_now_secs();
        let clean_title = if title.trim().is_empty() {
            url.trim()
        } else {
            title.trim()
        };
        self.conn.execute(
            "INSERT INTO bookmarks (parent_id, title, url, position, created_at)
             VALUES (?1, ?2, ?3, 0, ?4)",
            rusqlite::params![parent_id, clean_title, url.trim(), now],
        )?;
        let id = self.conn.last_insert_rowid();
        Ok(BookmarkItem {
            id,
            parent_id,
            title: clean_title.to_string(),
            url: url.trim().to_string(),
            is_folder: false,
            created_at: now,
        })
    }

    pub fn create_bookmark_folder(
        &self,
        title: &str,
        parent_id: Option<i64>,
    ) -> Result<BookmarkItem, StorageError> {
        let now = unix_now_secs();
        let clean = title.trim();
        self.conn.execute(
            "INSERT INTO bookmarks (parent_id, title, url, position, created_at)
             VALUES (?1, ?2, NULL, 0, ?3)",
            rusqlite::params![parent_id, clean, now],
        )?;
        let id = self.conn.last_insert_rowid();
        Ok(BookmarkItem {
            id,
            parent_id,
            title: clean.to_string(),
            url: String::new(),
            is_folder: true,
            created_at: now,
        })
    }

    fn get_or_create_folder(
        &self,
        title: &str,
        parent_id: Option<i64>,
    ) -> Result<i64, StorageError> {
        let existing: Option<i64> = match parent_id {
            Some(pid) => self
                .conn
                .query_row(
                    "SELECT id FROM bookmarks WHERE url IS NULL AND title = ?1 AND parent_id = ?2 LIMIT 1",
                    rusqlite::params![title, pid],
                    |r| r.get(0),
                )
                .optional()?,
            None => self
                .conn
                .query_row(
                    "SELECT id FROM bookmarks WHERE url IS NULL AND title = ?1 AND parent_id IS NULL LIMIT 1",
                    [title],
                    |r| r.get(0),
                )
                .optional()?,
        };
        if let Some(id) = existing {
            return Ok(id);
        }
        Ok(self.create_bookmark_folder(title, parent_id)?.id)
    }

    /// Imports bookmarks + folder hierarchy from a Netscape Bookmark HTML document.
    pub fn import_netscape_html(&self, html: &str) -> Result<usize, StorageError> {
        let parsed = parse_netscape_bookmarks_html(html);
        let now = unix_now_secs();
        let mut inserted = 0usize;

        for entry in parsed {
            let exists: bool = self
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM bookmarks WHERE url = ?1)",
                    [&entry.url],
                    |r| r.get::<_, i64>(0),
                )
                .map(|v| v != 0)?;
            if exists {
                continue;
            }
            let mut parent_id: Option<i64> = None;
            for folder in &entry.folder_path {
                parent_id = Some(self.get_or_create_folder(folder, parent_id)?);
            }
            self.conn.execute(
                "INSERT INTO bookmarks (parent_id, title, url, position, created_at)
                 VALUES (?1, ?2, ?3, 0, ?4)",
                rusqlite::params![parent_id, entry.title, entry.url, now],
            )?;
            inserted += 1;
        }
        Ok(inserted)
    }

    pub fn is_url_bookmarked(&self, url: &str) -> Result<bool, StorageError> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM bookmarks WHERE url = ?1)",
                [url.trim()],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v != 0)?;
        Ok(exists)
    }

    /// Toggles a URL in bookmarks. Returns `true` if now bookmarked, `false` if removed.
    pub fn toggle_bookmark(
        &self,
        title: &str,
        url: &str,
        parent_id: Option<i64>,
    ) -> Result<bool, StorageError> {
        let clean_url = url.trim();
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM bookmarks WHERE url = ?1 LIMIT 1",
                [clean_url],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            self.conn
                .execute("DELETE FROM bookmarks WHERE id = ?1", [id])?;
            Ok(false)
        } else {
            self.add_bookmark(title, clean_url, parent_id)?;
            Ok(true)
        }
    }

    pub fn delete_bookmark(&self, id: i64) -> Result<(), StorageError> {
        // Re-parent children to root before deleting a folder so no orphan bookmarks are lost.
        self.conn.execute(
            "UPDATE bookmarks SET parent_id = NULL WHERE parent_id = ?1",
            [id],
        )?;
        self.conn
            .execute("DELETE FROM bookmarks WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn list_bookmarks(&self, limit: usize) -> Result<Vec<BookmarkItem>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, parent_id, title, COALESCE(url, ''), created_at
             FROM bookmarks
             WHERE url IS NOT NULL AND url != ''
             ORDER BY id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            let url: String = row.get(3)?;
            Ok(BookmarkItem {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                title: row.get(2)?,
                is_folder: url.is_empty(),
                url,
                created_at: row.get(4)?,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    /// Lists both folders and bookmarks (folders first, then newest bookmarks).
    pub fn list_bookmarks_all(&self, limit: usize) -> Result<Vec<BookmarkItem>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, parent_id, title, COALESCE(url, ''), created_at
             FROM bookmarks
             ORDER BY CASE WHEN url IS NULL OR url = '' THEN 0 ELSE 1 END, id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            let url: String = row.get(3)?;
            Ok(BookmarkItem {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                title: row.get(2)?,
                is_folder: url.is_empty(),
                url,
                created_at: row.get(4)?,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    // -------------------------------------------------------------------------
    // History
    // -------------------------------------------------------------------------

    pub fn record_history_visit(&self, url: &str, title: &str) -> Result<i64, StorageError> {
        let clean_url = url.trim();
        let clean_title = if title.trim().is_empty() {
            clean_url
        } else {
            title.trim()
        };
        let now = unix_now_secs();
        // Avoid spamming consecutive duplicates of the exact same URL within 10s.
        let recent_same: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM history WHERE url = ?1 AND visited_at >= ?2 ORDER BY id DESC LIMIT 1",
                rusqlite::params![clean_url, now.saturating_sub(10)],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing_id) = recent_same {
            self.conn.execute(
                "UPDATE history SET title = ?1, visited_at = ?2 WHERE id = ?3",
                rusqlite::params![clean_title, now, existing_id],
            )?;
            return Ok(existing_id);
        }
        self.conn.execute(
            "INSERT INTO history (url, title, visited_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![clean_url, clean_title, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_latest_history_title(&self, url: &str, title: &str) -> Result<(), StorageError> {
        if title.trim().is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "UPDATE history SET title = ?1
             WHERE id = (SELECT id FROM history WHERE url = ?2 ORDER BY id DESC LIMIT 1)",
            rusqlite::params![title.trim(), url.trim()],
        )?;
        Ok(())
    }

    pub fn insert_history_dedup(&self, items: &[HistoryItem]) -> Result<usize, StorageError> {
        let mut inserted = 0usize;
        for item in items {
            let exists: bool = self
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM history WHERE url = ?1)",
                    [&item.url],
                    |r| r.get::<_, i64>(0),
                )
                .map(|v| v != 0)?;
            if !exists {
                self.conn.execute(
                    "INSERT INTO history (url, title, visited_at) VALUES (?1, ?2, ?3)",
                    rusqlite::params![item.url, item.title, item.visited_at],
                )?;
                inserted += 1;
            }
        }
        Ok(inserted)
    }

    pub fn list_history(&self, limit: usize) -> Result<Vec<HistoryItem>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, url, title, visited_at
             FROM history
             ORDER BY visited_at DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(HistoryItem {
                id: row.get(0)?,
                url: row.get(1)?,
                title: row.get(2)?,
                visited_at: row.get(3)?,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    pub fn search_history(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<HistoryItem>, StorageError> {
        let q = query.trim();
        if q.is_empty() {
            return self.list_history(limit);
        }
        let like = format!("%{q}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, url, title, visited_at
             FROM history
             WHERE url LIKE ?1 OR title LIKE ?1
             ORDER BY visited_at DESC, id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![like, limit as i64], |row| {
            Ok(HistoryItem {
                id: row.get(0)?,
                url: row.get(1)?,
                title: row.get(2)?,
                visited_at: row.get(3)?,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    pub fn delete_history_item(&self, id: i64) -> Result<(), StorageError> {
        self.conn
            .execute("DELETE FROM history WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn clear_history(&self) -> Result<(), StorageError> {
        self.conn.execute("DELETE FROM history", [])?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Downloads
    // -------------------------------------------------------------------------

    pub fn upsert_download(&self, item: &DownloadItem) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO downloads (id, url, target_path, total_bytes, received_bytes, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
               url = excluded.url,
               target_path = excluded.target_path,
               total_bytes = excluded.total_bytes,
               received_bytes = excluded.received_bytes,
               state = excluded.state",
            rusqlite::params![
                item.id,
                item.url,
                item.target_path,
                item.total_bytes,
                item.received_bytes,
                item.state,
                item.created_at
            ],
        )?;
        Ok(())
    }

    pub fn list_downloads(&self, limit: usize) -> Result<Vec<DownloadItem>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, url, target_path, total_bytes, received_bytes, state, created_at
             FROM downloads
             ORDER BY created_at DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            let target_path: String = row.get(2)?;
            let filename = Path::new(&target_path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| target_path.clone());
            Ok(DownloadItem {
                id: row.get(0)?,
                url: row.get(1)?,
                filename,
                target_path,
                total_bytes: row.get(3)?,
                received_bytes: row.get(4)?,
                state: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    pub fn delete_download(&self, id: i64) -> Result<(), StorageError> {
        self.conn
            .execute("DELETE FROM downloads WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn clear_finished_downloads(&self) -> Result<(), StorageError> {
        self.conn.execute(
            "DELETE FROM downloads WHERE state IN ('complete', 'canceled')",
            [],
        )?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Site Permissions
    // -------------------------------------------------------------------------

    pub fn set_permission(
        &self,
        origin: &str,
        permission: &str,
        decision: &str,
    ) -> Result<(), StorageError> {
        let now = unix_now_secs();
        self.conn.execute(
            "INSERT INTO permissions (origin, permission, decision, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(origin, permission) DO UPDATE SET
               decision = excluded.decision,
               updated_at = excluded.updated_at",
            rusqlite::params![origin.trim(), permission.trim(), decision.trim(), now],
        )?;
        Ok(())
    }

    pub fn get_permission(
        &self,
        origin: &str,
        permission: &str,
    ) -> Result<Option<String>, StorageError> {
        let res = self
            .conn
            .query_row(
                "SELECT decision FROM permissions WHERE origin = ?1 AND permission = ?2",
                rusqlite::params![origin.trim(), permission.trim()],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(res)
    }

    pub fn list_permissions(&self) -> Result<Vec<SitePermissionItem>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT origin, permission, decision, updated_at
             FROM permissions
             ORDER BY updated_at DESC, origin ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SitePermissionItem {
                origin: row.get(0)?,
                permission: row.get(1)?,
                decision: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    pub fn delete_permission(&self, origin: &str, permission: &str) -> Result<(), StorageError> {
        self.conn.execute(
            "DELETE FROM permissions WHERE origin = ?1 AND permission = ?2",
            rusqlite::params![origin.trim(), permission.trim()],
        )?;
        Ok(())
    }
}

fn unix_now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_and_reads_settings() {
        let storage = Storage::open_in_memory();
        assert!(storage.is_ok());
        if let Ok(db) = storage {
            assert_eq!(db.get_setting("locale").ok().flatten(), None);
            assert!(db.set_setting("locale", "en").is_ok());
            assert_eq!(
                db.get_setting("locale").ok().flatten().as_deref(),
                Some("en")
            );
            assert!(db.set_setting("locale", "pt-BR").is_ok());
            assert_eq!(
                db.get_setting("locale").ok().flatten().as_deref(),
                Some("pt-BR")
            );
        }
    }

    #[test]
    fn records_searches_and_clears_history() {
        let storage = Storage::open_in_memory();
        assert!(storage.is_ok());
        if let Ok(db) = storage {
            assert!(db
                .record_history_visit("https://rust-lang.org/", "Rust")
                .is_ok());
            assert!(db
                .record_history_visit("https://github.com/yanmamede10", "Yan GitHub")
                .is_ok());

            let found = db.search_history("github", 20).unwrap_or_default();
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].title, "Yan GitHub");

            assert!(db.delete_history_item(found[0].id).is_ok());
            assert_eq!(db.list_history(20).unwrap_or_default().len(), 1);

            assert!(db.clear_history().is_ok());
            assert!(db.list_history(20).unwrap_or_default().is_empty());
        }
    }

    #[test]
    fn manages_bookmarks_folders_and_netscape_import() {
        let storage = Storage::open_in_memory();
        assert!(storage.is_ok());
        if let Ok(db) = storage {
            let added = db
                .toggle_bookmark("Example", "https://example.com/", None)
                .unwrap_or(false);
            assert!(added);
            assert!(db
                .is_url_bookmarked("https://example.com/")
                .unwrap_or(false));

            let removed = db
                .toggle_bookmark("Example", "https://example.com/", None)
                .unwrap_or(true);
            assert!(!removed);

            let html = r#"
<DL><p>
  <DT><H3>Dev</H3>
  <DL><p>
    <DT><A HREF="https://docs.rs/">docs.rs</A>
  </DL><p>
</DL><p>
"#;
            let imported = db.import_netscape_html(html).unwrap_or(0);
            assert_eq!(imported, 1);
            let all = db.list_bookmarks_all(50).unwrap_or_default();
            assert!(all.iter().any(|b| b.is_folder && b.title == "Dev"));
            assert!(all
                .iter()
                .any(|b| !b.is_folder && b.url == "https://docs.rs/" && b.parent_id.is_some()));
        }
    }

    #[test]
    fn persists_downloads_and_permissions() {
        let storage = Storage::open_in_memory();
        assert!(storage.is_ok());
        if let Ok(db) = storage {
            let dl = DownloadItem {
                id: 42,
                url: "https://example.com/file.zip".into(),
                filename: "file.zip".into(),
                target_path: "C:\\Downloads\\file.zip".into(),
                total_bytes: 1000,
                received_bytes: 500,
                state: "in_progress".into(),
                created_at: 1700000000,
            };
            assert!(db.upsert_download(&dl).is_ok());
            let list = db.list_downloads(10).unwrap_or_default();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].received_bytes, 500);

            assert!(db
                .set_permission("https://meet.google.com", "camera", "allow")
                .is_ok());
            assert_eq!(
                db.get_permission("https://meet.google.com", "camera")
                    .ok()
                    .flatten()
                    .as_deref(),
                Some("allow")
            );
            assert!(db
                .delete_permission("https://meet.google.com", "camera")
                .is_ok());
            assert!(db.list_permissions().unwrap_or_default().is_empty());
        }
    }
}
