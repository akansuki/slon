//! Integration tests against a real PostgreSQL server.
//!
//!   eval "$(scripts/test-db.sh start)" && cargo test -p pg-core
//!
//! Without the SLON_TEST_* variables every test is skipped (and says so). Set
//! SLON_REQUIRE_DB=1 (CI) to turn a missing database into a failure instead.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pg_core::*;

fn config() -> Option<ConnectionConfig> {
    let host = std::env::var("SLON_TEST_HOST").ok()?;
    Some(ConnectionConfig {
        host,
        port: std::env::var("SLON_TEST_PORT").ok()?.parse().ok()?,
        user: std::env::var("SLON_TEST_USER").ok()?,
        password: std::env::var("SLON_TEST_PASSWORD").ok()?,
        database: std::env::var("SLON_TEST_DB").ok()?,
        ssl_mode: SslMode::Prefer,
        ssl_root_cert: None,
    })
}

macro_rules! cfg_or_skip {
    () => {
        match config() {
            Some(c) => c,
            None => {
                if std::env::var("SLON_REQUIRE_DB").is_ok() {
                    panic!("SLON_REQUIRE_DB is set but SLON_TEST_* are not");
                }
                eprintln!("skipped: SLON_TEST_* not set");
                return;
            }
        }
    };
}

/// Everything a query emitted, collected for assertions.
#[derive(Default, Debug)]
struct Collected {
    columns: Vec<Vec<Column>>,
    rows: Vec<Vec<Vec<Option<String>>>>,
    done: Vec<(u64, usize, bool, bool)>, // rows_affected, row_count, has_columns, truncated
}

async fn run(
    reg: &Registry,
    tab: u64,
    sql: &str,
    opts: &QueryOptions,
) -> (Result<QuerySummary, PgError>, Collected) {
    let mut c = Collected::default();
    let res = reg
        .run_query(tab, sql, opts, |ev| match ev {
            QueryEvent::Columns { columns, .. } => {
                c.columns.push(columns);
                c.rows.push(vec![]);
            }
            QueryEvent::Rows { rows, .. } => c.rows.last_mut().unwrap().extend(rows),
            QueryEvent::ResultDone {
                rows_affected,
                row_count,
                has_columns,
                truncated,
                ..
            } => c
                .done
                .push((rows_affected, row_count, has_columns, truncated)),
            QueryEvent::Message { .. } => {}
            QueryEvent::Finished { .. } | QueryEvent::Failed { .. } => {
                panic!("run_query must not emit terminal events")
            }
        })
        .await;
    (res, c)
}

async fn tab(reg: &Registry, cfg: &ConnectionConfig) -> u64 {
    let conn = reg.connect(cfg.clone()).await.expect("connect");
    reg.open_tab(conn.id, None).await.expect("open tab")
}

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn connects_and_reports_version() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let info = reg.connect(cfg.clone()).await.unwrap();
    assert!(!info.server_version.is_empty());
    assert_eq!(info.database, cfg.database);
    reg.disconnect(info.id).unwrap();
    assert_eq!(
        reg.disconnect(info.id).unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_password_is_a_structured_connection_error() {
    let mut cfg = cfg_or_skip!();
    cfg.password = "definitely-wrong".into();
    let err = Registry::new().connect(cfg).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Connection);
    assert_eq!(err.sqlstate.as_deref(), Some("28P01"), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn refused_connection_has_a_readable_message() {
    let mut cfg = cfg_or_skip!();
    cfg.port = 1;
    let err = Registry::new().connect(cfg).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Connection);
    assert!(!err.message.is_empty());
    eprintln!("refused message: {}", err.message);
}

#[tokio::test(flavor = "multi_thread")]
async fn typed_select_returns_text_values_and_sort_kinds() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let t = tab(&reg, &cfg).await;
    let (res, c) = run(
        &reg,
        t,
        "SELECT 1::int4 AS n, 2.50::numeric AS d, 'a'::text AS s, true AS b, NULL::int AS nul, now() AS ts",
        &QueryOptions::default(),
    )
    .await;
    res.unwrap();
    let kinds: Vec<_> = c.columns[0].iter().map(|c| c.sort_kind).collect();
    assert_eq!(
        kinds,
        [
            SortKind::Number,
            SortKind::Number,
            SortKind::Text,
            SortKind::Boolean,
            SortKind::Number,
            SortKind::Text
        ]
    );
    assert_eq!(c.columns[0][0].type_name.as_deref(), Some("int4"));
    assert_eq!(c.columns[0][5].type_name.as_deref(), Some("timestamptz"));
    let row = &c.rows[0][0];
    assert_eq!(&row[..5], [s("1"), s("2.50"), s("a"), s("t"), None]);
    assert_eq!(c.done, [(1, 1, true, false)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn syntax_error_reports_position() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let t = tab(&reg, &cfg).await;
    let (res, _) = run(&reg, t, "SELECT 1;\nSELEC 2", &QueryOptions::default()).await;
    let err = res.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Query);
    assert_eq!(err.sqlstate.as_deref(), Some("42601"));
    // 1-based character offset into the whole script: "SELECT 1;\n" is 10 chars.
    assert_eq!(err.position, Some(11), "{err:?}");

    // The tab survives a failed statement.
    let (res, c) = run(&reg, t, "SELECT 42", &QueryOptions::default()).await;
    res.unwrap();
    assert_eq!(c.rows[0][0][0], s("42"));
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_statement_script_reports_each_result() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let t = tab(&reg, &cfg).await;
    let (res, c) = run(
        &reg,
        t,
        "CREATE TEMP TABLE t(x int); INSERT INTO t VALUES (2),(1); SELECT * FROM t ORDER BY x",
        &QueryOptions::default(),
    )
    .await;
    let summary = res.unwrap();
    assert_eq!(summary.statements, 3);
    assert_eq!(
        c.done,
        [
            (0, 0, false, false),
            (2, 0, false, false),
            (2, 2, true, false)
        ]
    );
    assert_eq!(c.columns[0][0].sort_kind, SortKind::Unknown);
    assert_eq!(c.rows[0], [vec![s("1")], vec![s("2")]]);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_stops_a_running_query_and_the_tab_stays_usable() {
    let cfg = cfg_or_skip!();
    let reg = Arc::new(Registry::new());
    let t = tab(&reg, &cfg).await;

    let started = Instant::now();
    let r2 = reg.clone();
    let running = tokio::spawn(async move {
        run(&r2, t, "SELECT pg_sleep(60)", &QueryOptions::default())
            .await
            .0
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    reg.cancel_query(t).await.unwrap();

    let err = running.await.unwrap().unwrap_err();
    assert_eq!(err.kind, ErrorKind::Cancelled, "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(10));

    let (res, c) = run(&reg, t, "SELECT 'still alive'", &QueryOptions::default()).await;
    res.unwrap();
    assert_eq!(c.rows[0][0][0], s("still alive"));
}

#[tokio::test(flavor = "multi_thread")]
async fn second_query_on_a_busy_tab_is_rejected() {
    let cfg = cfg_or_skip!();
    let reg = Arc::new(Registry::new());
    let t = tab(&reg, &cfg).await;

    let r2 = reg.clone();
    let running = tokio::spawn(async move {
        run(&r2, t, "SELECT pg_sleep(60)", &QueryOptions::default())
            .await
            .0
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    let (res, _) = run(&reg, t, "SELECT 1", &QueryOptions::default()).await;
    assert_eq!(res.unwrap_err().kind, ErrorKind::Busy);

    reg.cancel_query(t).await.unwrap();
    let _ = running.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn row_cap_truncates_without_poisoning_the_next_query() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let t = tab(&reg, &cfg).await;
    let opts = QueryOptions {
        max_rows: 100,
        batch_size: 30,
    };

    // The cancel we send at the cap is asynchronous; make sure it can never hit the
    // statement that follows. Repeat to give a race a chance to show up.
    for round in 0..15 {
        let (res, c) = run(&reg, t, "SELECT g FROM generate_series(1, 200000) g", &opts).await;
        let summary = res.unwrap_or_else(|e| panic!("round {round}: {e:?}"));
        assert!(summary.truncated);
        assert_eq!(c.rows[0].len(), 100);
        // rows_affected is 100 if our cancel won the race and the server's full count
        // if the statement finished first; both are fine. What the UI relies on:
        assert_eq!(c.done.len(), 1);
        let (affected, delivered, has_columns, truncated) = c.done[0];
        assert!(
            affected >= 100 && delivered == 100 && has_columns && truncated,
            "{:?}",
            c.done
        );

        let (res, c) = run(&reg, t, "SELECT 'next'", &QueryOptions::default()).await;
        res.unwrap_or_else(|e| panic!("round {round}, follow-up query failed: {e:?}"));
        assert_eq!(c.rows[0][0][0], s("next"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn scripts_are_never_cancelled_at_the_row_cap() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let t = tab(&reg, &cfg).await;
    let opts = QueryOptions {
        max_rows: 10,
        batch_size: 5,
    };
    // Cancelling would roll back the INSERT (the script is one implicit transaction).
    let (res, c) = run(
        &reg,
        t,
        "CREATE TEMP TABLE keep(x int); INSERT INTO keep VALUES (1); SELECT g FROM generate_series(1, 5000) g; SELECT count(*) FROM keep",
        &opts,
    )
    .await;
    let summary = res.unwrap();
    assert!(summary.truncated);
    assert_eq!(summary.statements, 4);
    assert_eq!(c.rows[0].len(), 10);
    assert_eq!(c.rows[1][0][0], s("1"), "INSERT must have survived");
}

#[tokio::test(flavor = "multi_thread")]
async fn large_result_arrives_in_batches() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let t = tab(&reg, &cfg).await;
    let mut batches = 0;
    let mut total = 0;
    reg.run_query(
        t,
        "SELECT g, repeat('x', 20) FROM generate_series(1, 2500) g",
        &QueryOptions {
            max_rows: 100_000,
            batch_size: 1000,
        },
        |ev| {
            if let QueryEvent::Rows { rows, .. } = ev {
                batches += 1;
                total += rows.len();
            }
        },
    )
    .await
    .unwrap();
    assert_eq!((batches, total), (3, 2500));
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_lists_schemas_and_relation_kinds() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let conn = reg.connect(cfg.clone()).await.unwrap();
    let t = reg.open_tab(conn.id, None).await.unwrap();
    let (res, _) = run(
        &reg,
        t,
        "DROP SCHEMA IF EXISTS slon_cat CASCADE; CREATE SCHEMA slon_cat; \
         CREATE TABLE slon_cat.b_table(id int); CREATE TABLE slon_cat.part(id int) PARTITION BY RANGE (id); \
         CREATE VIEW slon_cat.a_view AS SELECT 1; CREATE MATERIALIZED VIEW slon_cat.m_view AS SELECT 1",
        &QueryOptions::default(),
    )
    .await;
    res.unwrap();

    let dbs = reg.list_databases(conn.id).await.unwrap();
    assert!(dbs.contains(&cfg.database));
    assert!(!dbs.iter().any(|d| d.starts_with("template")));

    let schemas = reg.list_schemas(conn.id, &cfg.database).await.unwrap();
    assert!(schemas.contains(&"slon_cat".to_string()) && schemas.contains(&"public".to_string()));
    assert!(!schemas
        .iter()
        .any(|s| s == "pg_catalog" || s == "information_schema" || s.starts_with("pg_toast")));

    let rels = reg
        .list_relations(conn.id, &cfg.database, "slon_cat")
        .await
        .unwrap();
    let got: Vec<_> = rels.iter().map(|r| (r.name.as_str(), r.kind)).collect();
    assert_eq!(
        got,
        [
            ("a_view", RelationKind::View),
            ("b_table", RelationKind::Table),
            ("m_view", RelationKind::MaterializedView),
            ("part", RelationKind::Table),
        ]
    );

    // Browsing another database opens a connection to *that* database: it must not see
    // the schema we just created in the first one.
    let (res, _) = run(
        &reg,
        t,
        "DROP DATABASE IF EXISTS slon_other",
        &QueryOptions::default(),
    )
    .await;
    res.unwrap();
    let (res, _) = run(
        &reg,
        t,
        "CREATE DATABASE slon_other",
        &QueryOptions::default(),
    )
    .await;
    res.unwrap();
    let other = reg.list_schemas(conn.id, "slon_other").await.unwrap();
    assert!(
        other.contains(&"public".to_string()) && !other.contains(&"slon_cat".to_string()),
        "{other:?}"
    );
    assert!(reg
        .list_databases(conn.id)
        .await
        .unwrap()
        .contains(&"slon_other".to_string()));

    // template0 refuses connections; the error must surface as a connection error.
    let err = reg.list_schemas(conn.id, "template0").await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Connection, "{err:?}");
}

async fn ssl_in_use(reg: &Registry, tab: u64) -> bool {
    let (res, c) = run(
        reg,
        tab,
        "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
        &QueryOptions::default(),
    )
    .await;
    res.unwrap();
    c.rows[0][0][0] == s("t")
}

#[tokio::test(flavor = "multi_thread")]
async fn tls_modes_behave_like_libpq() {
    let cfg = cfg_or_skip!();

    // Does this server offer TLS at all? (Depends on how the test cluster was made.)
    let probe = Registry::new();
    let t = tab(&probe, &cfg).await;
    let (_, c) = run(&probe, t, "SHOW ssl", &QueryOptions::default()).await;
    if c.rows[0][0][0] != s("on") {
        eprintln!("skipped: server has ssl = off");
        return;
    }

    for (mode, expect_tls) in [
        (SslMode::Disable, false),
        (SslMode::Prefer, true),
        (SslMode::Require, true),
    ] {
        let reg = Registry::new();
        let conn = reg
            .connect(ConnectionConfig {
                ssl_mode: mode,
                ..cfg.clone()
            })
            .await
            .unwrap();
        let t = reg.open_tab(conn.id, None).await.unwrap();
        assert_eq!(ssl_in_use(&reg, t).await, expect_tls, "{mode:?}");

        // Cancelling must work over TLS as well.
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let running = tokio::spawn(async move {
            run(&r2, t, "SELECT pg_sleep(60)", &QueryOptions::default())
                .await
                .0
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        reg.cancel_query(t).await.unwrap();
        assert_eq!(
            running.await.unwrap().unwrap_err().kind,
            ErrorKind::Cancelled,
            "{mode:?}"
        );
    }
}

#[tokio::test]
async fn completion_schema_lists_relations_with_columns_and_default_schema() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let conn = reg.connect(cfg.clone()).await.unwrap();
    let t = reg.open_tab(conn.id, None).await.unwrap();
    let (res, _) = run(
        &reg,
        t,
        "DROP SCHEMA IF EXISTS slon_cmp CASCADE; CREATE SCHEMA slon_cmp; \
         CREATE TABLE slon_cmp.orders(id bigint, \"Odd Name\" text, price numeric(10,2), dropped int); \
         ALTER TABLE slon_cmp.orders DROP COLUMN dropped; \
         CREATE VIEW slon_cmp.big_orders AS SELECT id FROM slon_cmp.orders; \
         CREATE TABLE slon_cmp.empty_cols(); \
         DROP SCHEMA IF EXISTS slon_cmp_empty CASCADE; CREATE SCHEMA slon_cmp_empty",
        &QueryOptions::default(),
    )
    .await;
    res.unwrap();

    let schema = reg.completion_schema(conn.id, &cfg.database).await.unwrap();
    assert_eq!(schema.default_schema.as_deref(), Some("public"));
    assert!(!schema.truncated);
    assert!(!schema.relations.iter().any(|r| r.schema == "pg_catalog"
        || r.schema == "information_schema"
        || r.schema.starts_with("pg_toast")));

    let find = |name: &str| {
        schema
            .relations
            .iter()
            .find(|r| r.schema == "slon_cmp" && r.name == name)
            .unwrap()
    };
    let orders = find("orders");
    assert_eq!(orders.kind, RelationKind::Table);
    let cols: Vec<_> = orders
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c.data_type.as_str()))
        .collect();
    assert_eq!(
        cols,
        [
            ("id", "bigint"),
            ("Odd Name", "text"),
            ("price", "numeric(10,2)")
        ]
    );
    assert_eq!(find("big_orders").kind, RelationKind::View);
    assert_eq!(find("big_orders").columns.len(), 1);
    // A table without columns is still listed.
    assert!(find("empty_cols").columns.is_empty());
    // Ordered by schema then name.
    let names: Vec<_> = schema
        .relations
        .iter()
        .filter(|r| r.schema == "slon_cmp")
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(names, ["big_orders", "empty_cols", "orders"]);
}

#[tokio::test]
async fn table_details_describe_columns_indexes_constraints_and_ddl() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let conn = reg.connect(cfg.clone()).await.unwrap();
    let t = reg.open_tab(conn.id, None).await.unwrap();
    let (res, _) = run(
        &reg,
        t,
        "DROP SCHEMA IF EXISTS slon_td CASCADE; CREATE SCHEMA slon_td; \
         CREATE TABLE slon_td.parent(id int PRIMARY KEY); \
         CREATE TABLE slon_td.child(\
            id bigint GENERATED ALWAYS AS IDENTITY, \
            parent_id int NOT NULL REFERENCES slon_td.parent(id), \
            \"order\" text NOT NULL DEFAULT 'new', \
            qty int NOT NULL CHECK (qty > 0), \
            price numeric(10,2), \
            total numeric GENERATED ALWAYS AS (qty * price) STORED, \
            tags text[], \
            code text UNIQUE, \
            PRIMARY KEY (id, parent_id)); \
         CREATE INDEX child_expr_idx ON slon_td.child (lower(\"order\")) WHERE qty > 5; \
         COMMENT ON TABLE slon_td.child IS 'it''s a child'; \
         COMMENT ON COLUMN slon_td.child.qty IS 'how many'; \
         CREATE TABLE slon_td.part(id int, d date) PARTITION BY RANGE (d)",
        &QueryOptions::default(),
    )
    .await;
    res.unwrap();

    let d = reg
        .table_details(conn.id, &cfg.database, "slon_td", "child")
        .await
        .unwrap();
    assert_eq!(d.kind, RelationKind::Table);
    assert_eq!(d.comment.as_deref(), Some("it's a child"));
    let col = |n: &str| d.columns.iter().find(|c| c.name == n).unwrap();
    assert_eq!(d.columns.len(), 8);
    assert_eq!(col("id").identity, Some(Identity::Always));
    assert!(
        col("id").is_primary_key && col("parent_id").is_primary_key && !col("qty").is_primary_key
    );
    assert!(col("id").default.is_none());
    assert_eq!(col("order").default.as_deref(), Some("'new'::text"));
    assert!(col("order").not_null);
    assert_eq!(col("price").data_type, "numeric(10,2)");
    assert_eq!(col("tags").data_type, "text[]");
    assert_eq!(
        col("total").generated.as_deref(),
        Some("((qty)::numeric * price)")
    );
    assert!(col("total").default.is_none());
    assert_eq!(col("qty").comment.as_deref(), Some("how many"));

    let kinds: Vec<_> = d.constraints.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            ConstraintKind::PrimaryKey,
            ConstraintKind::Unique,
            ConstraintKind::ForeignKey,
            ConstraintKind::Check
        ]
    );
    assert!(d.constraints[2]
        .definition
        .contains("REFERENCES slon_td.parent(id)"));
    assert!(d.constraints.iter().all(|c| c.validated));

    assert_eq!(d.indexes.len(), 3);
    assert!(d.indexes[0].primary && d.indexes[0].backs_constraint);
    let expr = d
        .indexes
        .iter()
        .find(|i| i.name == "child_expr_idx")
        .unwrap();
    assert!(!expr.backs_constraint && !expr.unique);
    assert!(expr.definition.contains("lower(\"order\")") && expr.definition.contains("WHERE"));

    // Reserved words are quoted by the server's own quote_ident.
    assert!(
        d.ddl
            .contains("\"order\" text DEFAULT 'new'::text NOT NULL"),
        "{}",
        d.ddl
    );
    assert!(d.ddl.starts_with("CREATE TABLE slon_td.child ("));
    assert_eq!(
        d.ddl.matches("CREATE UNIQUE INDEX").count(),
        0,
        "constraint indexes are not repeated"
    );
    assert!(d.ddl.contains("CREATE INDEX child_expr_idx"));
    assert!(d
        .ddl
        .contains("COMMENT ON TABLE slon_td.child IS 'it''s a child';"));

    // The generated script must be runnable and reproduce itself exactly.
    let (res, _) = run(
        &reg,
        t,
        "DROP TABLE slon_td.child",
        &QueryOptions::default(),
    )
    .await;
    res.unwrap();
    let (res, _) = run(&reg, t, &d.ddl, &QueryOptions::default()).await;
    res.unwrap();
    let again = reg
        .table_details(conn.id, &cfg.database, "slon_td", "child")
        .await
        .unwrap();
    assert_eq!(again.ddl, d.ddl);

    let (res, _) = run(
        &reg,
        t,
        "CREATE VIEW slon_td.child_view AS SELECT id, qty FROM slon_td.child",
        &QueryOptions::default(),
    )
    .await;
    res.unwrap();
    let v = reg
        .table_details(conn.id, &cfg.database, "slon_td", "child_view")
        .await
        .unwrap();
    assert_eq!(v.kind, RelationKind::View);
    assert_eq!(v.columns.len(), 2);
    assert!(v.ddl.starts_with("CREATE VIEW slon_td.child_view AS\n"));
    assert!(v.ddl.contains("FROM slon_td.child"));

    let p = reg
        .table_details(conn.id, &cfg.database, "slon_td", "part")
        .await
        .unwrap();
    assert_eq!(p.partition_key.as_deref(), Some("RANGE (d)"));
    assert!(p.ddl.contains(") PARTITION BY RANGE (d);"));

    let err = reg
        .table_details(conn.id, &cfg.database, "slon_td", "nope")
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

/// Certificate verification against the private CA made by scripts/test-db.sh. The server
/// certificate is valid for the name "localhost" only.
#[tokio::test]
async fn certificate_verification_modes() {
    let cfg = cfg_or_skip!();
    let (Ok(ca), Ok(other_ca)) = (
        std::env::var("SLON_TEST_CA"),
        std::env::var("SLON_TEST_OTHER_CA"),
    ) else {
        if std::env::var("SLON_REQUIRE_DB").is_ok() {
            panic!("SLON_REQUIRE_DB is set but SLON_TEST_CA / SLON_TEST_OTHER_CA are not");
        }
        eprintln!("skipped: SLON_TEST_CA not set");
        return;
    };
    let with = |mode: SslMode, host: &str, root: Option<&str>| ConnectionConfig {
        host: host.to_string(),
        ssl_mode: mode,
        ssl_root_cert: root.map(str::to_string),
        ..cfg.clone()
    };
    let connect = |c: ConnectionConfig| async move {
        let reg = Registry::new();
        let conn = reg.connect(c).await?;
        let t = reg.open_tab(conn.id, None).await?;
        let tls = ssl_in_use(&reg, t).await;
        Ok::<_, PgError>((reg, t, tls))
    };

    // Trusted chain + matching name: both verifying modes work, over TLS.
    for mode in [SslMode::VerifyCa, SslMode::VerifyFull] {
        let (_, _, tls) = connect(with(mode, "localhost", Some(&ca)))
            .await
            .unwrap_or_else(|e| panic!("{mode:?}: {e}"));
        assert!(tls, "{mode:?} must use TLS");
    }

    // Right CA, wrong name (connecting by IP, certificate says localhost):
    // verify-ca does not look at the name, verify-full does.
    let (_, _, tls) = connect(with(SslMode::VerifyCa, "127.0.0.1", Some(&ca)))
        .await
        .unwrap();
    assert!(tls);
    let err = connect(with(SslMode::VerifyFull, "127.0.0.1", Some(&ca)))
        .await
        .err()
        .expect("name mismatch must fail");
    assert_eq!(err.kind, ErrorKind::Connection);
    assert!(
        err.message.contains("not valid for name"),
        "{}",
        err.message
    );
    assert!(
        err.hint.as_deref().unwrap_or("").contains("127.0.0.1"),
        "{:?}",
        err.hint
    );

    // A CA that did not sign the server certificate is rejected in both modes.
    for mode in [SslMode::VerifyCa, SslMode::VerifyFull] {
        let err = connect(with(mode, "localhost", Some(&other_ca)))
            .await
            .err()
            .expect("wrong CA must fail");
        assert!(
            err.message.to_lowercase().contains("unknownissuer")
                || err.message.to_lowercase().contains("unknown issuer"),
            "{mode:?}: {}",
            err.message
        );
        assert!(err
            .hint
            .as_deref()
            .unwrap_or("")
            .contains("not signed by a CA you trust"));
    }

    // No file: the system store is used, which does not know our private CA.
    let err = connect(with(SslMode::VerifyFull, "localhost", None))
        .await
        .err()
        .expect("system roots must not trust it");
    assert!(
        err.message.to_lowercase().contains("unknown") || err.message.contains("No trusted CA"),
        "{}",
        err.message
    );

    // Bad files say what is wrong with them.
    let err = connect(with(
        SslMode::VerifyFull,
        "localhost",
        Some("/definitely/not/here.pem"),
    ))
    .await
    .err()
    .unwrap();
    assert!(
        err.message
            .contains("Could not read the CA certificate file"),
        "{}",
        err.message
    );
    assert!(err.message.contains("/definitely/not/here.pem"));
    let junk = std::env::temp_dir().join("slon-not-a-cert.pem");
    std::fs::write(&junk, "hello, I am not a certificate").unwrap();
    let err = connect(with(
        SslMode::VerifyFull,
        "localhost",
        Some(junk.to_str().unwrap()),
    ))
    .await
    .err()
    .unwrap();
    assert!(
        err.message.contains("contains no usable certificates"),
        "{}",
        err.message
    );

    // Modes that do not verify ignore the file entirely.
    let (_, _, tls) = connect(with(
        SslMode::Require,
        "127.0.0.1",
        Some("/definitely/not/here.pem"),
    ))
    .await
    .unwrap();
    assert!(tls);

    // Cancelling uses the same verified TLS settings.
    let (reg, t, _) = connect(with(SslMode::VerifyFull, "localhost", Some(&ca)))
        .await
        .unwrap();
    let reg = Arc::new(reg);
    let r2 = reg.clone();
    let running = tokio::spawn(async move {
        run(&r2, t, "SELECT pg_sleep(60)", &QueryOptions::default())
            .await
            .0
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    reg.cancel_query(t).await.unwrap();
    assert_eq!(
        running.await.unwrap().unwrap_err().kind,
        ErrorKind::Cancelled
    );
}

async fn scalar(reg: &Registry, tab: u64, sql: &str) -> String {
    let (res, c) = run(reg, tab, sql, &QueryOptions::default()).await;
    res.expect("scalar query");
    c.rows[0][0][0].clone().expect("not null")
}

#[tokio::test]
async fn explain_returns_json_plans_and_analyze_never_keeps_changes() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let t = tab(&reg, &cfg).await;
    let q = QueryOptions::default();
    run(
        &reg,
        t,
        "DROP TABLE IF EXISTS slon_explain; CREATE TABLE slon_explain (id int primary key, v text)",
        &q,
    )
    .await
    .0
    .unwrap();
    run(
        &reg,
        t,
        "INSERT INTO slon_explain SELECT g, 'x' FROM generate_series(1, 100) g",
        &q,
    )
    .await
    .0
    .unwrap();

    // Plain EXPLAIN: a JSON document with a plan, no timings, nothing executed.
    let plain = reg
        .explain(t, "SELECT * FROM slon_explain WHERE id = 5;", false)
        .await
        .unwrap();
    assert!(!plain.analyzed);
    assert!(
        plain.plan.contains("\"Plan\"") && plain.plan.contains("\"Node Type\""),
        "{}",
        plain.plan
    );
    assert!(!plain.plan.contains("Execution Time"));
    let deleted = reg
        .explain(t, "DELETE FROM slon_explain", false)
        .await
        .unwrap();
    assert!(deleted.plan.contains("\"Delete\""), "{}", deleted.plan);
    assert_eq!(
        scalar(&reg, t, "SELECT count(*) FROM slon_explain").await,
        "100"
    );

    // ANALYZE executes the statement but must roll it back.
    let analyzed = reg
        .explain(t, "DELETE FROM slon_explain", true)
        .await
        .unwrap();
    assert!(analyzed.analyzed);
    assert!(
        analyzed.plan.contains("Execution Time") && analyzed.plan.contains("Actual Rows"),
        "{}",
        analyzed.plan
    );
    assert_eq!(
        scalar(&reg, t, "SELECT count(*) FROM slon_explain").await,
        "100"
    );

    // Inside a transaction of the user's own, that transaction must survive.
    run(&reg, t, "BEGIN", &q).await.0.unwrap();
    run(
        &reg,
        t,
        "INSERT INTO slon_explain VALUES (1000, 'mine')",
        &q,
    )
    .await
    .0
    .unwrap();
    reg.explain(t, "DELETE FROM slon_explain", true)
        .await
        .unwrap();
    assert_eq!(
        scalar(&reg, t, "SELECT count(*) FROM slon_explain").await,
        "101",
        "user's insert kept"
    );
    run(&reg, t, "ROLLBACK", &q).await.0.unwrap();
    assert_eq!(
        scalar(&reg, t, "SELECT count(*) FROM slon_explain").await,
        "100"
    );

    // Not left in a transaction afterwards.
    assert_eq!(
        scalar(
            &reg,
            t,
            "SELECT (pg_current_xact_id_if_assigned() IS NULL)::text"
        )
        .await,
        "true"
    );
    run(&reg, t, "DROP TABLE slon_explain", &q).await.0.unwrap();
}

#[tokio::test]
async fn explain_rejects_scripts_and_maps_error_positions_back() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let t = tab(&reg, &cfg).await;
    let q = QueryOptions::default();
    run(
        &reg,
        t,
        "DROP TABLE IF EXISTS slon_explain2; CREATE TABLE slon_explain2 (id int)",
        &q,
    )
    .await
    .0
    .unwrap();

    // Nothing after the first statement may run.
    let err = reg
        .explain(t, "SELECT 1; DELETE FROM slon_explain2", false)
        .await
        .unwrap_err();
    assert!(err.message.contains("one statement"), "{err:?}");

    // "SELECT nope" -> the server points at `nope`, character 8 of the user's text.
    let sql = "SELECT nope FROM slon_explain2";
    let err = reg.explain(t, sql, false).await.unwrap_err();
    assert_eq!(err.sqlstate.as_deref(), Some("42703"), "{err:?}");
    assert_eq!(err.position, Some(8), "{err:?}");

    assert_eq!(
        reg.explain(t, "  ;  ", false).await.unwrap_err().message,
        "Nothing to explain."
    );

    // A statement that fails while executing leaves the tab usable.
    let err = reg.explain(t, "SELECT 1/0", true).await.unwrap_err();
    assert_eq!(err.sqlstate.as_deref(), Some("22012"), "{err:?}");
    assert_eq!(scalar(&reg, t, "SELECT 1").await, "1");
    run(&reg, t, "DROP TABLE slon_explain2", &q)
        .await
        .0
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn explain_analyze_can_be_cancelled() {
    let cfg = cfg_or_skip!();
    let reg = Arc::new(Registry::new());
    let t = tab(&reg, &cfg).await;
    let r2 = reg.clone();
    let started = Instant::now();
    let handle = tokio::spawn(async move { r2.explain(t, "SELECT pg_sleep(30)", true).await });
    tokio::time::sleep(Duration::from_millis(400)).await;
    reg.cancel_query(t).await.unwrap();
    let err = handle.await.unwrap().unwrap_err();
    assert_eq!(err.kind, ErrorKind::Cancelled, "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(
        scalar(&reg, t, "SELECT 1").await,
        "1",
        "tab usable after cancel"
    );
}

/// Runs `sql` and returns what happened, in order: "rows:N", "done", "msg:SEVERITY:text".
async fn trace(
    reg: &Registry,
    tab: u64,
    sql: &str,
) -> (
    Result<QuerySummary, PgError>,
    Vec<String>,
    Vec<ServerMessage>,
) {
    let mut events = Vec::new();
    let mut messages = Vec::new();
    let res = reg
        .run_query(tab, sql, &QueryOptions::default(), |ev| match ev {
            QueryEvent::Rows { rows, .. } => events.push(format!("rows:{}", rows.len())),
            QueryEvent::ResultDone { .. } => events.push("done".into()),
            QueryEvent::Message { message } => {
                events.push(format!("msg:{}:{}", message.severity, message.message));
                messages.push(message);
            }
            _ => {}
        })
        .await;
    (res, events, messages)
}

#[tokio::test]
async fn server_messages_arrive_in_order_with_their_details() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let t = tab(&reg, &cfg).await;

    let (res, events, messages) = trace(
        &reg,
        t,
        "SELECT 1; \
         DO $$ BEGIN \
           RAISE NOTICE 'starting %', 42; \
           RAISE WARNING 'careful' USING DETAIL = 'some detail', HINT = 'some hint'; \
           RAISE INFO 'for your information'; \
         END $$; \
         SELECT 2",
    )
    .await;
    res.unwrap();
    // The messages come in the order they were raised. Where they land among the row events is
    // not promised (the connection may run ahead of the consumer), so only compare the two kinds
    // of event separately.
    let only = |prefix: &str| {
        events
            .iter()
            .filter(|e| e.starts_with(prefix))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        only("msg:"),
        [
            "msg:NOTICE:starting 42",
            "msg:WARNING:careful",
            "msg:INFO:for your information"
        ]
    );
    assert_eq!(only("rows:"), ["rows:1", "rows:1"]);
    assert_eq!(only("done").len(), 3);
    let warning = &messages[1];
    assert_eq!(warning.detail.as_deref(), Some("some detail"));
    assert_eq!(warning.hint.as_deref(), Some("some hint"));
    assert!(
        warning
            .context
            .as_deref()
            .unwrap_or("")
            .contains("PL/pgSQL function inline_code_block"),
        "{warning:?}"
    );
    assert_eq!(warning.sqlstate, "01000");

    // Ordinary server notices count too, and the next statement starts with a clean slate.
    let (_, _, messages) = trace(&reg, t, "DROP TABLE IF EXISTS slon_surely_missing").await;
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0].severity, "NOTICE");
    assert!(messages[0].message.contains("does not exist, skipping"));
    let (_, events, _) = trace(&reg, t, "SELECT 1").await;
    assert_eq!(events, ["rows:1", "done"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn messages_are_delivered_while_the_statement_is_still_running() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let t = tab(&reg, &cfg).await;
    let started = Instant::now();
    let mut first_at = None;
    let mut last_at = None;
    reg.run_query(
        t,
        "DO $$ BEGIN RAISE NOTICE 'one'; PERFORM pg_sleep(1.2); RAISE NOTICE 'two'; END $$",
        &QueryOptions::default(),
        |ev| {
            if let QueryEvent::Message { message } = ev {
                let now = started.elapsed();
                if message.message == "one" {
                    first_at = Some(now);
                } else {
                    last_at = Some(now);
                }
            }
        },
    )
    .await
    .unwrap();
    let (first, last) = (first_at.unwrap(), last_at.unwrap());
    assert!(
        first < Duration::from_millis(900),
        "first message waited for the end: {first:?}"
    );
    assert!(last >= Duration::from_millis(1100), "{last:?}");
}

#[tokio::test]
async fn messages_raised_before_an_error_are_still_delivered() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let t = tab(&reg, &cfg).await;
    let (res, events, _) = trace(
        &reg,
        t,
        "DO $$ BEGIN RAISE NOTICE 'about to fail'; PERFORM 1/0; END $$",
    )
    .await;
    assert_eq!(res.unwrap_err().sqlstate.as_deref(), Some("22012"));
    assert_eq!(events, ["msg:NOTICE:about to fail"]);
    let (res, _, _) = trace(&reg, t, "SELECT 1").await;
    res.unwrap();
}

#[tokio::test]
async fn a_flood_of_messages_is_capped() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let t = tab(&reg, &cfg).await;
    let (res, _, messages) = trace(
        &reg,
        t,
        "DO $$ BEGIN FOR i IN 1..10050 LOOP RAISE NOTICE 'n%', i; END LOOP; END $$",
    )
    .await;
    res.unwrap();
    assert_eq!(
        messages.len(),
        10_001,
        "10,000 delivered plus one note about the rest"
    );
    assert_eq!(messages[0].message, "n1");
    assert_eq!(messages[9_999].message, "n10000");
    assert!(
        messages[10_000]
            .message
            .starts_with("50 more messages were not shown"),
        "{:?}",
        messages[10_000]
    );
}

#[tokio::test]
async fn table_details_list_triggers_and_sequences_and_the_script_replays() {
    let cfg = cfg_or_skip!();
    let reg = Registry::new();
    let conn = reg.connect(cfg.clone()).await.expect("connect");
    let t = reg.open_tab(conn.id, None).await.expect("tab");
    let q = QueryOptions::default();
    run(
        &reg,
        t,
        "DROP SCHEMA IF EXISTS slon_seq CASCADE; CREATE SCHEMA slon_seq; \
         CREATE FUNCTION slon_seq.touch() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN NEW.note := coalesce(NEW.note, 'x'); RETURN NEW; END $$; \
         CREATE TABLE slon_seq.jobs ( \
             id serial PRIMARY KEY, \
             ticket bigint GENERATED BY DEFAULT AS IDENTITY (START WITH 100 INCREMENT BY 5 CACHE 10), \
             plain int GENERATED ALWAYS AS IDENTITY, \
             note text); \
         CREATE TRIGGER a_touch BEFORE INSERT OR UPDATE ON slon_seq.jobs FOR EACH ROW EXECUTE FUNCTION slon_seq.touch(); \
         CREATE TRIGGER b_after AFTER DELETE ON slon_seq.jobs FOR EACH STATEMENT EXECUTE FUNCTION slon_seq.touch(); \
         ALTER TABLE slon_seq.jobs DISABLE TRIGGER b_after; \
         INSERT INTO slon_seq.jobs (note) VALUES ('first'), ('second')",
        &q,
    )
    .await
    .0
    .expect("setup");

    let d = reg
        .table_details(conn.id, &cfg.database, "slon_seq", "jobs")
        .await
        .expect("details");

    assert_eq!(d.triggers.len(), 2);
    let a = &d.triggers[0];
    assert_eq!(
        (a.name.as_str(), a.timing.as_str(), a.level.as_str()),
        ("a_touch", "BEFORE", "ROW")
    );
    assert_eq!(a.events, ["INSERT", "UPDATE"]);
    assert!(a.function.contains("touch"), "{}", a.function);
    assert_eq!(a.state, TriggerState::Enabled);
    assert!(
        a.definition
            .starts_with("CREATE TRIGGER a_touch BEFORE INSERT OR UPDATE ON slon_seq.jobs"),
        "{}",
        a.definition
    );
    let b = &d.triggers[1];
    assert_eq!(
        (b.timing.as_str(), b.level.as_str(), b.events.as_slice()),
        ("AFTER", "STATEMENT", ["DELETE".to_string()].as_slice())
    );
    assert_eq!(b.state, TriggerState::Disabled);

    assert_eq!(d.sequences.len(), 3, "{:?}", d.sequences);
    let serial = d.sequences.iter().find(|s| s.column == "id").unwrap();
    assert!(!serial.identity);
    assert_eq!(serial.name, "jobs_id_seq");
    assert_eq!(serial.data_type, "integer");
    assert_eq!(
        serial.last_value.as_deref(),
        Some("2"),
        "two rows were inserted"
    );
    let ticket = d.sequences.iter().find(|s| s.column == "ticket").unwrap();
    assert!(ticket.identity);
    assert_eq!(
        (
            ticket.start.as_str(),
            ticket.increment.as_str(),
            ticket.cache.as_str()
        ),
        ("100", "5", "10")
    );
    // With CACHE 10 the server has already reserved a block, so last_value is ahead of the rows.
    assert!(ticket.last_value.is_some());
    let plain = d.sequences.iter().find(|s| s.column == "plain").unwrap();
    assert_eq!(plain.max, "2147483647");

    assert!(
        d.ddl
            .contains("CREATE SEQUENCE slon_seq.jobs_id_seq AS integer"),
        "{}",
        d.ddl
    );
    assert!(
        d.ddl
            .contains("ALTER SEQUENCE slon_seq.jobs_id_seq OWNED BY slon_seq.jobs.id;"),
        "{}",
        d.ddl
    );
    assert!(
        d.ddl
            .contains("GENERATED BY DEFAULT AS IDENTITY (INCREMENT BY 5"),
        "{}",
        d.ddl
    );
    assert!(
        d.ddl
            .contains("ALTER TABLE slon_seq.jobs DISABLE TRIGGER b_after;"),
        "{}",
        d.ddl
    );

    // The script must rebuild the same thing: run it again on an empty slate.
    run(&reg, t, "DROP TABLE slon_seq.jobs", &q)
        .await
        .0
        .expect("drop");
    let (res, _) = run(&reg, t, &d.ddl, &q).await;
    res.unwrap_or_else(|e| panic!("script did not replay: {e:?}\n{}", d.ddl));
    let again = reg
        .table_details(conn.id, &cfg.database, "slon_seq", "jobs")
        .await
        .expect("details again");
    assert_eq!(
        again.ddl, d.ddl,
        "replayed table describes itself identically"
    );
    assert_eq!(again.triggers.len(), 2);
    assert_eq!(again.sequences.len(), 3);

    // A view with an INSTEAD OF trigger.
    run(
        &reg,
        t,
        "CREATE VIEW slon_seq.jobs_v AS SELECT id, note FROM slon_seq.jobs; \
         CREATE TRIGGER v_ins INSTEAD OF INSERT ON slon_seq.jobs_v FOR EACH ROW EXECUTE FUNCTION slon_seq.touch()",
        &q,
    )
    .await
    .0
    .expect("view");
    let v = reg
        .table_details(conn.id, &cfg.database, "slon_seq", "jobs_v")
        .await
        .unwrap();
    assert_eq!(v.triggers.len(), 1);
    assert_eq!(v.triggers[0].timing, "INSTEAD OF");
    assert!(v.sequences.is_empty());
    assert!(
        v.ddl
            .contains("CREATE TRIGGER v_ins INSTEAD OF INSERT ON slon_seq.jobs_v"),
        "{}",
        v.ddl
    );

    run(&reg, t, "DROP SCHEMA slon_seq CASCADE", &q)
        .await
        .0
        .expect("cleanup");
}
