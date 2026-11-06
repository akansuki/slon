//! Changing data straight from the results grid: deleting rows, and editing single cells.
//!
//! A grid row can only be deleted when it can be traced back to exactly one row of one
//! table. The server tells us which table each result column came from (`RowDescription`
//! carries a table OID and an attribute number per column), so there is no SQL to parse
//! and an aliased column is still matched correctly.
//!
//! Three rules decide whether the action is offered at all: the columns must come from a
//! single ordinary table, that table must have a primary key, and every primary-key column
//! must be present in the result.
//!
//! Every identifier and every value in the generated statement is quoted by the server
//! (`quote_ident`, `quote_nullable`), never by hand. The statement shown in the confirmation
//! is the statement that runs: `plan_delete` and `run_delete` build it the same way from
//! the same input, so the preview cannot drift from what is executed. The transaction
//! commands around it are plumbing and are not part of what is shown.
//!
//! An edit also carries the value the grid was showing into the WHERE clause, so a row that
//! somebody else changed in the meantime is reported instead of being overwritten.
//!
//! Latency matters here: a cell edit happens while the person waits, and on a remote server
//! every round trip is felt. So the catalog lookup, the value quoting and the transaction
//! probe are pipelined into one round trip, the statement carries its own `BEGIN`, and the
//! whole write costs three round trips rather than five.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio_postgres::error::SqlState;
use tokio_postgres::{SimpleQueryMessage, SimpleQueryRow};

use crate::error::{ErrorKind, PgError};
use crate::session::Session;

/// More rows than this in one go is almost certainly a mistake, and the statement would be
/// unreadable in the confirmation anyway.
pub const MAX_ROWS: usize = 1000;

const DELETE_SAVEPOINT: &str = "slon_delete";
const UPDATE_SAVEPOINT: &str = "slon_update";

const FINISH_FAILED: &str = "The statement ran, but Slon could not close the transaction, \
                             so the change may not have been saved. Run the query again to \
                             see what the table holds.";
const ROLLBACK_FAILED: &str = "Slon could not undo that, so this connection is still inside \
                               a transaction that has to be ended by hand. Run ROLLBACK \
                               before using this tab again.";

/// The table a result set can be deleted from, and the key that identifies a row in it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowIdentity {
    pub schema: String,
    pub table: String,
    /// Quoted by the server and ready to drop into SQL (`"shop"."orders"`).
    pub qualified: String,
    /// The primary-key columns, in key order.
    pub key: Vec<KeyColumn>,
    /// Every column of the table, so the grid knows which cells can be edited.
    pub columns: Vec<TableColumn>,
}

/// A column of the source table, and whether a cell of it can be written.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableColumn {
    pub name: String,
    /// Quoted by the server.
    pub quoted: String,
    pub attnum: i16,
    pub editable: bool,
    /// Why not, when it cannot be edited.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyColumn {
    pub name: String,
    /// Quoted by the server.
    pub quoted: String,
    /// Position in the table. Result columns carry the same number, which is how a key
    /// column is found in the result even when the query renamed it.
    pub attnum: i16,
}

/// Which rows to delete: the key values of each row, in key-column order.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRequest {
    pub table_oid: u32,
    pub rows: Vec<Vec<String>>,
}

/// The statement that would run, for the confirmation dialog.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePlan {
    pub sql: String,
    pub row_count: usize,
    pub identity: RowIdentity,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOutcome {
    pub sql: String,
    pub deleted: u64,
    pub elapsed_ms: u64,
}

/// One cell: which row, which column, the new value, and the value the grid was showing.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRequest {
    pub table_oid: u32,
    /// Position in the table of the column being written.
    pub column: i16,
    /// `None` sets the cell to NULL.
    pub value: Option<String>,
    /// What the grid was showing, used to detect a row somebody else changed meanwhile.
    pub old_value: Option<String>,
    /// The row's key values, in key order.
    pub key: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOutcome {
    pub sql: String,
    /// What the column holds now, read back from the server: writing `1.5` into a
    /// `numeric(10,2)` stores `1.50`, and the grid should show that rather than the text
    /// that was typed.
    pub stored: Option<String>,
    pub elapsed_ms: u64,
}

fn query_err(e: &tokio_postgres::Error) -> PgError {
    PgError::from_driver(e, ErrorKind::Query)
}

fn refuse(msg: impl Into<String>) -> PgError {
    PgError::new(ErrorKind::Query, msg)
}

/// Add a sentence about the connection's state without losing what the server said.
fn with_hint(mut err: PgError, note: &str) -> PgError {
    err.hint = Some(match err.hint {
        Some(h) => format!("{h}\n{note}"),
        None => note.to_string(),
    });
    err
}

// ---------------------------------------------------------------------------- catalog

/// Look up the table behind a result column's table OID, and its primary key.
pub async fn row_identity(session: &Session, table_oid: u32) -> Result<RowIdentity, PgError> {
    let _guard = session.begin_query()?;
    identity_of(session, table_oid).await
}

/// The catalog query itself, without the busy guard: the callers below already hold it.
async fn identity_of(session: &Session, table_oid: u32) -> Result<RowIdentity, PgError> {
    // The OID came from the server in a RowDescription, so it is a plain number and safe to
    // write into the statement.
    let sql = format!(
        "SELECT c.relkind::text,
                n.nspname,
                c.relname,
                quote_ident(n.nspname) || '.' || quote_ident(c.relname),
                a.attname,
                quote_ident(a.attname),
                a.attnum::text,
                a.attgenerated::text,
                a.attidentity::text,
                (SELECT array_position(i.indkey::int2[], a.attnum)
                   FROM pg_catalog.pg_index i
                  WHERE i.indrelid = c.oid AND i.indisprimary)::text
         FROM pg_catalog.pg_class c
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
         LEFT JOIN pg_catalog.pg_attribute a
                ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
         WHERE c.oid = {table_oid}
         ORDER BY a.attnum"
    );
    let msgs = session
        .client
        .simple_query(&sql)
        .await
        .map_err(|e| query_err(&e))?;
    let rows: Vec<_> = msgs
        .iter()
        .filter_map(|m| match m {
            SimpleQueryMessage::Row(r) => Some(r),
            _ => None,
        })
        .collect();

    let first = rows
        .first()
        .ok_or_else(|| refuse("That table no longer exists."))?;
    let relkind = first.get(0).unwrap_or("");
    match relkind {
        "r" | "p" => {}
        "v" | "m" => {
            return Err(refuse(
                "These rows come from a view, so Slon cannot tell which table row to delete.",
            ))
        }
        "f" => return Err(refuse("Foreign tables cannot be edited from the grid.")),
        _ => return Err(refuse("These rows do not come from a table.")),
    }

    let mut columns = Vec::new();
    // (position within the primary key, column) for the columns that make it up.
    let mut keyed: Vec<(i32, KeyColumn)> = Vec::new();
    for r in &rows {
        let (Some(name), Some(quoted), Some(attnum)) = (r.get(4), r.get(5), r.get(6)) else {
            continue; // a table with no columns at all
        };
        let attnum: i16 = attnum
            .parse()
            .map_err(|_| refuse("Unreadable column number."))?;
        // `attgenerated` is 's' for a stored generated column, `attidentity` 'a' for
        // GENERATED ALWAYS AS IDENTITY. Neither accepts a plain UPDATE; 'd' (BY DEFAULT)
        // does, so it stays editable.
        let reason = match (r.get(7).unwrap_or(""), r.get(8).unwrap_or("")) {
            ("s", _) => Some("a generated column".to_string()),
            (_, "a") => Some("an identity column defined as GENERATED ALWAYS".to_string()),
            _ => None,
        };
        columns.push(TableColumn {
            name: name.to_string(),
            quoted: quoted.to_string(),
            attnum,
            editable: reason.is_none(),
            reason,
        });
        if let Some(ord) = r.get(9).filter(|o| !o.is_empty()) {
            let ord: i32 = ord
                .parse()
                .map_err(|_| refuse("Unreadable key position."))?;
            keyed.push((
                ord,
                KeyColumn {
                    name: name.to_string(),
                    quoted: quoted.to_string(),
                    attnum,
                },
            ));
        }
    }
    if keyed.is_empty() {
        return Err(refuse(
            "This table has no primary key, so a row cannot be identified reliably.",
        ));
    }
    keyed.sort_by_key(|(ord, _)| *ord);
    let key: Vec<KeyColumn> = keyed.into_iter().map(|(_, k)| k).collect();

    Ok(RowIdentity {
        schema: first.get(1).unwrap_or_default().to_string(),
        table: first.get(2).unwrap_or_default().to_string(),
        qualified: first.get(3).unwrap_or_default().to_string(),
        key,
        columns,
    })
}

/// Quote every value with the server's own `quote_nullable`, in one round trip. `None`
/// becomes the keyword NULL, which is how a cell is set to NULL while an empty string
/// stays an empty string.
async fn quote_values(
    session: &Session,
    values: &[Option<String>],
) -> Result<Vec<String>, PgError> {
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let owned: Vec<Option<String>> = values.to_vec();
    let rows = session
        .client
        .query(
            "SELECT quote_nullable(v) FROM unnest($1::text[]) WITH ORDINALITY AS t(v, o) ORDER BY o",
            &[&owned],
        )
        .await
        .map_err(|e| query_err(&e))?;
    Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
}

// ------------------------------------------------------------------------ transactions

/// Find out whether the person already has a transaction of their own open.
///
/// A savepoint is only legal inside a transaction block — outside one the server answers
/// 25P01 and nothing happens — so the attempt both asks the question and, when the answer is
/// yes, leaves behind exactly the savepoint the statement will need. Whoever asks for the
/// probe owns the savepoint from then on: see [`discard_savepoint`].
async fn probe_savepoint(session: &Session, name: &str) -> Result<bool, PgError> {
    match session
        .client
        .simple_query(&format!("SAVEPOINT {name}"))
        .await
    {
        Ok(_) => Ok(true),
        Err(e) if e.code() == Some(&SqlState::NO_ACTIVE_SQL_TRANSACTION) => Ok(false),
        Err(e) => Err(query_err(&e)),
    }
}

/// Give back the probe's savepoint when the request turns out to be one we will not run.
///
/// Rolling back first, rather than only releasing, matters: the pipelined catalog read ran
/// inside the person's transaction, and if *it* was what failed, their transaction is now
/// aborted. Rolling back to the savepoint hands it back to them in working order.
async fn discard_savepoint(
    session: &Session,
    name: &str,
    in_user_txn: bool,
    err: PgError,
) -> PgError {
    if !in_user_txn {
        return err;
    }
    let sql = format!("ROLLBACK TO SAVEPOINT {name}; RELEASE SAVEPOINT {name}");
    match session.client.simple_query(&sql).await {
        Ok(_) => err,
        Err(_) => with_hint(err, ROLLBACK_FAILED),
    }
}

/// Run one statement so that it either takes effect completely or not at all.
///
/// When the person has a transaction open, the statement runs inside it behind the savepoint
/// the probe left: a bad value (a letter typed into an `int4`, say) rolls back to that
/// savepoint instead of aborting everything they had already done. When they do not, `BEGIN`
/// travels with the statement in a single simple-query batch, which leaves the transaction
/// open for the commit below and costs no extra round trip.
///
/// `accept` decides whether to keep the work. Undoing it is not best-effort: when the
/// statement failed *and* the undo failed, both are reported, because the connection is then
/// sitting in a transaction nobody has ended.
async fn run_guarded(
    session: &Session,
    savepoint: &str,
    in_user_txn: bool,
    sql: &str,
    accept: impl Fn(&[SimpleQueryMessage]) -> bool,
) -> Result<Vec<SimpleQueryMessage>, PgError> {
    let client = &session.client;
    let batch = if in_user_txn {
        sql.to_string()
    } else {
        format!("BEGIN;\n{sql}")
    };

    let result = client.simple_query(&batch).await.map_err(|e| query_err(&e));
    let keep = result.as_ref().map(|m| accept(m)).unwrap_or(false);
    let finish = match (keep, in_user_txn) {
        (true, true) => format!("RELEASE SAVEPOINT {savepoint}"),
        (true, false) => "COMMIT".to_string(),
        (false, true) => {
            format!("ROLLBACK TO SAVEPOINT {savepoint}; RELEASE SAVEPOINT {savepoint}")
        }
        (false, false) => "ROLLBACK".to_string(),
    };
    let cleanup = client
        .simple_query(&finish)
        .await
        .map_err(|e| query_err(&e));

    match (result, cleanup) {
        (Ok(msgs), Ok(_)) => Ok(msgs),
        (Ok(_), Err(c)) => Err(with_hint(c, FINISH_FAILED)),
        (Err(e), Ok(_)) => Err(e),
        (Err(e), Err(_)) => Err(with_hint(e, ROLLBACK_FAILED)),
    }
}

/// What a grid write needs to know before it can run, fetched in a single round trip.
///
/// The catalog lookup, the value quoting and the savepoint probe do not depend on one
/// another, so they are polled together and tokio-postgres pipelines them onto the
/// connection. `join!` and not `try_join!` on purpose: the probe's answer has to be read even
/// when one of the others failed, or its savepoint would be left behind.
struct Preamble {
    identity: Result<RowIdentity, PgError>,
    quoted: Result<Vec<String>, PgError>,
    /// Whether a transaction of the person's own is open. `Ok(false)` when no probe was asked
    /// for, which is the case for a preview that will not write anything.
    in_user_txn: Result<bool, PgError>,
}

async fn preamble(
    session: &Session,
    table_oid: u32,
    values: &[Option<String>],
    savepoint: Option<&str>,
) -> Preamble {
    let (identity, quoted, in_user_txn) = tokio::join!(
        identity_of(session, table_oid),
        quote_values(session, values),
        async {
            match savepoint {
                Some(name) => probe_savepoint(session, name).await,
                None => Ok(false),
            }
        },
    );
    Preamble {
        identity,
        quoted,
        in_user_txn,
    }
}

/// Pick up the preamble: the plan when everything worked, and otherwise the error that best
/// explains the refusal, with the probe's savepoint handed back.
async fn settle<T>(
    session: &Session,
    savepoint: &str,
    built: Result<T, PgError>,
    in_user_txn: Result<bool, PgError>,
) -> Result<(T, bool), PgError> {
    match (built, in_user_txn) {
        (Ok(plan), Ok(txn)) => Ok((plan, txn)),
        (Err(e), Ok(txn)) => Err(discard_savepoint(session, savepoint, txn, e).await),
        // The request itself was bad, which says more than a failed probe does.
        (Err(e), Err(_)) => Err(e),
        (Ok(_), Err(e)) => Err(e),
    }
}

/// The row count of the last statement in a batch. `BEGIN` reports one of its own, so the
/// last is the one that matters.
fn affected(msgs: &[SimpleQueryMessage]) -> u64 {
    msgs.iter()
        .rev()
        .find_map(|m| match m {
            SimpleQueryMessage::CommandComplete(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0)
}

fn returned(msgs: &[SimpleQueryMessage]) -> Vec<&SimpleQueryRow> {
    msgs.iter()
        .filter_map(|m| match m {
            SimpleQueryMessage::Row(r) => Some(r),
            _ => None,
        })
        .collect()
}

// ----------------------------------------------------------------------------- deleting

/// Check the selection and flatten it into the values the server has to quote.
fn delete_values(req: &DeleteRequest) -> Result<Vec<Option<String>>, PgError> {
    if req.rows.is_empty() {
        return Err(refuse("No rows are selected."));
    }
    if req.rows.len() > MAX_ROWS {
        return Err(refuse(format!(
            "{} rows selected; Slon deletes at most {MAX_ROWS} at a time from the grid.",
            req.rows.len()
        )));
    }
    Ok(req.rows.iter().flatten().cloned().map(Some).collect())
}

/// Build the statement. Shared by the preview and the real thing so they cannot differ.
fn delete_plan(
    identity: Result<RowIdentity, PgError>,
    quoted: Result<Vec<String>, PgError>,
    req: &DeleteRequest,
) -> Result<DeletePlan, PgError> {
    let identity = identity?;
    let quoted = quoted?;
    let width = identity.key.len();
    if req.rows.iter().any(|r| r.len() != width) {
        return Err(refuse("The key values do not match the primary key."));
    }

    let predicate = if width == 1 {
        let list = quoted.join(", ");
        format!("{} IN ({list})", identity.key[0].quoted)
    } else {
        let rows: Vec<String> = quoted
            .chunks(width)
            .map(|row| {
                let parts: Vec<String> = identity
                    .key
                    .iter()
                    .zip(row)
                    .map(|(k, v)| format!("{} = {v}", k.quoted))
                    .collect();
                format!("({})", parts.join(" AND "))
            })
            .collect();
        rows.join("\n   OR ")
    };

    Ok(DeletePlan {
        sql: format!("DELETE FROM {}\nWHERE {predicate}", identity.qualified),
        row_count: req.rows.len(),
        identity,
    })
}

/// The statement the confirmation dialog shows, without running anything.
pub async fn plan_delete(session: &Session, req: &DeleteRequest) -> Result<DeletePlan, PgError> {
    let _guard = session.begin_query()?;
    let values = delete_values(req)?;
    let p = preamble(session, req.table_oid, &values, None).await;
    delete_plan(p.identity, p.quoted, req)
}

/// Run the statement, and only keep it if it touched exactly the selected rows.
///
/// The count check is what protects against a surprise: if the key turned out not to be
/// unique, or somebody else deleted the row first, the whole thing is rolled back and
/// nothing changes.
pub async fn run_delete(session: &Session, req: &DeleteRequest) -> Result<DeleteOutcome, PgError> {
    let _guard = session.begin_query()?;
    let values = delete_values(req)?;
    let p = preamble(session, req.table_oid, &values, Some(DELETE_SAVEPOINT)).await;
    let built = delete_plan(p.identity, p.quoted, req);
    let (plan, in_user_txn) = settle(session, DELETE_SAVEPOINT, built, p.in_user_txn).await?;

    let expected = plan.row_count as u64;
    let started = Instant::now();
    let msgs = run_guarded(session, DELETE_SAVEPOINT, in_user_txn, &plan.sql, |m| {
        affected(m) == expected
    })
    .await?;

    let deleted = affected(&msgs);
    if deleted != expected {
        return Err(refuse(format!(
            "Expected to delete {expected} row{}, but the statement matched {deleted}. \
             Nothing was deleted.",
            if expected == 1 { "" } else { "s" }
        )));
    }

    Ok(DeleteOutcome {
        sql: plan.sql,
        deleted,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

// ------------------------------------------------------------------------------ editing

/// The column rendered exactly as the grid received it.
///
/// Not `::text`: casting `character(n)` to text drops the trailing padding the wire form
/// carries, so the guard below could never match and every edit of a `char` column was
/// refused as a conflict. `format('%s', …)` runs the type's own output function, which is
/// what the result rows were built with, and it needs no equality operator — so `json`,
/// `xml` and `point` work too. It renders NULL as an empty string, so NULL is kept apart.
fn wire_text(quoted: &str) -> String {
    format!("CASE WHEN {quoted} IS NULL THEN NULL ELSE format('%s', {quoted}) END")
}

/// Build the UPDATE for one cell. Separate from running it so a test, and a future preview,
/// see exactly the statement that would be sent.
fn update_sql(
    identity: Result<RowIdentity, PgError>,
    quoted: Result<Vec<String>, PgError>,
    req: &UpdateRequest,
) -> Result<String, PgError> {
    let identity = identity?;
    let quoted = quoted?;
    if req.key.len() != identity.key.len() {
        return Err(refuse("The key values do not match the primary key."));
    }
    let column = identity
        .columns
        .iter()
        .find(|c| c.attnum == req.column)
        .ok_or_else(|| refuse("That column is no longer part of the table."))?;
    if let Some(reason) = &column.reason {
        return Err(refuse(format!("{} is {reason}.", column.name)));
    }

    // The order `update_values` put them in: the new value, the old one, then the key.
    let (new_value, old_value, keys) = (&quoted[0], &quoted[1], &quoted[2..]);
    let mut wheres: Vec<String> = identity
        .key
        .iter()
        .zip(keys)
        .map(|(k, v)| format!("{} = {v}", k.quoted))
        .collect();
    // The guard that turns a silent overwrite into a reported conflict: compared against
    // the exact text the grid was showing. IS NOT DISTINCT FROM so NULL matches NULL.
    wheres.push(format!(
        "{} IS NOT DISTINCT FROM {old_value}",
        wire_text(&column.quoted)
    ));

    Ok(format!(
        "UPDATE {}\n   SET {} = {new_value}\n WHERE {}\nRETURNING {}",
        identity.qualified,
        column.quoted,
        wheres.join("\n   AND "),
        wire_text(&column.quoted)
    ))
}

/// Everything the UPDATE needs quoted, in the order `update_sql` reads it back.
fn update_values(req: &UpdateRequest) -> Vec<Option<String>> {
    let mut values = vec![req.value.clone(), req.old_value.clone()];
    values.extend(req.key.iter().cloned().map(Some));
    values
}

/// Write one cell, and give back what the column holds afterwards.
///
/// Exactly one row must match: zero means the row changed or went away since the query
/// ran, and in that case nothing is written and the grid keeps showing the old value.
pub async fn run_update(session: &Session, req: &UpdateRequest) -> Result<UpdateOutcome, PgError> {
    let _guard = session.begin_query()?;
    let values = update_values(req);
    let p = preamble(session, req.table_oid, &values, Some(UPDATE_SAVEPOINT)).await;
    let built = update_sql(p.identity, p.quoted, req);
    let (sql, in_user_txn) = settle(session, UPDATE_SAVEPOINT, built, p.in_user_txn).await?;

    let started = Instant::now();
    let msgs = run_guarded(session, UPDATE_SAVEPOINT, in_user_txn, &sql, |m| {
        returned(m).len() == 1
    })
    .await?;

    let rows = returned(&msgs);
    match rows.len() {
        1 => Ok(UpdateOutcome {
            stored: rows[0].get(0).map(str::to_owned),
            sql,
            elapsed_ms: started.elapsed().as_millis() as u64,
        }),
        0 => Err(refuse(
            "This row changed since the query ran, so nothing was written. \
             Run the query again to see the current values.",
        )),
        n => Err(refuse(format!(
            "The key matched {n} rows instead of one, so nothing was written."
        ))),
    }
}
