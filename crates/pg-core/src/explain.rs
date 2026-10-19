use std::time::Instant;

use serde::Serialize;
use tokio_postgres::error::SqlState;
use tokio_postgres::SimpleQueryMessage;

use crate::error::{ErrorKind, PgError};
use crate::session::Session;

/// A query plan as the server reports it (`EXPLAIN (FORMAT JSON)`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainResult {
    /// The JSON document exactly as PostgreSQL produced it. Parsing and presenting it is
    /// the frontend's job.
    pub plan: String,
    pub analyzed: bool,
    pub elapsed_ms: u64,
}

fn query_err(e: &tokio_postgres::Error) -> PgError {
    PgError::from_driver(e, ErrorKind::Query)
}

/// Run `sql` through EXPLAIN.
///
/// * Plain EXPLAIN only plans the statement.
/// * `analyze` really executes it, so it is wrapped in a transaction that is always rolled
///   back: an `EXPLAIN ANALYZE DELETE ...` must not delete anything. If the tab is already
///   inside a transaction of the user's, a savepoint is used instead so that transaction
///   survives. Effects that PostgreSQL does not roll back (sequence values, side effects of
///   functions that talk to the outside world) still happen.
///
/// Only a single statement can be explained; a script is rejected before anything runs,
/// because `EXPLAIN` would otherwise execute the statements that follow the first one.
pub async fn run_explain(
    session: &Session,
    sql: &str,
    analyze: bool,
) -> Result<ExplainResult, PgError> {
    let _guard = session.begin_query()?;
    let started = Instant::now();

    let body = sql.trim_end().trim_end_matches(';').trim_end();
    if body.trim().is_empty() {
        return Err(PgError::new(ErrorKind::Query, "Nothing to explain."));
    }
    let prefix = if analyze {
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) "
    } else {
        "EXPLAIN (FORMAT JSON) "
    };
    let wrapped = format!("{prefix}{body}");
    let prefix_chars = prefix.chars().count() as u32;

    // Parsing the wrapped text without running it rejects scripts ("cannot insert multiple
    // commands into a prepared statement") and reports mistakes in the statement with a
    // position, which we translate back to the user's text.
    if let Err(e) = session.client.prepare(&wrapped).await {
        let mut err = query_err(&e);
        if err.message.contains("multiple commands") {
            err.message = "EXPLAIN works on one statement at a time. Select the statement you want to explain.".into();
            err.position = None;
        } else {
            err.position = err
                .position
                .and_then(|p| p.checked_sub(prefix_chars))
                .filter(|p| *p >= 1);
        }
        return Err(err);
    }

    let client = &session.client;
    let run = || async {
        let msgs = client.simple_query(&wrapped).await.map_err(|e| {
            let mut err = query_err(&e);
            err.position = err
                .position
                .and_then(|p| p.checked_sub(prefix_chars))
                .filter(|p| *p >= 1);
            err
        })?;
        msgs.iter()
            .find_map(|m| match m {
                SimpleQueryMessage::Row(r) => r.get(0).map(str::to_owned),
                _ => None,
            })
            .ok_or_else(|| PgError::new(ErrorKind::Query, "The server returned no plan."))
    };

    let plan = if !analyze {
        run().await?
    } else {
        // A savepoint only works inside a transaction block (25P01 otherwise): that tells us
        // whether the user has a transaction open without disturbing it.
        let in_user_txn = match client.simple_query("SAVEPOINT slon_explain").await {
            Ok(_) => true,
            Err(e) if e.code() == Some(&SqlState::NO_ACTIVE_SQL_TRANSACTION) => {
                client
                    .simple_query("BEGIN")
                    .await
                    .map_err(|e| query_err(&e))?;
                false
            }
            // For example an already aborted transaction: report it as is.
            Err(e) => return Err(query_err(&e)),
        };
        let result = run().await;
        let cleanup = if in_user_txn {
            "ROLLBACK TO SAVEPOINT slon_explain; RELEASE SAVEPOINT slon_explain"
        } else {
            "ROLLBACK"
        };
        let cleaned = client.simple_query(cleanup).await;
        let plan = result?;
        if let Err(e) = cleaned {
            return Err(query_err(&e));
        }
        plan
    };

    Ok(ExplainResult {
        plan,
        analyzed: analyze,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}
