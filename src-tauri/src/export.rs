//! Saving result files. The save dialog is opened here (not in the web view) and the only
//! paths `write_export` accepts are ones the dialog just handed out, so the page cannot be
//! used to write anywhere else on disk.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use pg_core::{ErrorKind, PgError};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

const EXTENSIONS: [&str; 5] = ["csv", "tsv", "json", "jsonl", "txt"];

#[derive(Default)]
pub struct Exports {
    allowed: Mutex<HashSet<PathBuf>>,
}

fn storage(msg: impl Into<String>) -> PgError {
    PgError::new(ErrorKind::Storage, msg)
}

fn extension_ok(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Debug builds only: a folder that replaces the dialog (used by the end-to-end test, which
/// cannot drive a native dialog).
fn forced_dir() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        std::env::var_os("SLON_EXPORT_DIR").map(PathBuf::from)
    } else {
        None
    }
}

/// Keep only the file name, so a suggestion can never point outside the chosen folder.
fn safe_name(name: &str) -> String {
    let base: String = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    if base.trim_matches('.').is_empty() {
        "result.csv".into()
    } else {
        base
    }
}

#[tauri::command]
pub async fn choose_export_path(
    app: AppHandle,
    state: State<'_, Exports>,
    suggested_name: String,
    label: String,
    extensions: Vec<String>,
) -> Result<Option<String>, PgError> {
    let name = safe_name(&suggested_name);
    let path = if let Some(dir) = forced_dir() {
        Some(dir.join(&name))
    } else {
        let exts: Vec<String> = extensions
            .into_iter()
            .filter(|e| EXTENSIONS.contains(&e.as_str()))
            .collect();
        let picked = tauri::async_runtime::spawn_blocking(move || {
            let mut dialog = app.dialog().file().set_file_name(&name);
            if !exts.is_empty() {
                let refs: Vec<&str> = exts.iter().map(String::as_str).collect();
                dialog = dialog.add_filter(&label, &refs);
            }
            dialog.blocking_save_file()
        })
        .await
        .map_err(|e| storage(format!("The save dialog failed: {e}")))?;
        match picked {
            None => None,
            Some(p) => Some(
                p.into_path()
                    .map_err(|e| storage(format!("Unusable path: {e}")))?,
            ),
        }
    };
    let Some(path) = path else { return Ok(None) };
    if !extension_ok(&path) {
        return Err(storage(
            "Choose a file name ending in .csv, .tsv, .json, .jsonl or .txt",
        ));
    }
    state.allowed.lock().unwrap().insert(path.clone());
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Writes a piece of the file: `append = false` starts it (replacing an existing file).
#[tauri::command]
pub fn write_export(
    state: State<'_, Exports>,
    path: String,
    text: String,
    append: bool,
) -> Result<(), PgError> {
    let path = PathBuf::from(path);
    if !state.allowed.lock().unwrap().contains(&path) {
        return Err(storage("That file was not chosen in the save dialog"));
    }
    let mut opts = OpenOptions::new();
    opts.create(true);
    if append {
        opts.append(true);
    } else {
        opts.write(true).truncate(true);
    }
    let mut file = opts
        .open(&path)
        .map_err(|e| storage(format!("Cannot write {}: {e}", path.display())))?;
    file.write_all(text.as_bytes())
        .map_err(|e| storage(format!("Cannot write {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_lose_their_folders_and_odd_characters() {
        assert_eq!(safe_name("../../etc/x.csv"), "x.csv");
        assert_eq!(safe_name("C:\\a\\b:c.csv"), "b_c.csv");
        assert_eq!(safe_name(".."), "result.csv");
    }

    #[test]
    fn only_known_extensions_pass() {
        assert!(extension_ok(Path::new("a.CSV")));
        assert!(!extension_ok(Path::new("a.exe")));
        assert!(!extension_ok(Path::new("a")));
    }
}
