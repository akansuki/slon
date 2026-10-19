//! Saved connection profiles.
//!
//! Settings (host, port, user, ...) live in a JSON file. Passwords never do: they go to
//! the operating system's credential store (Windows Credential Manager on Windows).

mod history;
mod jsonfile;
mod secrets;
mod store;
mod workspace;

pub use history::{looks_secret, HistoryEntry, HistoryInput, HistoryStore};
pub use workspace::{SavedTab, Workspace, WorkspaceStore};

pub use secrets::{KeyringSecrets, MemorySecrets, SecretError, SecretStore};
pub use store::{Profile, ProfileInput, ProfileStore};
