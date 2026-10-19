use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use tokio_postgres::config::SslMode as PgSslMode;
use tokio_postgres::{AsyncMessage, CancelToken, Client, Config};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::config::{ConnectionConfig, SslMode};
use crate::error::{error_chain, ErrorKind, PgError};
use crate::notice::{NoticeBox, ServerMessage};
use crate::tls::{explain_tls_failure, make_connector};

/// One live PostgreSQL connection. Every query tab owns one, so cancelling or
/// breaking one tab never affects another.
pub struct Session {
    pub(crate) client: Client,
    cancel: CancelHandle,
    /// Server messages (`RAISE NOTICE`, warnings) waiting to be shown.
    pub(crate) notices: Arc<NoticeBox>,
    closed_reason: Arc<Mutex<Option<String>>>,
    busy: AtomicBool,
}

/// Everything needed to send a cancel request without touching the session.
/// Cheap to clone, so it can be moved into a spawned task.
#[derive(Clone)]
pub(crate) struct CancelHandle {
    token: CancelToken,
    tls: MakeRustlsConnect,
}

impl CancelHandle {
    pub(crate) async fn cancel(&self) -> Result<(), PgError> {
        self.token
            .cancel_query(self.tls.clone())
            .await
            .map_err(|e| PgError::from_driver(&e, ErrorKind::Connection))
    }
}

pub(crate) struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Session {
    pub async fn connect(cfg: &ConnectionConfig) -> Result<Self, PgError> {
        if cfg.host.trim().is_empty() {
            return Err(PgError::new(ErrorKind::Connection, "Host is required"));
        }
        if cfg.user.trim().is_empty() {
            return Err(PgError::new(ErrorKind::Connection, "User name is required"));
        }

        let mut pg = Config::new();
        pg.host(cfg.host.trim())
            .port(cfg.port)
            .user(cfg.user.trim())
            .password(&cfg.password)
            .connect_timeout(Duration::from_secs(10))
            .application_name("slon")
            .ssl_mode(match cfg.ssl_mode {
                SslMode::Disable => PgSslMode::Disable,
                SslMode::Prefer => PgSslMode::Prefer,
                SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => PgSslMode::Require,
            });
        if !cfg.database.trim().is_empty() {
            pg.dbname(cfg.database.trim());
        }

        let tls = make_connector(cfg)?;
        let (client, connection) = pg.connect(tls.clone()).await.map_err(|e| {
            let mut err = PgError::from_driver(&e, ErrorKind::Connection);
            explain_tls_failure(&mut err, cfg);
            err
        })?;

        let closed_reason = Arc::new(Mutex::new(None));
        let reason_slot = closed_reason.clone();
        let notices = Arc::new(NoticeBox::default());
        let sink = notices.clone();
        tokio::spawn(async move {
            // Driving the connection through `poll_message` (instead of just awaiting it) is what
            // hands us the NOTICE/WARNING messages; awaiting it silently throws them away.
            let mut connection = connection;
            let mut messages = futures_util::stream::poll_fn(move |cx| connection.poll_message(cx));
            while let Some(m) = messages.next().await {
                match m {
                    Ok(AsyncMessage::Notice(n)) => sink.push(ServerMessage::from_db(&n)),
                    Ok(_) => {}
                    Err(e) => {
                        *reason_slot.lock().unwrap() = Some(error_chain(&e));
                        break;
                    }
                }
            }
        });

        let cancel = CancelHandle {
            token: client.cancel_token(),
            tls,
        };
        Ok(Self {
            client,
            cancel,
            notices,
            closed_reason,
            busy: AtomicBool::new(false),
        })
    }

    pub fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    /// Fails with a `Connection` error if the socket has gone away.
    pub(crate) fn check_open(&self) -> Result<(), PgError> {
        if !self.client.is_closed() {
            return Ok(());
        }
        let reason = self.closed_reason.lock().unwrap().clone();
        Err(PgError::new(
            ErrorKind::Connection,
            match reason {
                Some(r) => format!("Connection lost: {r}"),
                None => "Connection closed".to_string(),
            },
        ))
    }

    pub(crate) fn begin_query(&self) -> Result<BusyGuard<'_>, PgError> {
        self.check_open()?;
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err(PgError::new(
                ErrorKind::Busy,
                "A query is already running in this tab",
            ));
        }
        Ok(BusyGuard(&self.busy))
    }

    pub(crate) fn cancel_handle(&self) -> CancelHandle {
        self.cancel.clone()
    }

    /// Ask the server to cancel whatever this session is running.
    pub async fn cancel(&self) -> Result<(), PgError> {
        self.cancel.cancel().await
    }

    pub async fn server_version(&self) -> Result<String, PgError> {
        self.check_open()?;
        let row = self
            .client
            .query_one("SHOW server_version", &[])
            .await
            .map_err(|e| PgError::from_driver(&e, ErrorKind::Query))?;
        Ok(row.get::<_, String>(0))
    }
}
