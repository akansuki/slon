//! The query tabs that were open on a server, so they come back after reconnecting.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use pg_core::PgError;

use crate::jsonfile::JsonFile;

/// More tabs than this are not remembered.
pub const MAX_TABS: usize = 50;
/// Servers whose tabs are remembered; the least recently saved are forgotten beyond this.
pub const MAX_SERVERS: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedTab {
    pub title: String,
    pub database: String,
    pub sql: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub tabs: Vec<SavedTab>,
    /// Index into `tabs` of the tab that was in front.
    pub active: Option<usize>,
    /// Unix milliseconds, filled in when saved.
    #[serde(default)]
    pub saved_at: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Data {
    servers: BTreeMap<String, Workspace>,
}

pub struct WorkspaceStore {
    file: JsonFile,
    lock: Mutex<()>,
}

impl WorkspaceStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            file: JsonFile::new(path),
            lock: Mutex::new(()),
        }
    }

    pub fn load(&self, server: &str) -> Option<Workspace> {
        let _g = self.lock.lock().unwrap();
        let data: Data = self.file.read();
        data.servers.get(server).cloned()
    }

    /// An empty workspace removes the entry.
    pub fn save(&self, server: &str, mut ws: Workspace) -> Result<(), PgError> {
        let _g = self.lock.lock().unwrap();
        let mut data: Data = self.file.read();
        ws.tabs.truncate(MAX_TABS);
        ws.active = ws.active.filter(|&i| i < ws.tabs.len());
        if ws.tabs.is_empty() {
            data.servers.remove(server);
        } else {
            ws.saved_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            data.servers.insert(server.to_string(), ws);
            while data.servers.len() > MAX_SERVERS {
                let oldest = data
                    .servers
                    .iter()
                    .min_by_key(|(_, w)| w.saved_at)
                    .map(|(k, _)| k.clone());
                match oldest {
                    Some(k) => data.servers.remove(&k),
                    None => break,
                };
            }
        }
        self.file.write(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(sql: &str) -> SavedTab {
        SavedTab {
            title: "t".into(),
            database: "app".into(),
            sql: sql.into(),
        }
    }

    #[test]
    fn saves_loads_and_keeps_servers_apart() {
        let dir = tempfile::tempdir().unwrap();
        let s = WorkspaceStore::new(dir.path().join("w.json"));
        assert!(s.load("a@h:1").is_none());
        s.save(
            "a@h:1",
            Workspace {
                tabs: vec![tab("select 1"), tab("select 2")],
                active: Some(1),
                saved_at: 0,
            },
        )
        .unwrap();
        s.save(
            "b@h:1",
            Workspace {
                tabs: vec![tab("other")],
                active: None,
                saved_at: 0,
            },
        )
        .unwrap();
        let again = WorkspaceStore::new(dir.path().join("w.json"));
        let a = again.load("a@h:1").unwrap();
        assert_eq!(a.tabs.len(), 2);
        assert_eq!(a.active, Some(1));
        assert!(a.saved_at > 0);
        assert_eq!(again.load("b@h:1").unwrap().tabs[0].sql, "other");
    }

    #[test]
    fn empty_workspace_forgets_the_server_and_bad_indexes_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let s = WorkspaceStore::new(dir.path().join("w.json"));
        s.save(
            "a@h:1",
            Workspace {
                tabs: vec![tab("x")],
                active: Some(7),
                saved_at: 0,
            },
        )
        .unwrap();
        assert_eq!(s.load("a@h:1").unwrap().active, None);
        s.save("a@h:1", Workspace::default()).unwrap();
        assert!(s.load("a@h:1").is_none());
    }

    #[test]
    fn too_many_tabs_are_cut() {
        let dir = tempfile::tempdir().unwrap();
        let s = WorkspaceStore::new(dir.path().join("w.json"));
        let tabs = (0..MAX_TABS + 10).map(|i| tab(&i.to_string())).collect();
        s.save(
            "a@h:1",
            Workspace {
                tabs,
                active: Some(MAX_TABS + 3),
                saved_at: 0,
            },
        )
        .unwrap();
        let w = s.load("a@h:1").unwrap();
        assert_eq!(w.tabs.len(), MAX_TABS);
        assert_eq!(w.active, None);
    }
}
