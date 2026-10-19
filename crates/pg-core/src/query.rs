use std::time::Instant;

use futures_util::StreamExt;
use serde::Serialize;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::Type;
use tokio_postgres::SimpleQueryMessage;

use crate::error::{ErrorKind, PgError};
use crate::notice::{ServerMessage, CAPACITY};
use crate::session::Session;

/// How the frontend should compare values of a column when sorting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SortKind {
    Number,
    Boolean,
    Text,
    /// Type information was not available (multi-statement scripts).
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub name: String,
    pub type_name: Option<String>,
    pub sort_kind: SortKind,
}

fn sort_kind_for(t: &Type) -> SortKind {
    if matches!(
        *t,
        Type::INT2
            | Type::INT4
            | Type::INT8
            | Type::FLOAT4
            | Type::FLOAT8
            | Type::NUMERIC
            | Type::OID
    ) {
        SortKind::Number
    } else if *t == Type::BOOL {
        SortKind::Boolean
    } else {
        SortKind::Text
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum QueryEvent {
    /// A statement that returns rows has started.
    Columns {
        result_index: usize,
        columns: Vec<Column>,
    },
    /// A batch of rows for the result set most recently announced.
    Rows {
        result_index: usize,
        rows: Vec<Vec<Option<String>>>,
    },
    /// A statement finished.
    ResultDone {
        result_index: usize,
        has_columns: bool,
        /// Rows returned (SELECT) or affected (INSERT/UPDATE/DELETE) as reported by the server.
        rows_affected: u64,
        /// Rows actually delivered to the client; lower than `rows_affected` when truncated.
        row_count: usize,
        truncated: bool,
    },
    /// A message from the server (`RAISE NOTICE`, a warning, ...). Messages arrive in the order
    /// the server sent them, but not at a fixed position among the row events: the connection
    /// can run ahead of the consumer, so use `elapsed_ms` rather than event order to relate a
    /// message to a point in time.
    Message { message: ServerMessage },
    /// Terminal event: the whole script finished. Never emitted by `run_query`
    /// itself; the app layer sends it so that it is ordered after every row batch
    /// (a command's return value travels on a different path than channel messages).
    Finished { summary: QuerySummary },
    /// Terminal event: the script stopped with an error. See `Finished`.
    Failed { error: PgError },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuerySummary {
    pub elapsed_ms: u64,
    /// Number of statements the server completed.
    pub statements: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct QueryOptions {
    /// Rows kept per result set. Client-side sorting only sees these rows, so the
    /// UI shows a notice when a result is truncated.
    pub max_rows: usize,
    pub batch_size: usize,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            max_rows: 100_000,
            batch_size: 1_000,
        }
    }
}

/// Run `sql` on `session`, reporting progress through `emit`.
///
/// Uses the simple query protocol so that
///  * scripts with several statements work,
///  * the server formats every value as text, exactly like psql would.
///
/// Column types come from a `prepare` beforehand, which only succeeds for a single
/// statement; scripts fall back to untyped columns.
pub async fn run_query<F>(
    session: &Session,
    sql: &str,
    opts: &QueryOptions,
    mut emit: F,
) -> Result<QuerySummary, PgError>
where
    F: FnMut(QueryEvent),
{
    let _guard = session.begin_query()?;
    let started = Instant::now();
    // Whatever arrived before this statement belongs to something else.
    session.notices.clear();

    let described: Option<Vec<Column>> = session.client.prepare(sql).await.ok().map(|stmt| {
        stmt.columns()
            .iter()
            .map(|c| Column {
                name: c.name().to_string(),
                type_name: Some(c.type_().name().to_string()),
                sort_kind: sort_kind_for(c.type_()),
            })
            .collect()
    });
    // A successful prepare means exactly one statement.
    let single_statement = described.is_some();

    let stream = session
        .client
        .simple_query_raw(sql)
        .await
        .map_err(|e| PgError::from_driver(&e, ErrorKind::Query))?;
    futures_util::pin_mut!(stream);

    let mut result_index = 0usize;
    let mut in_result_set = false;
    let mut pending: Vec<Vec<Option<String>>> = Vec::new();
    let mut row_count = 0usize;
    let mut truncated = false;
    let mut any_truncated = false;
    let mut statements = 0usize;

    // Pass on every server message that has arrived so far (the connection task queues them).
    let mut delivered = 0usize;
    let mut omitted = 0usize;
    let mut last_message_ms = 0u64;
    macro_rules! messages {
        () => {
            let (batch, overflow) = session.notices.take(started);
            omitted += overflow;
            for message in batch {
                last_message_ms = message.elapsed_ms;
                if delivered < CAPACITY {
                    delivered += 1;
                    emit(QueryEvent::Message { message });
                } else {
                    omitted += 1;
                }
            }
        };
    }

    macro_rules! close_messages {
        () => {
            messages!();
            if omitted > 0 {
                emit(QueryEvent::Message {
                    message: ServerMessage::omitted(omitted, last_message_ms),
                });
            }
        };
    }

    macro_rules! flush {
        () => {
            if !pending.is_empty() {
                emit(QueryEvent::Rows {
                    result_index,
                    rows: std::mem::take(&mut pending),
                });
            }
        };
    }

    loop {
        let msg = tokio::select! {
            // Messages first: they were queued before the response that is ready now, and a
            // statement that only talks (a DO block reporting progress) has no other traffic.
            biased;
            _ = session.notices.arrived() => {
                messages!();
                continue;
            }
            next = stream.next() => match next {
                Some(m) => m,
                None => break,
            },
        };
        match msg {
            Ok(SimpleQueryMessage::RowDescription(cols)) => {
                let columns = match &described {
                    Some(d) if d.len() == cols.len() => d.clone(),
                    _ => cols
                        .iter()
                        .map(|c| Column {
                            name: c.name().to_string(),
                            type_name: None,
                            sort_kind: SortKind::Unknown,
                        })
                        .collect(),
                };
                in_result_set = true;
                emit(QueryEvent::Columns {
                    result_index,
                    columns,
                });
            }
            Ok(SimpleQueryMessage::Row(row)) => {
                if truncated {
                    continue; // draining what the server already sent
                }
                if row_count >= opts.max_rows {
                    truncated = true;
                    any_truncated = true;
                    flush!();
                    // For a single statement it is safe to stop the server early. For
                    // scripts we keep draining instead: an error would roll back the
                    // implicit transaction that wraps the whole script.
                    if single_statement {
                        let handle = session.cancel_handle();
                        tokio::spawn(async move {
                            let _ = handle.cancel().await;
                        });
                    }
                    continue;
                }
                pending.push(
                    (0..row.len())
                        .map(|i| row.get(i).map(str::to_owned))
                        .collect(),
                );
                row_count += 1;
                if pending.len() >= opts.batch_size {
                    flush!();
                }
            }
            Ok(SimpleQueryMessage::CommandComplete(n)) => {
                messages!();
                flush!();
                emit(QueryEvent::ResultDone {
                    result_index,
                    has_columns: in_result_set,
                    rows_affected: n,
                    row_count,
                    truncated,
                });
                statements += 1;
                result_index += 1;
                in_result_set = false;
                row_count = 0;
                truncated = false;
            }
            Ok(_) => {}
            Err(e) => {
                messages!();
                flush!();
                // The cancel we sent ourselves after hitting the row cap is not an error.
                if truncated && e.code() == Some(&SqlState::QUERY_CANCELED) {
                    emit(QueryEvent::ResultDone {
                        result_index,
                        has_columns: in_result_set,
                        rows_affected: row_count as u64,
                        row_count,
                        truncated: true,
                    });
                    statements += 1;
                    break;
                }
                close_messages!();
                return Err(PgError::from_driver(&e, ErrorKind::Query));
            }
        }
    }

    close_messages!();
    Ok(QuerySummary {
        elapsed_ms: started.elapsed().as_millis() as u64,
        statements,
        truncated: any_truncated,
    })
}
