use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde::Serialize;

use crate::catalog::{self, CompletionSchema, Relation, TableDetails};
use crate::config::ConnectionConfig;
use crate::error::PgError;
use crate::explain::{self, ExplainResult};
use crate::query::{self, QueryEvent, QueryOptions, QuerySummary};
use crate::session::Session;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub id: u64,
    pub server_version: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
}

struct Server {
    config: ConnectionConfig,
    /// One catalog connection per database: PostgreSQL can't look across databases,
    /// so browsing a database's schemas needs a connection to that database.
    catalogs: tokio::sync::Mutex<HashMap<String, Arc<Session>>>,
}

struct Tab {
    server_id: u64,
    session: Arc<Session>,
}

/// Owns every connection the app has open. Lock scopes never span an `.await`
/// (except the per-server catalog map, which intentionally serialises connects).
#[derive(Default)]
pub struct Registry {
    next_id: AtomicU64,
    servers: RwLock<HashMap<u64, Arc<Server>>>,
    tabs: RwLock<HashMap<u64, Tab>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    fn fresh_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn server(&self, id: u64) -> Result<Arc<Server>, PgError> {
        self.servers
            .read()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| PgError::not_found("connection", id))
    }

    fn tab_session(&self, tab_id: u64) -> Result<Arc<Session>, PgError> {
        self.tabs
            .read()
            .unwrap()
            .get(&tab_id)
            .map(|t| t.session.clone())
            .ok_or_else(|| PgError::not_found("query tab", tab_id))
    }

    pub async fn connect(&self, config: ConnectionConfig) -> Result<ConnectionInfo, PgError> {
        let session = Arc::new(Session::connect(&config).await?);
        let server_version = session.server_version().await?;
        let id = self.fresh_id();
        let info = ConnectionInfo {
            id,
            server_version,
            host: config.host.trim().to_string(),
            port: config.port,
            database: config.database.trim().to_string(),
            user: config.user.trim().to_string(),
        };
        let mut catalogs = HashMap::new();
        catalogs.insert(config.database.trim().to_string(), session);
        self.servers.write().unwrap().insert(
            id,
            Arc::new(Server {
                config,
                catalogs: tokio::sync::Mutex::new(catalogs),
            }),
        );
        Ok(info)
    }

    /// Drops the server entry and every tab that belongs to it, which closes their sockets.
    pub fn disconnect(&self, id: u64) -> Result<(), PgError> {
        self.servers
            .write()
            .unwrap()
            .remove(&id)
            .ok_or_else(|| PgError::not_found("connection", id))?;
        self.tabs.write().unwrap().retain(|_, t| t.server_id != id);
        Ok(())
    }

    async fn catalog_session(&self, id: u64, database: &str) -> Result<Arc<Session>, PgError> {
        let server = self.server(id)?;
        let mut catalogs = server.catalogs.lock().await;
        if let Some(s) = catalogs.get(database) {
            if !s.is_closed() {
                return Ok(s.clone());
            }
        }
        let session = Arc::new(Session::connect(&server.config.with_database(database)).await?);
        catalogs.insert(database.to_string(), session.clone());
        Ok(session)
    }

    pub async fn list_databases(&self, id: u64) -> Result<Vec<String>, PgError> {
        let server = self.server(id)?;
        let session = self.catalog_session(id, &server.config.database).await?;
        catalog::list_databases(&session).await
    }

    pub async fn list_schemas(&self, id: u64, database: &str) -> Result<Vec<String>, PgError> {
        let session = self.catalog_session(id, database).await?;
        catalog::list_schemas(&session).await
    }

    pub async fn list_relations(
        &self,
        id: u64,
        database: &str,
        schema: &str,
    ) -> Result<Vec<Relation>, PgError> {
        let session = self.catalog_session(id, database).await?;
        catalog::list_relations(&session, schema).await
    }

    /// Columns, indexes, constraints and DDL of one table or view.
    pub async fn table_details(
        &self,
        id: u64,
        database: &str,
        schema: &str,
        name: &str,
    ) -> Result<TableDetails, PgError> {
        let session = self.catalog_session(id, database).await?;
        catalog::table_details(&session, schema, name).await
    }

    /// Table and column names of `database`, for editor autocompletion.
    pub async fn completion_schema(
        &self,
        id: u64,
        database: &str,
    ) -> Result<CompletionSchema, PgError> {
        let session = self.catalog_session(id, database).await?;
        catalog::completion_schema(&session).await
    }

    /// Opens a dedicated connection for a query tab. `database` defaults to the one
    /// the server connection was made with.
    pub async fn open_tab(&self, id: u64, database: Option<&str>) -> Result<u64, PgError> {
        let server = self.server(id)?;
        let config = match database {
            Some(db) => server.config.with_database(db),
            None => server.config.clone(),
        };
        let session = Arc::new(Session::connect(&config).await?);
        let tab_id = self.fresh_id();
        self.tabs.write().unwrap().insert(
            tab_id,
            Tab {
                server_id: id,
                session,
            },
        );
        Ok(tab_id)
    }

    pub fn close_tab(&self, tab_id: u64) -> Result<(), PgError> {
        self.tabs
            .write()
            .unwrap()
            .remove(&tab_id)
            .map(|_| ())
            .ok_or_else(|| PgError::not_found("query tab", tab_id))
    }

    pub async fn run_query<F>(
        &self,
        tab_id: u64,
        sql: &str,
        opts: &QueryOptions,
        emit: F,
    ) -> Result<QuerySummary, PgError>
    where
        F: FnMut(QueryEvent),
    {
        let session = self.tab_session(tab_id)?;
        query::run_query(&session, sql, opts, emit).await
    }

    /// Shows the plan of one statement. With `analyze` the statement is executed and rolled back.
    pub async fn explain(
        &self,
        tab_id: u64,
        sql: &str,
        analyze: bool,
    ) -> Result<ExplainResult, PgError> {
        let session = self.tab_session(tab_id)?;
        explain::run_explain(&session, sql, analyze).await
    }

    /// Cancels the statement running in `tab_id`. Safe to call when nothing is running.
    pub async fn cancel_query(&self, tab_id: u64) -> Result<(), PgError> {
        let session = self.tab_session(tab_id)?;
        session.cancel().await
    }
}
