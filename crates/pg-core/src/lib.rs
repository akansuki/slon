//! PostgreSQL engine for slon: connections, catalog browsing, streaming query
//! execution and cancellation. Free of any UI/Tauri dependency so it can be tested
//! against a real server.

mod catalog;
mod config;
mod ddl;
mod error;
mod explain;
mod notice;
mod query;
mod registry;
mod rowdelete;
mod session;
mod tls;

pub use catalog::{
    ColumnInfo, CompletionColumn, CompletionRelation, CompletionSchema, ConstraintInfo,
    ConstraintKind, Identity, IndexInfo, Relation, RelationKind, SequenceInfo, TableDetails,
    TriggerInfo, TriggerState,
};
pub use config::{ConnectionConfig, SslMode};
pub use error::{ErrorKind, PgError};
pub use explain::ExplainResult;
pub use notice::ServerMessage;
pub use query::{Column, QueryEvent, QueryOptions, QuerySummary, SortKind};
pub use registry::{ConnectionInfo, Registry};
pub use rowdelete::{
    DeleteOutcome, DeletePlan, DeleteRequest, KeyColumn, RowIdentity, MAX_ROWS as MAX_DELETE_ROWS,
};
