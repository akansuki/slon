use std::collections::HashMap;

use serde::Serialize;

use crate::ddl::{build_ddl, DdlNames};
use crate::error::{ErrorKind, PgError};
use crate::session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RelationKind {
    Table,
    View,
    MaterializedView,
    ForeignTable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Relation {
    pub name: String,
    pub kind: RelationKind,
}

fn query_err(e: tokio_postgres::Error) -> PgError {
    PgError::from_driver(&e, ErrorKind::Query)
}

/// Databases a user can connect to (templates and `datallowconn = false` are hidden).
pub async fn list_databases(s: &Session) -> Result<Vec<String>, PgError> {
    s.check_open()?;
    let rows = s
        .client
        .query(
            "SELECT datname::text FROM pg_database \
             WHERE NOT datistemplate AND datallowconn ORDER BY 1",
            &[],
        )
        .await
        .map_err(query_err)?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// User-visible schemas of the database `s` is connected to.
pub async fn list_schemas(s: &Session) -> Result<Vec<String>, PgError> {
    s.check_open()?;
    let rows = s
        .client
        .query(
            "SELECT nspname::text FROM pg_namespace \
             WHERE nspname NOT IN ('pg_catalog', 'information_schema') \
               AND nspname NOT LIKE 'pg\\_toast%' \
               AND nspname NOT LIKE 'pg\\_temp\\_%' \
             ORDER BY 1",
            &[],
        )
        .await
        .map_err(query_err)?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// Tables, views, materialized views and foreign tables in `schema`.
pub async fn list_relations(s: &Session, schema: &str) -> Result<Vec<Relation>, PgError> {
    s.check_open()?;
    let rows = s
        .client
        .query(
            "SELECT c.relname::text, c.relkind::text \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f') \
             ORDER BY 1",
            &[&schema],
        )
        .await
        .map_err(query_err)?;
    Ok(rows
        .iter()
        .map(|r| {
            let kind: String = r.get(1);
            Relation {
                name: r.get(0),
                kind: match kind.as_str() {
                    "v" => RelationKind::View,
                    "m" => RelationKind::MaterializedView,
                    "f" => RelationKind::ForeignTable,
                    _ => RelationKind::Table, // 'r' and partitioned 'p'
                },
            }
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionColumn {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionRelation {
    pub schema: String,
    pub name: String,
    pub kind: RelationKind,
    pub columns: Vec<CompletionColumn>,
}

/// Everything the editor needs to complete names in one database.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionSchema {
    /// First existing schema of the search path: unqualified names resolve there first.
    pub default_schema: Option<String>,
    pub relations: Vec<CompletionRelation>,
    /// More relations exist than were returned.
    pub truncated: bool,
}

/// Upper bound so a database with a huge number of tables cannot stall the editor.
pub const COMPLETION_RELATION_LIMIT: i64 = 5_000;

/// Relations (with their columns) of every user-visible schema, in one round trip.
pub async fn completion_schema(s: &Session) -> Result<CompletionSchema, PgError> {
    s.check_open()?;
    let default_schema: Option<String> = s
        .client
        .query_one("SELECT (current_schemas(false))[1]::text", &[])
        .await
        .map_err(query_err)?
        .get(0);

    let rows = s
        .client
        .query(
            "SELECT n.nspname::text, c.relname::text, c.relkind::text, \
                    COALESCE(array_agg(a.attname::text ORDER BY a.attnum) \
                             FILTER (WHERE a.attnum IS NOT NULL), '{}') AS cols, \
                    COALESCE(array_agg(format_type(a.atttypid, a.atttypmod) ORDER BY a.attnum) \
                             FILTER (WHERE a.attnum IS NOT NULL), '{}') AS types \
             FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_attribute a \
               ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped \
             WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f') \
               AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
               AND n.nspname NOT LIKE 'pg\\_toast%' \
               AND n.nspname NOT LIKE 'pg\\_temp\\_%' \
             GROUP BY 1, 2, 3 \
             ORDER BY 1, 2 \
             LIMIT $1",
            &[&(COMPLETION_RELATION_LIMIT + 1)],
        )
        .await
        .map_err(query_err)?;

    let truncated = rows.len() as i64 > COMPLETION_RELATION_LIMIT;
    let relations = rows
        .iter()
        .take(COMPLETION_RELATION_LIMIT as usize)
        .map(|r| {
            let kind: String = r.get(2);
            let names: Vec<String> = r.get(3);
            let types: Vec<String> = r.get(4);
            CompletionRelation {
                schema: r.get(0),
                name: r.get(1),
                kind: match kind.as_str() {
                    "v" => RelationKind::View,
                    "m" => RelationKind::MaterializedView,
                    "f" => RelationKind::ForeignTable,
                    _ => RelationKind::Table,
                },
                columns: names
                    .into_iter()
                    .zip(types)
                    .map(|(name, data_type)| CompletionColumn { name, data_type })
                    .collect(),
            }
        })
        .collect();
    Ok(CompletionSchema {
        default_schema,
        relations,
        truncated,
    })
}

/* ------------------------------------------------------------------ table details */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Identity {
    Always,
    ByDefault,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    /// The name as it must be written in SQL (quoted when needed).
    pub quoted_name: String,
    pub data_type: String,
    pub not_null: bool,
    /// Plain `DEFAULT` expression (not set for identity or generated columns).
    pub default: Option<String>,
    pub identity: Option<Identity>,
    /// Expression of a stored generated column.
    pub generated: Option<String>,
    pub comment: Option<String>,
    pub is_primary_key: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    /// Complete `CREATE INDEX ...` statement.
    pub definition: String,
    pub unique: bool,
    pub primary: bool,
    /// The index exists because of a PRIMARY KEY / UNIQUE / EXCLUDE constraint.
    pub backs_constraint: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConstraintKind {
    PrimaryKey,
    Unique,
    ForeignKey,
    Check,
    Exclusion,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintInfo {
    pub name: String,
    pub kind: ConstraintKind,
    /// What follows the constraint name, e.g. `FOREIGN KEY (a) REFERENCES t(id)`.
    pub definition: String,
    /// `false` for constraints added with `NOT VALID`.
    pub validated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TriggerState {
    Enabled,
    Disabled,
    /// Fires only while `session_replication_role` is `replica`.
    Replica,
    /// Fires in every replication role.
    Always,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerInfo {
    pub name: String,
    /// Complete `CREATE TRIGGER ...` statement.
    pub definition: String,
    /// `BEFORE`, `AFTER` or `INSTEAD OF`.
    pub timing: String,
    /// Any of `INSERT`, `UPDATE`, `DELETE`, `TRUNCATE`.
    pub events: Vec<String>,
    /// `ROW` or `STATEMENT`.
    pub level: String,
    /// The function it calls, schema-qualified when it is not on the search path.
    pub function: String,
    pub state: TriggerState,
}

/// A sequence a column depends on: the one behind a `serial` column, or behind an identity
/// column. Numbers are text because a bigint sequence reaches beyond what a JavaScript
/// number holds exactly.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceInfo {
    pub schema: String,
    pub name: String,
    /// The column that owns it.
    pub column: String,
    /// Created by `GENERATED ... AS IDENTITY` (as opposed to `serial` / `OWNED BY`).
    pub identity: bool,
    pub data_type: String,
    pub start: String,
    pub increment: String,
    pub min: String,
    pub max: String,
    pub cache: String,
    pub cycle: bool,
    /// Last value handed out; `None` while the sequence was never used (or may not be read).
    pub last_value: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDetails {
    pub schema: String,
    pub name: String,
    /// `schema.name`, quoted as the server would.
    pub qualified_name: String,
    pub kind: RelationKind,
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
    pub constraints: Vec<ConstraintInfo>,
    pub triggers: Vec<TriggerInfo>,
    pub sequences: Vec<SequenceInfo>,
    /// Query of a view or materialized view.
    pub view_definition: Option<String>,
    /// `RANGE (id)` etc. for partitioned tables.
    pub partition_key: Option<String>,
    pub comment: Option<String>,
    /// `CREATE ...` script reconstructed from the facts above.
    pub ddl: String,
}

/// Columns, indexes, constraints and a reconstructed `CREATE` script for one relation.
pub async fn table_details(s: &Session, schema: &str, name: &str) -> Result<TableDetails, PgError> {
    s.check_open()?;
    let head = s
        .client
        .query_opt(
            "SELECT c.oid, c.relkind::text, obj_description(c.oid, 'pg_class'), \
                    CASE WHEN c.relkind IN ('v', 'm') THEN pg_get_viewdef(c.oid, true) END, \
                    CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('r', 'p', 'v', 'm', 'f')",
            &[&schema, &name],
        )
        .await
        .map_err(query_err)?
        .ok_or_else(|| {
            PgError::new(
                ErrorKind::NotFound,
                format!("{schema}.{name} no longer exists. Refresh the tree."),
            )
        })?;
    let oid: u32 = head.get(0);
    let relkind: String = head.get(1);
    let kind = match relkind.as_str() {
        "v" => RelationKind::View,
        "m" => RelationKind::MaterializedView,
        "f" => RelationKind::ForeignTable,
        _ => RelationKind::Table,
    };
    let comment: Option<String> = head.get(2);
    let view_definition: Option<String> = head.get(3);
    let partition_key: Option<String> = head.get(4);

    let constraint_rows = s
        .client
        .query(
            "SELECT conname::text, contype::text, pg_get_constraintdef(oid, true), convalidated, \
                    conkey::int4[] \
             FROM pg_constraint \
             WHERE conrelid = $1 AND contype IN ('p', 'u', 'f', 'c', 'x') \
             ORDER BY CASE contype WHEN 'p' THEN 0 WHEN 'u' THEN 1 WHEN 'f' THEN 2 WHEN 'c' THEN 3 ELSE 4 END, \
                      conname",
            &[&oid],
        )
        .await
        .map_err(query_err)?;
    let mut pk_attnums: Vec<i32> = Vec::new();
    let constraints: Vec<ConstraintInfo> = constraint_rows
        .iter()
        .map(|r| {
            let kind = match r.get::<_, String>(1).as_str() {
                "p" => {
                    pk_attnums = r.get::<_, Option<Vec<i32>>>(4).unwrap_or_default();
                    ConstraintKind::PrimaryKey
                }
                "u" => ConstraintKind::Unique,
                "f" => ConstraintKind::ForeignKey,
                "c" => ConstraintKind::Check,
                _ => ConstraintKind::Exclusion,
            };
            ConstraintInfo {
                name: r.get(0),
                kind,
                definition: r.get(2),
                validated: r.get(3),
            }
        })
        .collect();

    let column_rows = s
        .client
        .query(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), a.attnotnull, \
                    pg_get_expr(d.adbin, d.adrelid), a.attidentity::text, a.attgenerated::text, \
                    col_description(a.attrelid, a.attnum), a.attnum::int4 \
             FROM pg_attribute a \
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            &[&oid],
        )
        .await
        .map_err(query_err)?;
    let columns: Vec<ColumnInfo> = column_rows
        .iter()
        .map(|r| {
            let identity: String = r.get(4);
            let generated: String = r.get(5);
            let expr: Option<String> = r.get(3);
            let attnum: i32 = r.get(7);
            let is_generated = generated == "s";
            ColumnInfo {
                name: r.get(0),
                quoted_name: String::new(),
                data_type: r.get(1),
                not_null: r.get(2),
                default: if identity.is_empty() && !is_generated {
                    expr.clone()
                } else {
                    None
                },
                identity: match identity.as_str() {
                    "a" => Some(Identity::Always),
                    "d" => Some(Identity::ByDefault),
                    _ => None,
                },
                generated: if is_generated { expr } else { None },
                comment: r.get(6),
                is_primary_key: pk_attnums.contains(&attnum),
            }
        })
        .collect();

    let index_rows = s
        .client
        .query(
            "SELECT i.relname::text, pg_get_indexdef(x.indexrelid), x.indisunique, x.indisprimary, \
                    EXISTS (SELECT 1 FROM pg_constraint k \
                            WHERE k.conindid = x.indexrelid AND k.conrelid = x.indrelid \
                              AND k.contype IN ('p', 'u', 'x')) \
             FROM pg_index x JOIN pg_class i ON i.oid = x.indexrelid \
             WHERE x.indrelid = $1 \
             ORDER BY x.indisprimary DESC, x.indisunique DESC, i.relname",
            &[&oid],
        )
        .await
        .map_err(query_err)?;
    let indexes: Vec<IndexInfo> = index_rows
        .iter()
        .map(|r| IndexInfo {
            name: r.get(0),
            definition: r.get(1),
            unique: r.get(2),
            primary: r.get(3),
            backs_constraint: r.get(4),
        })
        .collect();

    let trigger_rows = s
        .client
        .query(
            "SELECT t.tgname::text, t.tgtype::int4, t.tgenabled::text, pg_get_triggerdef(t.oid, true), \
                    t.tgfoid::regproc::text \
             FROM pg_trigger t \
             WHERE t.tgrelid = $1 AND NOT t.tgisinternal \
             ORDER BY t.tgname",
            &[&oid],
        )
        .await
        .map_err(query_err)?;
    let triggers: Vec<TriggerInfo> = trigger_rows
        .iter()
        .map(|r| {
            let tgtype: i32 = r.get(1);
            let state: String = r.get(2);
            // tgtype bits: 1 ROW, 2 BEFORE, 4 INSERT, 8 DELETE, 16 UPDATE, 32 TRUNCATE, 64 INSTEAD OF
            let mut events = Vec::new();
            for (bit, name) in [
                (4, "INSERT"),
                (16, "UPDATE"),
                (8, "DELETE"),
                (32, "TRUNCATE"),
            ] {
                if tgtype & bit != 0 {
                    events.push(name.to_string());
                }
            }
            TriggerInfo {
                name: r.get(0),
                definition: r.get(3),
                timing: if tgtype & 64 != 0 {
                    "INSTEAD OF"
                } else if tgtype & 2 != 0 {
                    "BEFORE"
                } else {
                    "AFTER"
                }
                .to_string(),
                events,
                level: if tgtype & 1 != 0 { "ROW" } else { "STATEMENT" }.to_string(),
                function: r.get(4),
                state: match state.as_str() {
                    "D" => TriggerState::Disabled,
                    "R" => TriggerState::Replica,
                    "A" => TriggerState::Always,
                    _ => TriggerState::Enabled,
                },
            }
        })
        .collect();

    // Sequences that belong to a column of this relation ('a' = serial / OWNED BY, 'i' = identity).
    let sequence_rows = s
        .client
        .query(
            "SELECT sn.nspname::text, sc.relname::text, a.attname::text, d.deptype::text = 'i', \
                    format_type(sq.seqtypid, NULL), sq.seqstart::text, sq.seqincrement::text, \
                    sq.seqmin::text, sq.seqmax::text, sq.seqcache::text, sq.seqcycle, \
                    (SELECT ps.last_value::text FROM pg_sequences ps \
                      WHERE ps.schemaname = sn.nspname AND ps.sequencename = sc.relname) \
             FROM pg_depend d \
             JOIN pg_sequence sq ON sq.seqrelid = d.objid \
             JOIN pg_class sc ON sc.oid = sq.seqrelid \
             JOIN pg_namespace sn ON sn.oid = sc.relnamespace \
             JOIN pg_attribute a ON a.attrelid = d.refobjid AND a.attnum = d.refobjsubid \
             WHERE d.classid = 'pg_class'::regclass AND d.refclassid = 'pg_class'::regclass \
               AND d.refobjid = $1 AND d.deptype IN ('a', 'i') \
             ORDER BY a.attnum, sc.relname",
            &[&oid],
        )
        .await
        .map_err(query_err)?;
    let sequences: Vec<SequenceInfo> = sequence_rows
        .iter()
        .map(|r| SequenceInfo {
            schema: r.get(0),
            name: r.get(1),
            column: r.get(2),
            identity: r.get(3),
            data_type: r.get(4),
            start: r.get(5),
            increment: r.get(6),
            min: r.get(7),
            max: r.get(8),
            cache: r.get(9),
            cycle: r.get(10),
            last_value: r.get(11),
        })
        .collect();

    let mut details = TableDetails {
        schema: schema.to_string(),
        name: name.to_string(),
        qualified_name: String::new(),
        kind,
        columns,
        indexes,
        constraints,
        triggers,
        sequences,
        view_definition,
        partition_key,
        comment,
        ddl: String::new(),
    };

    // Let the server quote identifiers: it knows the reserved words.
    let mut idents: Vec<String> = vec![details.schema.clone(), details.name.clone()];
    idents.extend(details.columns.iter().map(|c| c.name.clone()));
    idents.extend(details.constraints.iter().map(|c| c.name.clone()));
    idents.extend(details.triggers.iter().map(|t| t.name.clone()));
    for q in &details.sequences {
        idents.push(q.schema.clone());
        idents.push(q.name.clone());
    }
    let quoted = s
        .client
        .query(
            "SELECT quote_ident(x) FROM unnest($1::text[]) AS x",
            &[&idents],
        )
        .await
        .map_err(query_err)?;
    let map: HashMap<String, String> = idents
        .iter()
        .cloned()
        .zip(quoted.iter().map(|r| r.get::<_, String>(0)))
        .collect();
    let quote = |n: &str| map.get(n).cloned().unwrap_or_else(|| n.to_string());
    details.qualified_name = format!("{}.{}", quote(&details.schema), quote(&details.name));
    for c in &mut details.columns {
        c.quoted_name = quote(&c.name);
    }
    details.ddl = build_ddl(&details, &DdlNames { quote: &quote });
    Ok(details)
}
