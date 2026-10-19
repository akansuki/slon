use serde::Serialize;
use std::error::Error as StdError;
use tokio_postgres::error::{ErrorPosition, SqlState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    /// Could not connect, authenticate, or the connection was lost.
    Connection,
    /// The server rejected a statement.
    Query,
    /// The statement was cancelled by the user (SQLSTATE 57014).
    Cancelled,
    /// Another query is already running in this tab.
    Busy,
    /// Unknown connection/tab id or similar client-side misuse.
    NotFound,
    /// Saved connections or the system credential store could not be read or written.
    Storage,
}

/// Structured error sent to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PgError {
    pub kind: ErrorKind,
    pub message: String,
    pub sqlstate: Option<String>,
    pub detail: Option<String>,
    pub hint: Option<String>,
    /// 1-based character offset into the submitted SQL, when the server reports one.
    pub position: Option<u32>,
}

impl PgError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            sqlstate: None,
            detail: None,
            hint: None,
            position: None,
        }
    }

    pub(crate) fn not_found(what: &str, id: u64) -> Self {
        Self::new(ErrorKind::NotFound, format!("Unknown {what} #{id}"))
    }

    /// Convert a driver error. `ctx` is the kind to use unless the server says the
    /// statement was cancelled.
    pub(crate) fn from_driver(e: &tokio_postgres::Error, ctx: ErrorKind) -> Self {
        if let Some(db) = e.as_db_error() {
            let kind = if *db.code() == SqlState::QUERY_CANCELED {
                ErrorKind::Cancelled
            } else {
                ctx
            };
            let message = if kind == ErrorKind::Cancelled {
                "Query cancelled".to_string()
            } else {
                db.message().to_string()
            };
            return Self {
                kind,
                message,
                sqlstate: Some(db.code().code().to_string()),
                detail: db.detail().map(str::to_string),
                hint: db.hint().map(str::to_string),
                position: match db.position() {
                    Some(ErrorPosition::Original(p)) => Some(*p),
                    _ => None,
                },
            };
        }
        Self::new(ctx, error_chain(e))
    }
}

impl std::fmt::Display for PgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl StdError for PgError {}

/// Join an error and its sources ("error connecting to server: Connection refused (os error 111)").
pub(crate) fn error_chain(e: &dyn StdError) -> String {
    let mut out = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        let m = c.to_string();
        if !out.contains(&m) {
            out.push_str(": ");
            out.push_str(&m);
        }
        cur = c.source();
    }
    out
}
