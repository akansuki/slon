use serde::{Deserialize, Serialize};

/// Mirrors libpq's `sslmode`.
///
/// `Prefer` and `Require` encrypt the connection but do not verify the server
/// certificate, which is also what libpq does for these modes. `VerifyCa` checks that
/// the certificate chains to a trusted CA; `VerifyFull` also checks that it was issued
/// for the host name used to connect.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SslMode {
    #[serde(rename = "disable")]
    Disable,
    #[default]
    #[serde(rename = "prefer")]
    Prefer,
    #[serde(rename = "require")]
    Require,
    #[serde(rename = "verify-ca")]
    VerifyCa,
    #[serde(rename = "verify-full")]
    VerifyFull,
}

impl SslMode {
    /// The server certificate is validated (not just used for encryption).
    pub fn verifies(self) -> bool {
        matches!(self, SslMode::VerifyCa | SslMode::VerifyFull)
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
    #[serde(default)]
    pub ssl_mode: SslMode,
    /// PEM file with the CA certificate(s) to trust for `VerifyCa` / `VerifyFull`.
    /// Empty or absent: the operating system's certificate store.
    #[serde(default)]
    pub ssl_root_cert: Option<String>,
}

impl ConnectionConfig {
    pub fn with_database(&self, database: &str) -> Self {
        Self {
            database: database.to_string(),
            ..self.clone()
        }
    }
}

// Manual Debug so the password can never end up in a log line.
impl std::fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .field("ssl_mode", &self.ssl_mode)
            .field("ssl_root_cert", &self.ssl_root_cert)
            .finish()
    }
}
