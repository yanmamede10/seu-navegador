//! SQLite persistence layer for Seu Navegador.
//!
//! Platform-agnostic (no CEF/Win32 dependencies) so it can be shared with
//! Android in future versions.

pub mod importer;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkItem {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryItem {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub visited_at: i64,
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

    pub fn list_bookmarks(&self, limit: usize) -> Result<Vec<BookmarkItem>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, COALESCE(url, ''), created_at
             FROM bookmarks
             WHERE url IS NOT NULL AND url != ''
             ORDER BY id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(BookmarkItem {
                id: row.get(0)?,
                title: row.get(1)?,
                url: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.flatten().collect())
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
}
