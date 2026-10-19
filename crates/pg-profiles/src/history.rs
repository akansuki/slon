//! Query history: what was run, where, and how it went.
//!
//! Stored as plain text in a JSON file in the app's config folder (like the saved
//! connections, but it holds SQL, not secrets). Statements that look like they carry a
//! password are never stored.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use pg_core::PgError;

use crate::jsonfile::JsonFile;

/// Oldest entries are dropped beyond this many.
pub const MAX_ENTRIES: usize = 1000;
/// Longer statements are not recorded (a pasted data dump is not history).
pub const MAX_SQL_BYTES: usize = 100_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryInput {
    pub sql: String,
    /// `user@host:port`
    pub server: String,
    pub database: String,
    pub ok: bool,
    /// Short error message when the run failed.
    pub error: Option<String>,
    pub duration_ms: u64,
    pub rows: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: u64,
    pub sql: String,
    pub server: String,
    pub database: String,
    pub ok: bool,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub rows: Option<u64>,
    /// Unix time in milliseconds of the latest run.
    pub at: u64,
    /// How many times in a row this exact statement was run.
    pub count: u32,
}

#[derive(Default, Serialize, Deserialize)]
struct Data {
    entries: Vec<HistoryEntry>,
}

pub struct HistoryStore {
    file: JsonFile,
    lock: Mutex<()>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Best effort: `PASSWORD 'x'`, `PASSWORD = 'x'`, `password=x` (connection strings) and
/// `PASSWORD $$x$$`. A column or function merely named `password` is fine.
pub fn looks_secret(sql: &str) -> bool {
    let lower = sql.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut from = 0;
    while let Some(i) = lower[from..].find("password") {
        let mut j = from + i + "password".len();
        from = j;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let mut assigned = false;
        if j < bytes.len() && bytes[j] == b'=' {
            assigned = true;
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
        }
        // E'..' string literal
        if j + 1 < bytes.len() && bytes[j] == b'e' && bytes[j + 1] == b'\'' {
            return true;
        }
        match bytes.get(j) {
            Some(b'\'') | Some(b'$') => return true,
            Some(c)
                if assigned
                    && !c.is_ascii_whitespace()
                    && *c != b';'
                    && *c != b','
                    && *c != b')' =>
            {
                return true
            }
            _ => {}
        }
    }
    false
}

impl HistoryStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            file: JsonFile::new(path),
            lock: Mutex::new(()),
        }
    }

    /// Records a run. Returns `false` when the statement was deliberately not stored.
    pub fn add(&self, input: HistoryInput) -> Result<bool, PgError> {
        let sql = input.sql.trim();
        if sql.is_empty() || sql.len() > MAX_SQL_BYTES || looks_secret(sql) {
            return Ok(false);
        }
        let _g = self.lock.lock().unwrap();
        let mut data: Data = self.file.read();
        let at = now_ms();
        let error = input.error.map(|e| e.chars().take(300).collect::<String>());
        match data.entries.last_mut() {
            Some(last)
                if last.sql == sql
                    && last.server == input.server
                    && last.database == input.database =>
            {
                last.at = at;
                last.duration_ms = input.duration_ms;
                last.ok = input.ok;
                last.error = error;
                last.rows = input.rows;
                last.count = last.count.saturating_add(1);
            }
            _ => {
                let id = data.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
                data.entries.push(HistoryEntry {
                    id,
                    sql: sql.to_string(),
                    server: input.server,
                    database: input.database,
                    ok: input.ok,
                    error,
                    duration_ms: input.duration_ms,
                    rows: input.rows,
                    at,
                    count: 1,
                });
                let extra = data.entries.len().saturating_sub(MAX_ENTRIES);
                if extra > 0 {
                    data.entries.drain(..extra);
                }
            }
        }
        self.file.write(&data)?;
        Ok(true)
    }

    /// Newest first. `search` is a case-insensitive substring of the SQL; `server` limits
    /// the list to one `user@host:port`.
    pub fn list(
        &self,
        search: Option<&str>,
        server: Option<&str>,
        limit: usize,
    ) -> Vec<HistoryEntry> {
        let _g = self.lock.lock().unwrap();
        let data: Data = self.file.read();
        let needle = search
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());
        data.entries
            .into_iter()
            .rev()
            .filter(|e| server.is_none_or(|s| e.server == s))
            .filter(|e| {
                needle
                    .as_ref()
                    .is_none_or(|n| e.sql.to_lowercase().contains(n))
            })
            .take(limit)
            .collect()
    }

    pub fn delete(&self, id: u64) -> Result<(), PgError> {
        let _g = self.lock.lock().unwrap();
        let mut data: Data = self.file.read();
        data.entries.retain(|e| e.id != id);
        self.file.write(&data)
    }

    /// Forget everything, or only what ran on one server.
    pub fn clear(&self, server: Option<&str>) -> Result<(), PgError> {
        let _g = self.lock.lock().unwrap();
        let mut data: Data = self.file.read();
        match server {
            Some(s) => data.entries.retain(|e| e.server != s),
            None => data.entries.clear(),
        }
        self.file.write(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(sql: &str) -> HistoryInput {
        HistoryInput {
            sql: sql.into(),
            server: "me@db:5432".into(),
            database: "app".into(),
            ok: true,
            error: None,
            duration_ms: 3,
            rows: Some(1),
        }
    }

    fn store() -> (tempfile::TempDir, HistoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = HistoryStore::new(dir.path().join("history.json"));
        (dir, s)
    }

    #[test]
    fn adds_lists_newest_first_and_survives_a_restart() {
        let (dir, s) = store();
        s.add(input("select 1")).unwrap();
        s.add(input("select 2")).unwrap();
        let again = HistoryStore::new(dir.path().join("history.json"));
        let l = again.list(None, None, 10);
        assert_eq!(
            l.iter().map(|e| e.sql.as_str()).collect::<Vec<_>>(),
            ["select 2", "select 1"]
        );
    }

    #[test]
    fn repeating_the_same_statement_updates_the_entry() {
        let (_d, s) = store();
        s.add(input("select 1")).unwrap();
        let mut second = input("  select 1 \n");
        second.ok = false;
        second.error = Some("boom".into());
        s.add(second).unwrap();
        let l = s.list(None, None, 10);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].count, 2);
        assert!(!l[0].ok);
        assert_eq!(l[0].error.as_deref(), Some("boom"));
        // another statement in between breaks the run
        s.add(input("select 2")).unwrap();
        s.add(input("select 1")).unwrap();
        assert_eq!(s.list(None, None, 10).len(), 3);
    }

    #[test]
    fn same_text_on_another_database_is_a_new_entry() {
        let (_d, s) = store();
        s.add(input("select 1")).unwrap();
        let mut other = input("select 1");
        other.database = "other".into();
        s.add(other).unwrap();
        assert_eq!(s.list(None, None, 10).len(), 2);
    }

    #[test]
    fn filters_by_text_and_server() {
        let (_d, s) = store();
        s.add(input("SELECT * FROM Users")).unwrap();
        let mut other = input("select 1");
        other.server = "x@y:1".into();
        s.add(other).unwrap();
        assert_eq!(s.list(Some("users"), None, 10).len(), 1);
        assert_eq!(s.list(None, Some("x@y:1"), 10).len(), 1);
        assert_eq!(s.list(Some("users"), Some("x@y:1"), 10).len(), 0);
        assert_eq!(s.list(Some("  "), None, 10).len(), 2);
        assert_eq!(s.list(None, None, 1).len(), 1);
    }

    #[test]
    fn statements_with_passwords_are_not_kept() {
        for sql in [
            "ALTER ROLE bob PASSWORD 'hunter2'",
            "create user x with encrypted password 'a'",
            "ALTER USER x PASSWORD = 'a'",
            "alter role x password $$a$$",
            "alter role x password E'a'",
            "CREATE SUBSCRIPTION s CONNECTION 'host=h password=abc' PUBLICATION p",
        ] {
            assert!(looks_secret(sql), "{sql}");
        }
        for sql in [
            "select password from users",
            "select password_hash, password FROM u WHERE password IS NULL",
            "alter role x password null",
        ] {
            assert!(!looks_secret(sql), "{sql}");
        }
        let (_d, s) = store();
        assert!(!s.add(input("ALTER ROLE bob PASSWORD 'hunter2'")).unwrap());
        assert!(s.list(None, None, 10).is_empty());
    }

    #[test]
    fn empty_and_huge_statements_are_skipped() {
        let (_d, s) = store();
        assert!(!s.add(input("  \n")).unwrap());
        assert!(!s.add(input(&"x".repeat(MAX_SQL_BYTES + 1))).unwrap());
        assert!(s.add(input(&"x".repeat(MAX_SQL_BYTES))).unwrap());
    }

    #[test]
    fn keeps_only_the_newest_thousand() {
        let (_d, s) = store();
        for i in 0..(MAX_ENTRIES + 5) {
            s.add(input(&format!("select {i}"))).unwrap();
        }
        let l = s.list(None, None, usize::MAX);
        assert_eq!(l.len(), MAX_ENTRIES);
        assert_eq!(l[0].sql, format!("select {}", MAX_ENTRIES + 4));
        assert_eq!(l.last().unwrap().sql, "select 5");
    }

    #[test]
    fn delete_and_clear() {
        let (_d, s) = store();
        s.add(input("select 1")).unwrap();
        s.add(input("select 2")).unwrap();
        let id = s.list(Some("select 1"), None, 1)[0].id;
        s.delete(id).unwrap();
        assert_eq!(s.list(None, None, 10).len(), 1);
        let mut other = input("select 3");
        other.server = "x@y:1".into();
        s.add(other).unwrap();
        s.clear(Some("x@y:1")).unwrap();
        assert_eq!(s.list(None, None, 10).len(), 1);
        s.clear(None).unwrap();
        assert!(s.list(None, None, 10).is_empty());
    }

    #[test]
    fn a_damaged_file_is_set_aside_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        std::fs::write(&path, "{ not json").unwrap();
        let s = HistoryStore::new(&path);
        assert!(s.list(None, None, 10).is_empty());
        assert!(s.add(input("select 1")).unwrap());
        assert!(dir.path().join("history.json.bad").exists());
        assert_eq!(s.list(None, None, 10).len(), 1);
    }
}
