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
//! the same input, so the preview cannot drift from what is executed.
//!
//! An edit also carries the value the grid was showing into the WHERE clause, so a row that
//! somebody else changed in the meantime is reported instead of being overwritten.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio_postgres::error::SqlState;
use tokio_postgres::SimpleQueryMessage;

use crate::error::{ErrorKind, PgError};
use crate::session::Session;

/// More rows than this in one go is almost certainly a mistake, and the statement would be
/// unreadable in the confirmation anyway.
pub const MAX_ROWS: usize = 1000;

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

fn query_err(e: &tokio_postgres::Error) -> PgError {
    PgError::from_driver(e, ErrorKind::Query)
}

fn refuse(msg: impl Into<String>) -> PgError {
    PgError::new(ErrorKind::Query, msg)
}

/// Look up the table behind a result column's table OID, and its primary key.
pub async fn row_identity(session: &Session, table_oid: u32) -> Result<RowIdentity, PgError> {
    session.check_open()?;
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

/// Build the statement. Shared by the preview and the real thing so they cannot differ.
pub async fn plan_delete(session: &Session, req: &DeleteRequest) -> Result<DeletePlan, PgError> {
    if req.rows.is_empty() {
        return Err(refuse("No rows are selected."));
    }
    if req.rows.len() > MAX_ROWS {
        return Err(refuse(format!(
            "{} rows selected; Slon deletes at most {MAX_ROWS} at a time from the grid.",
            req.rows.len()
        )));
    }
    let identity = row_identity(session, req.table_oid).await?;
    let width = identity.key.len();
    if req.rows.iter().any(|r| r.len() != width) {
        return Err(refuse("The key values do not match the primary key."));
    }

    let flat: Vec<Option<String>> = req.rows.iter().flatten().cloned().map(Some).collect();
    let quoted = quote_values(session, &flat).await?;

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

/// Run the statement, and only keep it if it touched exactly the selected rows.
///
/// The count check is what protects against a surprise: if the key turned out not to be
/// unique, or somebody else deleted the row first, the whole thing is rolled back and
/// nothing changes.
pub async fn run_delete(session: &Session, req: &DeleteRequest) -> Result<DeleteOutcome, PgError> {
    let plan = plan_delete(session, req).await?;
    let _guard = session.begin_query()?;
    let started = Instant::now();
    let client = &session.client;

    // A savepoint only works inside a transaction block (25P01 otherwise): that tells us
    // whether the user has a transaction of their own open, without disturbing it.
    let in_user_txn = match client.simple_query("SAVEPOINT slon_delete").await {
        Ok(_) => true,
        Err(e) if e.code() == Some(&SqlState::NO_ACTIVE_SQL_TRANSACTION) => {
            client
                .simple_query("BEGIN")
                .await
                .map_err(|e| query_err(&e))?;
            false
        }
        Err(e) => return Err(query_err(&e)),
    };

    let result = client.execute(plan.sql.as_str(), &[]).await;
    let expected = plan.row_count as u64;
    let keep = matches!(&result, Ok(n) if *n == expected);

    let finish = match (keep, in_user_txn) {
        (true, true) => "RELEASE SAVEPOINT slon_delete",
        (true, false) => "COMMIT",
        (false, true) => "ROLLBACK TO SAVEPOINT slon_delete; RELEASE SAVEPOINT slon_delete",
        (false, false) => "ROLLBACK",
    };
    let finished = client.simple_query(finish).await;

    let deleted = result.map_err(|e| query_err(&e))?;
    finished.map_err(|e| query_err(&e))?;
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

/// Build the UPDATE for one cell. Separate from running it so a test, and a future
/// preview, see exactly the statement that would be sent.
async fn plan_update(
    session: &Session,
    req: &UpdateRequest,
) -> Result<(String, RowIdentity), PgError> {
    let identity = row_identity(session, req.table_oid).await?;
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

    // One round trip quotes the new value, the old one and every key value.
    let mut values: Vec<Option<String>> = vec![req.value.clone(), req.old_value.clone()];
    values.extend(req.key.iter().cloned().map(Some));
    let q = quote_values(session, &values).await?;
    let (new_value, old_value, keys) = (&q[0], &q[1], &q[2..]);

    let mut wheres: Vec<String> = identity
        .key
        .iter()
        .zip(keys)
        .map(|(k, v)| format!("{} = {v}", k.quoted))
        .collect();
    // The guard that turns a silent overwrite into a reported conflict. Compared as text
    // because that is exactly what the grid showed, and because some types (json, xml)
    // have no equality operator of their own. IS NOT DISTINCT FROM so NULL matches NULL.
    wheres.push(format!(
        "{}::text IS NOT DISTINCT FROM {old_value}",
        column.quoted
    ));

    Ok((
        format!(
            "UPDATE {}\n   SET {} = {new_value}\n WHERE {}\nRETURNING {}::text",
            identity.qualified,
            column.quoted,
            wheres.join("\n   AND "),
            column.quoted
        ),
        identity,
    ))
}

/// Write one cell, and give back what the column holds afterwards.
///
/// Exactly one row must match: zero means the row changed or went away since the query
/// ran, and in that case nothing is written and the grid keeps showing the old value.
pub async fn run_update(session: &Session, req: &UpdateRequest) -> Result<UpdateOutcome, PgError> {
    let (sql, _) = plan_update(session, req).await?;
    let _guard = session.begin_query()?;
    let started = Instant::now();
    let client = &session.client;

    let in_user_txn = match client.simple_query("SAVEPOINT slon_update").await {
        Ok(_) => true,
        Err(e) if e.code() == Some(&SqlState::NO_ACTIVE_SQL_TRANSACTION) => {
            client
                .simple_query("BEGIN")
                .await
                .map_err(|e| query_err(&e))?;
            false
        }
        Err(e) => return Err(query_err(&e)),
    };

    let result = client.query(sql.as_str(), &[]).await;
    let keep = matches!(&result, Ok(rows) if rows.len() == 1);
    let finish = match (keep, in_user_txn) {
        (true, true) => "RELEASE SAVEPOINT slon_update",
        (true, false) => "COMMIT",
        (false, true) => "ROLLBACK TO SAVEPOINT slon_update; RELEASE SAVEPOINT slon_update",
        (false, false) => "ROLLBACK",
    };
    let finished = client.simple_query(finish).await;

    let rows = result.map_err(|e| query_err(&e))?;
    finished.map_err(|e| query_err(&e))?;
    match rows.len() {
        1 => Ok(UpdateOutcome {
            sql,
            stored: rows[0].get::<_, Option<String>>(0),
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
