//! Saving result files. The save dialog is opened here (not in the web view) and the only
//! paths these commands accept are ones the dialog just handed out, so the page cannot be
//! used to write anywhere else on disk.
//!
//! Two ways in, one formatter. The grid's *Export* sends the rows it holds, in the sorted
//! order on screen, a chunk per call. *Export all rows* never sends rows to the page at all:
//! [`FileSink`] is handed to the query engine and writes each row as it streams past, so the
//! size of the result is bounded by the disk rather than by memory.
//!
//! Either way the bytes go to `<name>.part` first and the file is renamed only once it is
//! complete, so a cancelled or failed export cannot leave something that looks finished.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use pg_core::{
    export_footer, export_header, export_row, json_keys, Cell, Column, ErrorKind, ExportColumn,
    ExportOptions, PgError, RowSink,
};
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

/// Where the bytes go until the file is known to be complete.
fn part_of(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

fn open_part(path: &Path, append: bool) -> Result<BufWriter<File>, PgError> {
    let part = part_of(path);
    let mut opts = OpenOptions::new();
    opts.create(true);
    if append {
        opts.append(true);
    } else {
        opts.write(true).truncate(true);
    }
    opts.open(&part)
        .map(BufWriter::new)
        .map_err(|e| storage(format!("Cannot write {}: {e}", part.display())))
}

/// Put the finished file in place of whatever was there before.
fn publish(path: &Path) -> Result<(), PgError> {
    std::fs::rename(part_of(path), path)
        .map_err(|e| storage(format!("Cannot save {}: {e}", path.display())))
}

fn check_allowed(state: &State<'_, Exports>, path: &Path) -> Result<(), PgError> {
    if state.allowed.lock().unwrap().contains(path) {
        Ok(())
    } else {
        Err(storage("That file was not chosen in the save dialog"))
    }
}

/// Writes a piece of the file for the grid's export. `start` is the index of the first row
/// in the whole file, which is what tells a JSON array whether a comma comes first;
/// `start == 0` begins the file and `last` closes and renames it.
#[tauri::command]
pub fn write_export_rows(
    state: State<'_, Exports>,
    path: String,
    columns: Vec<ExportColumn>,
    rows: Vec<Vec<Cell>>,
    options: ExportOptions,
    start: u64,
    last: bool,
) -> Result<(), PgError> {
    let path = PathBuf::from(path);
    check_allowed(&state, &path)?;

    let mut file = open_part(&path, start > 0)?;
    let mut text = if start == 0 {
        export_header(&columns, &options)
    } else {
        String::new()
    };
    let keys = json_keys(&columns);
    for (i, cells) in rows.iter().enumerate() {
        export_row(
            &mut text,
            cells,
            &columns,
            &keys,
            &options,
            start + i as u64,
        );
    }
    if last {
        text.push_str(&export_footer(start + rows.len() as u64, &options));
    }
    file.write_all(text.as_bytes())
        .map_err(|e| storage(format!("Cannot write {}: {e}", path.display())))?;
    file.flush()
        .map_err(|e| storage(format!("Cannot write {}: {e}", path.display())))?;
    drop(file);
    if last {
        publish(&path)?;
    }
    Ok(())
}

/// Writes rows to a file as the query streams them, for *Export all rows*.
///
/// Rows never reach the page, so the grid's cap does not apply and the result can be far
/// larger than memory. One buffer is reused for every row; the `BufWriter` decides when that
/// actually reaches the disk.
pub struct FileSink {
    path: PathBuf,
    /// Taken out and dropped before the rename: Windows will not rename a file that is
    /// still open.
    file: Option<BufWriter<File>>,
    options: ExportOptions,
    columns: Vec<ExportColumn>,
    keys: Vec<String>,
    buf: String,
    rows: u64,
    started: bool,
}

impl FileSink {
    pub fn create(path: PathBuf, options: ExportOptions) -> Result<Self, PgError> {
        let file = open_part(&path, false)?;
        Ok(Self {
            path,
            file: Some(file),
            options,
            columns: Vec::new(),
            keys: Vec::new(),
            buf: String::new(),
            rows: 0,
            started: false,
        })
    }

    /// How many rows reached the file.
    pub fn rows(&self) -> u64 {
        self.rows
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Throw away a file that was never finished. Called when the query failed, so that a
    /// partial `.part` is not left lying beside the real one.
    pub fn discard(mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(part_of(&self.path));
    }

    fn write(&mut self) -> Result<(), PgError> {
        let path = &self.path;
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| storage(format!("{} is already closed", path.display())))?;
        file.write_all(self.buf.as_bytes())
            .map_err(|e| storage(format!("Cannot write {}: {e}", path.display())))
    }
}

impl RowSink for FileSink {
    fn start(&mut self, columns: &[Column]) -> Result<(), PgError> {
        if self.started {
            // The engine refuses scripts before running them, so this cannot normally
            // happen; two result sets in one file would be nonsense, so say so rather than
            // quietly concatenating them.
            return Err(storage(
                "That statement produced more than one result, which cannot go in one file.",
            ));
        }
        self.started = true;
        self.columns = columns.iter().map(ExportColumn::from).collect();
        self.keys = json_keys(&self.columns);
        self.buf = export_header(&self.columns, &self.options);
        self.write()
    }

    fn row(&mut self, cells: &[Cell]) -> Result<(), PgError> {
        self.buf.clear();
        export_row(
            &mut self.buf,
            cells,
            &self.columns,
            &self.keys,
            &self.options,
            self.rows,
        );
        self.rows += 1;
        self.write()
    }

    fn finish(&mut self) -> Result<(), PgError> {
        self.buf = export_footer(self.rows, &self.options);
        self.write()?;
        let mut file = self
            .file
            .take()
            .ok_or_else(|| storage("The export file is already closed"))?;
        file.flush()
            .map_err(|e| storage(format!("Cannot write {}: {e}", self.path.display())))?;
        drop(file);
        publish(&self.path)
    }
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
