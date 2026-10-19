use std::collections::HashMap;
use std::sync::Mutex;

/// Why a secret could not be read or written. The text is meant for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretError(pub String);

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SecretError {}

/// Where passwords live. The profile file never sees them.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, key: &str, value: &str) -> Result<(), SecretError>;
    /// Deleting a secret that does not exist is not an error.
    fn delete(&self, key: &str) -> Result<(), SecretError>;
}

/// The operating system's credential store: Windows Credential Manager, macOS
/// Keychain, or the Secret Service on Linux.
pub struct KeyringSecrets {
    service: String,
}

impl KeyringSecrets {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, key).map_err(describe)
    }
}

fn describe(e: keyring::Error) -> SecretError {
    SecretError(format!("The system credential store reported: {e}"))
}

impl SecretStore for KeyringSecrets {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        match self.entry(key)?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(describe(e)),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
        self.entry(key)?.set_password(value).map_err(describe)
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(describe(e)),
        }
    }
}

/// Keeps secrets in memory only. For tests, and for debug builds that must not touch
/// the real credential store.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl MemorySecrets {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl SecretStore for MemorySecrets {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}
