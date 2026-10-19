// Mirrors the serde types in crates/pg-core (camelCase on the wire).

export type SslMode = 'disable' | 'prefer' | 'require' | 'verify-ca' | 'verify-full';

export interface ConnectionConfig {
  host: string;
  port: number;
  database: string;
  user: string;
  password: string;
  sslMode: SslMode;
  /** PEM file with the CA to trust for verify-ca / verify-full; empty = system store. */
  sslRootCert?: string | null;
}

/** A saved connection. The password itself never reaches the UI, only whether one is stored. */
export interface Profile {
  id: string;
  name: string;
  host: string;
  port: number;
  database: string;
  user: string;
  sslMode: SslMode;
  sslRootCert?: string | null;
  hasPassword: boolean;
  /** Unix seconds of the last successful connect. */
  lastUsedAt: number | null;
}

export interface ProfileInput {
  /** Omit to create a new profile. */
  id?: string | null;
  name: string;
  host: string;
  port: number;
  database: string;
  user: string;
  sslMode: SslMode;
  sslRootCert?: string | null;
  /** A non-empty value stores or replaces the saved password; empty keeps what is saved. */
  password?: string | null;
  forgetPassword?: boolean;
}

export interface ConnectionInfo {
  id: number;
  serverVersion: string;
  host: string;
  port: number;
  database: string;
  user: string;
}

export type ErrorKind = 'connection' | 'query' | 'cancelled' | 'busy' | 'notFound' | 'storage';

export interface PgError {
  kind: ErrorKind;
  message: string;
  sqlstate?: string | null;
  detail?: string | null;
  hint?: string | null;
  /** 1-based character offset into the SQL that was run. */
  position?: number | null;
}

export type SortKind = 'number' | 'boolean' | 'text' | 'unknown';

export interface Column {
  name: string;
  typeName: string | null;
  sortKind: SortKind;
}

export type RelationKind = 'table' | 'view' | 'materializedView' | 'foreignTable';

export interface Relation {
  name: string;
  kind: RelationKind;
}

export type Cell = string | null;

export interface QuerySummary {
  elapsedMs: number;
  statements: number;
  truncated: boolean;
}

/** A message the server sent next to the results (RAISE NOTICE, warnings, VACUUM VERBOSE, ...). */
export interface ServerMessage {
  /** NOTICE, WARNING, INFO, LOG, DEBUG1..5 */
  severity: string;
  message: string;
  detail: string | null;
  hint: string | null;
  /** Call stack when raised inside a function. */
  context: string | null;
  sqlstate: string;
  /** Milliseconds since the run started. */
  elapsedMs: number;
}

export type QueryEvent =
  | { type: 'columns'; resultIndex: number; columns: Column[] }
  | { type: 'rows'; resultIndex: number; rows: Cell[][] }
  | {
      type: 'resultDone';
      resultIndex: number;
      hasColumns: boolean;
      rowsAffected: number;
      rowCount: number;
      truncated: boolean;
    }
  | { type: 'message'; message: ServerMessage }
  | { type: 'finished'; summary: QuerySummary }
  | { type: 'failed'; error: PgError };

export interface ExplainResult {
  /** The plan as the server's JSON document. */
  plan: string;
  analyzed: boolean;
  elapsedMs: number;
}

export interface CompletionColumn {
  name: string;
  dataType: string;
}

export interface CompletionRelation {
  schema: string;
  name: string;
  kind: RelationKind;
  columns: CompletionColumn[];
}

/** Names the editor can complete in one database. */
export interface CompletionSchema {
  defaultSchema: string | null;
  relations: CompletionRelation[];
  truncated: boolean;
}

export type Identity = 'always' | 'byDefault';

export interface ColumnInfo {
  name: string;
  /** Name as it must be written in SQL (quoted when needed). */
  quotedName: string;
  dataType: string;
  notNull: boolean;
  default: string | null;
  identity: Identity | null;
  generated: string | null;
  comment: string | null;
  isPrimaryKey: boolean;
}

export interface IndexInfo {
  name: string;
  definition: string;
  unique: boolean;
  primary: boolean;
  backsConstraint: boolean;
}

export type ConstraintKind = 'primaryKey' | 'unique' | 'foreignKey' | 'check' | 'exclusion';

export interface ConstraintInfo {
  name: string;
  kind: ConstraintKind;
  definition: string;
  validated: boolean;
}

export type TriggerState = 'enabled' | 'disabled' | 'replica' | 'always';

export interface TriggerInfo {
  name: string;
  /** Complete CREATE TRIGGER statement. */
  definition: string;
  timing: string;
  events: string[];
  level: string;
  function: string;
  state: TriggerState;
}

/** A sequence behind a serial or identity column. Numbers are text: bigint exceeds JS precision. */
export interface SequenceInfo {
  schema: string;
  name: string;
  column: string;
  identity: boolean;
  dataType: string;
  start: string;
  increment: string;
  min: string;
  max: string;
  cache: string;
  cycle: boolean;
  lastValue: string | null;
}

export interface TableDetails {
  schema: string;
  name: string;
  qualifiedName: string;
  kind: RelationKind;
  columns: ColumnInfo[];
  indexes: IndexInfo[];
  constraints: ConstraintInfo[];
  triggers: TriggerInfo[];
  sequences: SequenceInfo[];
  viewDefinition: string | null;
  partitionKey: string | null;
  comment: string | null;
  /** CREATE script reconstructed from the catalog. */
  ddl: string;
}

/** One run in the query history. */
export interface HistoryEntry {
  id: number;
  sql: string;
  /** `user@host:port` */
  server: string;
  database: string;
  ok: boolean;
  error: string | null;
  durationMs: number;
  rows: number | null;
  /** Unix milliseconds of the latest run. */
  at: number;
  /** How many times in a row the same statement was run. */
  count: number;
}

export interface HistoryInput {
  sql: string;
  server: string;
  database: string;
  ok: boolean;
  error: string | null;
  durationMs: number;
  rows: number | null;
}

export interface SavedTab {
  title: string;
  database: string;
  sql: string;
}

export interface Workspace {
  tabs: SavedTab[];
  active: number | null;
  savedAt?: number;
}

/** What build is running; shown in the footer and copied into bug reports. */
export interface BuildInfo {
  version: string;
  /** 'windows' | 'macos' | 'linux' | ... as Rust names it. */
  os: string;
  arch: string;
  debug: boolean;
}
