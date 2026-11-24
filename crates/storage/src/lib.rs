//! SQLite persistence layer for Seu Navegador.
//!
//! Platform-agnostic (no CEF/Win32 dependencies) so it can be shared with
//! Android in future versions.

use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use thiserror::Error;

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
