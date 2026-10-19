use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use pg_core::{ErrorKind, PgError, SslMode};
use serde::{Deserialize, Serialize};

use crate::secrets::{SecretError, SecretStore};

const FILE_VERSION: u32 = 1;

/// A saved connection as shown in the UI. Contains no secret: whether a password is
/// stored is only a flag, the password itself lives in the credential store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    #[serde(default)]
    pub ssl_mode: SslMode,
    /// PEM file with the CA to trust for the verifying SSL modes.
    #[serde(default)]
    pub ssl_root_cert: Option<String>,
    #[serde(default)]
    pub has_password: bool,
    /// Unix seconds of the last successful connect.
    #[serde(default)]
    pub last_used_at: Option<u64>,
}

/// What the UI sends when saving.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    /// `None` creates a new profile, `Some` updates that one.
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    #[serde(default)]
    pub ssl_mode: SslMode,
    #[serde(default)]
    pub ssl_root_cert: Option<String>,
    /// A non-empty value stores (or replaces) the saved password. `None` or empty keeps
    /// whatever is saved.
    pub password: Option<String>,
    /// Remove the saved password.
    #[serde(default)]
    pub forget_password: bool,
}

// Manual Debug so a password can never end up in a log line.
impl std::fmt::Debug for ProfileInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileInput")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("ssl_mode", &self.ssl_mode)
            .field("ssl_root_cert", &self.ssl_root_cert)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("forget_password", &self.forget_password)
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct FileData {
    version: u32,
    profiles: Vec<Profile>,
}

impl Default for FileData {
    fn default() -> Self {
        Self {
            version: FILE_VERSION,
            profiles: Vec::new(),
        }
    }
}

fn storage(message: impl Into<String>) -> PgError {
    PgError::new(ErrorKind::Storage, message)
}

fn secret_key(id: &str) -> String {
    format!("connection:{id}")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Saved connections: settings in a JSON file, passwords in a [`SecretStore`].
pub struct ProfileStore {
    path: PathBuf,
    secrets: Box<dyn SecretStore>,
    /// Serialises read-modify-write cycles within this process.
    lock: Mutex<()>,
}

impl ProfileStore {
    pub fn new(path: impl Into<PathBuf>, secrets: Box<dyn SecretStore>) -> Self {
        Self {
            path: path.into(),
            secrets,
            lock: Mutex::new(()),
        }
    }

    fn read(&self) -> Result<FileData, PgError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(FileData::default()),
            Err(e) => {
                return Err(storage(format!(
                    "Could not read saved connections ({}): {e}",
                    self.path.display()
                )))
            }
        };
        let data: FileData = serde_json::from_str(&text).map_err(|e| {
            // Never overwrite a file we cannot understand: that would destroy the user's data.
            storage(format!(
                "The saved connections file is damaged ({}): {e}. Fix or delete it to continue.",
                self.path.display()
            ))
        })?;
        if data.version > FILE_VERSION {
            return Err(storage(format!(
                "The saved connections file ({}) was written by a newer version of this app.",
                self.path.display()
            )));
        }
        Ok(data)
    }

    /// Write to a temporary file, then rename over the real one, so a crash can never
    /// leave a half-written file behind.
    fn write(&self, data: &FileData) -> Result<(), PgError> {
        let fail = |what: &str, e: std::io::Error| {
            storage(format!(
                "Could not {what} saved connections ({}): {e}",
                self.path.display()
            ))
        };
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| fail("create the folder for", e))?;
        }
        let json = serde_json::to_string_pretty(data).expect("profiles serialize");
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| fail("write", e))?;
        fs::rename(&tmp, &self.path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            fail("replace", e)
        })
    }

    pub fn list(&self) -> Result<Vec<Profile>, PgError> {
        let _guard = self.lock.lock().unwrap();
        let mut profiles = self.read()?.profiles;
        // Most recently used first, then alphabetical.
        profiles.sort_by(|a, b| {
            b.last_used_at
                .cmp(&a.last_used_at)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(profiles)
    }

    /// Create or update a profile. A password change and the file update succeed or fail
    /// together.
    pub fn save(&self, input: ProfileInput) -> Result<Profile, PgError> {
        let host = input.host.trim().to_string();
        let user = input.user.trim().to_string();
        let database = input.database.trim().to_string();
        if host.is_empty() {
            return Err(storage("Host is required"));
        }
        if user.is_empty() {
            return Err(storage("User name is required"));
        }
        if input.port == 0 {
            return Err(storage("Port must be between 1 and 65535"));
        }
        let name = match input.name.trim() {
            "" => format!(
                "{user}@{host}/{}",
                if database.is_empty() {
                    &user
                } else {
                    &database
                }
            ),
            n => n.to_string(),
        };

        let _guard = self.lock.lock().unwrap();
        let mut data = self.read()?;

        let existing =
            match &input.id {
                Some(id) => Some(data.profiles.iter().position(|p| &p.id == id).ok_or_else(
                    || {
                        PgError::new(
                            ErrorKind::NotFound,
                            "That saved connection no longer exists",
                        )
                    },
                )?),
                None => None,
            };
        let id = match existing {
            Some(i) => data.profiles[i].id.clone(),
            None => uuid::Uuid::new_v4().to_string(),
        };
        let key = secret_key(&id);

        // Change the secret first, remembering how to undo it.
        let mut has_password = existing
            .map(|i| data.profiles[i].has_password)
            .unwrap_or(false);
        let mut undo: Option<Option<String>> = None; // Some(previous value) once the secret was touched
        let new_password = input.password.as_deref().filter(|p| !p.is_empty());
        if new_password.is_some() || input.forget_password {
            let previous = self.secrets.get(&key).map_err(secret_err)?;
            match new_password {
                Some(p) => {
                    self.secrets.set(&key, p).map_err(secret_err)?;
                    has_password = true;
                }
                None => {
                    self.secrets.delete(&key).map_err(secret_err)?;
                    has_password = false;
                }
            }
            undo = Some(previous);
        }

        let profile = Profile {
            id,
            name,
            host,
            port: input.port,
            database,
            user,
            ssl_mode: input.ssl_mode,
            ssl_root_cert: input
                .ssl_root_cert
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string),
            has_password,
            last_used_at: existing.and_then(|i| data.profiles[i].last_used_at),
        };
        match existing {
            Some(i) => data.profiles[i] = profile.clone(),
            None => data.profiles.push(profile.clone()),
        }

        if let Err(e) = self.write(&data) {
            // Put the credential store back the way it was.
            if let Some(previous) = undo {
                let _ = match previous {
                    Some(p) => self.secrets.set(&key, &p),
                    None => self.secrets.delete(&key),
                };
            }
            return Err(e);
        }
        Ok(profile)
    }

    pub fn delete(&self, id: &str) -> Result<(), PgError> {
        let _guard = self.lock.lock().unwrap();
        let mut data = self.read()?;
        let idx = data
            .profiles
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| {
                PgError::new(
                    ErrorKind::NotFound,
                    "That saved connection no longer exists",
                )
            })?;
        // Secret first: if it cannot be removed the profile stays, and the user can retry
        // instead of ending up with an orphaned credential nobody can see.
        self.secrets.delete(&secret_key(id)).map_err(secret_err)?;
        data.profiles.remove(idx);
        self.write(&data)
    }

    /// The saved password of profile `id`, if one is stored.
    ///
    /// `Ok(None)` means the profile has no saved password. An error means it should have
    /// one but the credential store does not (it was removed outside the app).
    pub fn saved_password(&self, id: &str) -> Result<Option<String>, PgError> {
        let has_password = {
            let _guard = self.lock.lock().unwrap();
            self.read()?
                .profiles
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| {
                    PgError::new(
                        ErrorKind::NotFound,
                        "That saved connection no longer exists",
                    )
                })?
                .has_password
        };
        if !has_password {
            return Ok(None);
        }
        match self.secrets.get(&secret_key(id)).map_err(secret_err)? {
            Some(p) => Ok(Some(p)),
            None => Err(storage(
                "The saved password is no longer in the system credential store. Enter it again.",
            )),
        }
    }

    /// The password to use when connecting with profile `id` and a blank password field.
    ///
    /// The saved password is only released when the target is still the one the profile
    /// describes (host, port and user); otherwise editing the host in the form would send
    /// the saved password to a different server. `Ok(None)` means "ask the user".
    pub fn password_for_connect(
        &self,
        id: &str,
        host: &str,
        port: u16,
        user: &str,
    ) -> Result<Option<String>, PgError> {
        let matches = {
            let _guard = self.lock.lock().unwrap();
            match self.read()?.profiles.iter().find(|p| p.id == id) {
                Some(p) => p.host == host.trim() && p.port == port && p.user == user.trim(),
                None => false,
            }
        };
        if !matches {
            return Ok(None);
        }
        self.saved_password(id)
    }

    /// Record a successful connect. Best effort: failures are reported but harmless.
    pub fn touch(&self, id: &str) -> Result<(), PgError> {
        let _guard = self.lock.lock().unwrap();
        let mut data = self.read()?;
        if let Some(p) = data.profiles.iter_mut().find(|p| p.id == id) {
            p.last_used_at = Some(now());
            self.write(&data)?;
        }
        Ok(())
    }
}

fn secret_err(e: SecretError) -> PgError {
    storage(e.0)
}
