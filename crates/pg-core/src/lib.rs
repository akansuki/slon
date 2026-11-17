//! PostgreSQL engine for slon: connections, catalog browsing, streaming query
//! execution and cancellation. Free of any UI/Tauri dependency so it can be tested
//! against a real server.

mod catalog;
mod config;
mod ddl;
mod error;
mod explain;
mod export;
mod gridedit;
mod notice;
mod query;
mod registry;

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
pub use export::{
    export_all, export_footer, export_header, export_row, json_keys, Cell, ExportColumn,
    ExportFormat, ExportOptions,
};
pub use gridedit::{
    DeleteOutcome, DeletePlan, DeleteRequest, KeyColumn, RowIdentity, TableColumn, UpdateOutcome,
    UpdateRequest, MAX_ROWS as MAX_DELETE_ROWS,
};
pub use notice::ServerMessage;
pub use query::{Column, QueryEvent, QueryOptions, QuerySummary, RowSink, SortKind};
pub use registry::{ConnectionInfo, Registry};
