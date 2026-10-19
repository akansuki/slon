use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;
use tokio::sync::Notify;
use tokio_postgres::error::DbError;

/// A message the server sent next to the results: `RAISE NOTICE`, warnings such as
/// "there is no transaction in progress", `VACUUM VERBOSE` output, and so on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerMessage {
    /// NOTICE, WARNING, INFO, LOG, DEBUG1..5 (always the English, non-localized name when the
    /// server provides it).
    pub severity: String,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
    /// Call stack of a message raised inside a function ("PL/pgSQL function f() line 3 at RAISE").
    pub context: Option<String>,
    pub sqlstate: String,
    /// Milliseconds since the statement started, filled in when the message is delivered.
    pub elapsed_ms: u64,
}

impl ServerMessage {
    pub(crate) fn from_db(n: &DbError) -> Self {
        Self {
            severity: n
                .parsed_severity()
                .map(|s| s.to_string())
                .unwrap_or_else(|| n.severity().to_string()),
            message: n.message().to_string(),
            detail: n.detail().map(str::to_string),
            hint: n.hint().map(str::to_string),
            context: n.where_().map(str::to_string),
            sqlstate: n.code().code().to_string(),
            elapsed_ms: 0,
        }
    }

    /// The line that closes a list that had to be cut short.
    pub(crate) fn omitted(count: usize, elapsed_ms: u64) -> Self {
        let message = format!(
            "{count} more message{} not shown (limit of {CAPACITY} per run)",
            if count == 1 { " was" } else { "s were" }
        );
        Self {
            severity: "NOTICE".into(),
            message,
            detail: None,
            hint: None,
            context: None,
            sqlstate: "00000".into(),
            elapsed_ms,
        }
    }
}

/// More than this many messages per run are not delivered: a loop that raises a notice per
/// row must not be able to flood the UI or eat the memory of the app.
pub(crate) const CAPACITY: usize = 10_000;

#[derive(Default)]
struct Queue {
    items: VecDeque<(Instant, ServerMessage)>,
    dropped: usize,
}

/// Messages that arrived on a connection and have not been shown yet. The connection task
/// pushes, the running query takes them out in order.
#[derive(Default)]
pub(crate) struct NoticeBox {
    queue: Mutex<Queue>,
    wake: Notify,
}

impl NoticeBox {
    pub(crate) fn push(&self, message: ServerMessage) {
        {
            let mut q = self.queue.lock().unwrap();
            if q.items.len() >= CAPACITY {
                q.dropped += 1;
            } else {
                q.items.push_back((Instant::now(), message));
            }
        }
        // Store a permit even when nobody waits right now, so a message is never missed
        // between two polls of the query loop.
        self.wake.notify_one();
    }

    /// Resolves when something was pushed since the last time it resolved.
    pub(crate) async fn arrived(&self) {
        self.wake.notified().await;
    }

    /// Everything queued, oldest first, with `elapsed_ms` measured from `since`, plus how many
    /// messages did not fit into the queue.
    pub(crate) fn take(&self, since: Instant) -> (Vec<ServerMessage>, usize) {
        let (items, dropped) = {
            let mut q = self.queue.lock().unwrap();
            (std::mem::take(&mut q.items), std::mem::take(&mut q.dropped))
        };
        let out = items
            .into_iter()
            .map(|(at, mut m)| {
                m.elapsed_ms = at.saturating_duration_since(since).as_millis() as u64;
                m
            })
            .collect();
        (out, dropped)
    }

    /// Forget messages from before a statement starts.
    pub(crate) fn clear(&self) {
        let mut q = self.queue.lock().unwrap();
        q.items.clear();
        q.dropped = 0;
    }
}
