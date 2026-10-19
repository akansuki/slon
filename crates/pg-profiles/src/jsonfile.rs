//! A small JSON file that is replaced atomically. Used for data that is nice to keep but
//! not precious (query history, open tabs): a damaged file is moved aside and the app
//! starts empty instead of refusing to work.

use std::fs;
use std::path::PathBuf;

use serde::{de::DeserializeOwned, Serialize};

use pg_core::{ErrorKind, PgError};

pub(crate) struct JsonFile {
    path: PathBuf,
}

impl JsonFile {
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub(crate) fn read<T: DeserializeOwned + Default>(&self) -> T {
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(_) => return T::default(),
        };
        match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => {
                // Keep the unreadable file for inspection, but do not trip over it again.
                let _ = fs::rename(&self.path, self.path.with_extension("json.bad"));
                T::default()
            }
        }
    }

    pub(crate) fn write<T: Serialize>(&self, data: &T) -> Result<(), PgError> {
        let fail = |what: &str, e: std::io::Error| {
            PgError::new(
                ErrorKind::Storage,
                format!("Could not {what} {}: {e}", self.path.display()),
            )
        };
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| fail("create the folder for", e))?;
        }
        let json = serde_json::to_string(data).expect("serializes");
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| fail("write", e))?;
        fs::rename(&tmp, &self.path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            fail("replace", e)
        })
    }
}
