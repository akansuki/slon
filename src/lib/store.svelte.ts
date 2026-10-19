import { api, toPgError } from './api';
import { looksLikeDdl, quoteIfNeeded } from './completion';
import { parsePlan, type ParsedPlan } from './plan';
import { diagnostics } from './about';
import { copyText } from './selection';
import type { SortSpec } from './sort';
import type {
  BuildInfo,
  Cell,
  Column,
  CompletionSchema,
  ConnectionConfig,
  ConnectionInfo,
  ColumnInfo,
  ConstraintInfo,
  IndexInfo,
  PgError,
  Relation,
  SequenceInfo,
  ServerMessage,
  TableDetails,
  TriggerInfo,
} from './types';

/* ------------------------------------------------------------------ query tabs */

export type TabStatus = 'opening' | 'idle' | 'running';

export class QueryTab {
  readonly key: number;
  readonly database: string;
  title = $state('Query');
  sql = $state('');
  status = $state<TabStatus>('opening');

  // The result shown in the grid. `rows` is mutated in place while streaming (it can
  // hold 100k entries, so it must not be a deep reactive proxy); `rowsVersion` tells
  // the UI that it changed.
  columns = $state.raw<Column[]>([]);
  rows = $state.raw<Cell[][]>([]);
  rowsVersion = $state(0);
  truncated = $state(false);
  /** Sort chosen in the grid (kept here so an export follows the order on screen). */
  sort = $state<SortSpec | null>(null);

  /** Text for non-row results ("INSERT 0 3"-style) and the status bar. */
  message = $state<string | null>(null);
  elapsedMs = $state<number | null>(null);
  error = $state.raw<PgError | null>(null);
  /** Offset of the executed text inside the editor (non-zero when running a selection). */
  errorOffset = 0;
  /** Bumped on every failed run so the editor can react to a repeated identical error. */
  errorSeq = $state(0);

  /** What the result area shows: the grid, or the plan of the last EXPLAIN. */
  view = $state<'results' | 'messages' | 'plan'>('results');
  /** Server messages of the last run (RAISE NOTICE, warnings). Mutated in place like `rows`. */
  messages = $state.raw<ServerMessage[]>([]);
  messagesVersion = $state(0);
  plan = $state.raw<ParsedPlan | null>(null);
  planJson = '';
  /** The statement the plan belongs to, for the header of the plan view. */
  planSql = $state('');

  /** Table and column names of this tab's database, for autocompletion (best effort). */
  completion = $state.raw<CompletionSchema | null>(null);
  completionLoading = $state(false);
  private completionSeq = 0;

  private backendId: number | null = null;
  private opening: Promise<void>;

  constructor(
    key: number,
    private readonly connectionId: number,
    database: string,
    sql = '',
    /** Restored tabs connect when first used, so reopening 20 tabs does not open 20 connections. */
    lazy = false,
  ) {
    this.key = key;
    this.database = database;
    this.sql = sql;
    if (lazy) {
      this.status = 'idle';
      this.opening = Promise.resolve();
    } else {
      this.opening = this.open();
    }
  }

  /** Load the autocompletion names once, e.g. when a restored tab is first shown. */
  ensureCompletion() {
    if (!this.completion && !this.completionLoading) void this.loadCompletion();
  }

  private async open() {
    this.status = 'opening';
    try {
      this.backendId = await api.openTab(this.connectionId, this.database);
      this.status = 'idle';
      this.error = null;
      void this.loadCompletion();
    } catch (e) {
      this.status = 'idle';
      this.error = toPgError(e);
      this.errorOffset = 0;
      this.errorSeq++;
    }
  }

  /**
   * (Re)load the names used for autocompletion. Failures are ignored: completion is a
   * convenience and must never get in the way of running queries.
   */
  async loadCompletion() {
    const seq = ++this.completionSeq;
    this.completionLoading = true;
    try {
      const schema = await api.completionSchema(this.connectionId, this.database);
      if (seq === this.completionSeq) this.completion = schema;
    } catch {
      /* keep whatever we had */
    } finally {
      if (seq === this.completionSeq) this.completionLoading = false;
    }
  }

  private messagesTimer: ReturnType<typeof setTimeout> | undefined;

  /**
   * Tell the UI that `messages` grew, at most about ten times a second: a loop that raises a
   * notice per row would otherwise make it redraw the list thousands of times.
   */
  private scheduleMessagesUpdate() {
    if (this.messagesTimer !== undefined) return;
    this.messagesTimer = setTimeout(() => this.flushMessages(), 100);
  }

  private flushMessages() {
    clearTimeout(this.messagesTimer);
    this.messagesTimer = undefined;
    this.messagesVersion++;
  }

  /** Re-open the dedicated connection after it was lost. */
  async reconnect() {
    this.backendId = null;
    this.opening = this.open();
    await this.opening;
  }

  get connected() {
    return this.backendId !== null;
  }

  async run(text: string, offset = 0) {
    await this.opening;
    if (this.status === 'running') return;
    if (this.backendId === null) {
      // Opening failed earlier; try once more before giving up.
      await this.reconnect();
      if (this.backendId === null) return;
    }
    if (!text.trim()) return;

    this.status = 'running';
    this.view = 'results';
    this.messages = [];
    this.flushMessages();
    this.error = null;
    this.message = null;
    this.elapsedMs = null;
    this.truncated = false;
    this.columns = [];
    this.rows = [];
    this.rowsVersion++;

    const started = performance.now();
    let lastDoneAffected = 0;
    let switchedToMessages = false;
    try {
      const summary = await api.runQuery(this.backendId, text, (ev) => {
        switch (ev.type) {
          case 'columns':
            // A later result set replaces the earlier one; the grid shows the last.
            this.columns = ev.columns;
            this.rows = [];
            this.truncated = false;
            this.rowsVersion++;
            break;
          case 'rows': {
            const target = this.rows;
            for (const r of ev.rows) target.push(r);
            this.rowsVersion++;
            break;
          }
          case 'message':
            this.messages.push(ev.message);
            this.scheduleMessagesUpdate();
            // A statement that only talks (a DO block reporting progress) has nothing else to
            // show yet, so show the messages as they come in.
            if (this.columns.length === 0 && this.view === 'results') {
              this.view = 'messages';
              switchedToMessages = true;
            }
            break;
          case 'resultDone':
            lastDoneAffected = ev.rowsAffected;
            if (ev.hasColumns) this.truncated = ev.truncated;
            break;
        }
      });
      this.elapsedMs = summary.elapsedMs;
      app.recordRun(this.database, text, {
        ok: true,
        error: null,
        durationMs: summary.elapsedMs,
        rows: this.columns.length > 0 ? this.rows.length : summary.statements === 1 ? lastDoneAffected : null,
      });
      // New tables and columns should be completable right away.
      if (looksLikeDdl(text)) void this.loadCompletion();
      if (this.columns.length === 0) {
        this.message =
          summary.statements === 0
            ? 'Nothing to run.'
            : summary.statements === 1
              ? `Command completed successfully. ${lastDoneAffected} row${lastDoneAffected === 1 ? '' : 's'} affected.`
              : `${summary.statements} statements completed successfully.`;
      }
    } catch (e) {
      this.elapsedMs = Math.round(performance.now() - started);
      this.error = toPgError(e);
      this.errorOffset = offset;
      this.errorSeq++;
      app.recordRun(this.database, text, {
        ok: false,
        error: this.error.kind === 'cancelled' ? 'Cancelled' : this.error.message,
        durationMs: this.elapsedMs,
        rows: null,
      });
    } finally {
      this.status = 'idle';
      this.rowsVersion++;
      this.flushMessages();
      // Rows turned up after all: they are what the person wants to see.
      if (switchedToMessages && this.columns.length > 0) this.view = 'results';
    }
  }

  /**
   * Show the execution plan of `text`. With `analyze` the statement is really executed
   * (and rolled back), which gives actual timings and row counts.
   */
  async explain(text: string, offset: number, analyze: boolean) {
    await this.opening;
    if (this.status === 'running') return;
    if (this.backendId === null) {
      await this.reconnect();
      if (this.backendId === null) return;
    }
    if (!text.trim()) return;

    this.status = 'running';
    this.error = null;
    this.elapsedMs = null;
    const started = performance.now();
    try {
      const result = await api.explain(this.backendId, text, analyze);
      let plan: ParsedPlan;
      try {
        plan = parsePlan(result.plan);
      } catch (e) {
        throw { kind: 'query', message: e instanceof Error ? e.message : String(e) } satisfies PgError;
      }
      this.plan = plan;
      this.planJson = result.plan;
      this.planSql = text.trim();
      this.view = 'plan';
      this.elapsedMs = result.elapsedMs;
    } catch (e) {
      this.elapsedMs = Math.round(performance.now() - started);
      this.error = toPgError(e);
      this.errorOffset = offset;
      this.errorSeq++;
    } finally {
      this.status = 'idle';
    }
  }

  async cancel() {
    if (this.backendId === null || this.status !== 'running') return;
    try {
      await api.cancelQuery(this.backendId);
    } catch {
      // The query result (or failure) is reported by run(); nothing more to do here.
    }
  }

  async close() {
    if (this.backendId === null) return;
    const id = this.backendId;
    this.backendId = null;
    if (this.status === 'running') {
      try {
        await api.cancelQuery(id);
      } catch {
        /* closing anyway */
      }
    }
    try {
      await api.closeTab(id);
    } catch {
      /* already gone */
    }
  }
}

/* ------------------------------------------------------------------ tree */

export type NodeKind =
  | 'server'
  | 'database'
  | 'schema'
  | 'group'
  | 'relation'
  | 'detailGroup'
  | 'detail';

export type DetailPart = 'columns' | 'indexes' | 'constraints' | 'triggers' | 'sequences';

export class TreeNode {
  expanded = $state(false);
  loading = $state(false);
  error = $state<string | null>(null);
  children = $state.raw<TreeNode[] | null>(null);
  /** Loaded when a relation is expanded; backs the DDL actions. */
  details = $state.raw<TableDetails | null>(null);

  constructor(
    readonly id: string,
    readonly kind: NodeKind,
    readonly label: string,
    readonly extra: {
      database?: string;
      schema?: string;
      relation?: Relation;
      group?: 'tables' | 'views';
      /** For `detailGroup` and `detail` nodes: which part of the relation they describe. */
      part?: DetailPart;
      column?: ColumnInfo;
      index?: IndexInfo;
      constraint?: ConstraintInfo;
      trigger?: TriggerInfo;
      sequence?: SequenceInfo;
    } = {},
  ) {}

  get expandable() {
    return this.kind !== 'detail';
  }
}

/* ------------------------------------------------------------------ app */

const LAST_CONNECTION_KEY = 'slon.lastConnection';

export function loadLastConnection(): Partial<ConnectionConfig> {
  try {
    return JSON.parse(localStorage.getItem(LAST_CONNECTION_KEY) ?? '{}');
  } catch {
    return {};
  }
}

function saveLastConnection(c: ConnectionConfig) {
  try {
    // Never persist the password.
    const { password: _password, ...rest } = c;
    localStorage.setItem(LAST_CONNECTION_KEY, JSON.stringify(rest));
  } catch {
    /* storage unavailable */
  }
}

export function quoteIdent(name: string): string {
  return `"${name.replaceAll('"', '""')}"`;
}

class AppState {
  connection = $state.raw<ConnectionInfo | null>(null);
  root = $state.raw<TreeNode | null>(null);
  selected = $state.raw<TreeNode | null>(null);
  tabs = $state.raw<QueryTab[]>([]);
  activeKey = $state<number | null>(null);
  private nextKey = 1;

  get activeTab(): QueryTab | undefined {
    return this.tabs.find((t) => t.key === this.activeKey);
  }

  /* ---- which build this is, and what the title bar says */

  build = $state.raw<BuildInfo | null>(null);

  /** Loaded once at start-up; the footer shows it and bug reports copy it. */
  async loadBuildInfo() {
    try {
      this.build = await api.buildInfo();
    } catch {
      /* the footer falls back to the name alone */
    }
  }

  /** The title bar names the server, which is the thing you can get wrong. */
  private applyTitle() {
    const c = this.connection;
    const title = c ? `${c.user}@${c.host}:${c.port} \u2014 Slon` : 'Slon';
    // The native window title is the one the title bar draws; the document title is kept
    // in step for the tools (and the end-to-end test) that read that instead.
    document.title = title;
    void api.setWindowTitle(title).catch(() => {
      /* the window keeps the title it had */
    });
  }

  /** Copy what a bug report needs. Deliberately carries no host, user or database name. */
  async copyDiagnostics() {
    const text = diagnostics(this.build, this.connection?.serverVersion);
    this.flash((await copyText(text)) ? 'Copied version details' : 'Copy failed: clipboard not available');
  }

  /** Identifies the server in history and in the remembered tabs. */
  get serverKey(): string {
    const c = this.connection;
    return c ? `${c.user}@${c.host}:${c.port}` : '';
  }

  /* ---- query history and remembered tabs */

  historyEnabled = $state(readFlag(HISTORY_KEY, true));
  historyOpen = $state(false);
  /** Bumped whenever an entry is added, so an open history panel can refresh. */
  historyVersion = $state(0);

  setHistoryEnabled(on: boolean) {
    this.historyEnabled = on;
    writeFlag(HISTORY_KEY, on);
  }

  recordRun(database: string, sql: string, r: { ok: boolean; error: string | null; durationMs: number; rows: number | null }) {
    if (!this.historyEnabled || !this.connection) return;
    api
      .historyAdd({ sql, server: this.serverKey, database, ...r })
      .then((stored) => {
        if (stored) this.historyVersion++;
      })
      .catch(() => {
        /* history is a convenience; never get in the way of running queries */
      });
  }

  private restoring = false;
  private saveTimer: ReturnType<typeof setTimeout> | undefined;

  /** Remember the open tabs shortly after the last change. */
  scheduleSave() {
    if (!this.connection || this.restoring) return;
    clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => void this.flushSave(), 700);
  }

  async flushSave() {
    clearTimeout(this.saveTimer);
    this.saveTimer = undefined;
    if (!this.connection || this.restoring) return;
    const idx = this.tabs.findIndex((t) => t.key === this.activeKey);
    // A single empty tab is what a fresh connection starts with: nothing worth restoring.
    const nothing = this.tabs.length <= 1 && this.tabs.every((t) => !t.sql.trim());
    try {
      await api.workspaceSave(this.serverKey, {
        tabs: nothing ? [] : this.tabs.map((t) => ({ title: t.title, database: t.database, sql: t.sql })),
        active: nothing || idx < 0 ? null : idx,
      });
    } catch {
      /* not remembered this time */
    }
  }

  async connect(config: ConnectionConfig, profileId: string | null = null) {
    const info = await api.connect(config, profileId);
    saveLastConnection(config);
    let saved = null;
    try {
      saved = await api.workspaceLoad(`${info.user}@${info.host}:${info.port}`);
    } catch {
      /* start with a fresh tab */
    }
    this.restoring = true;
    this.connection = info;
    this.tabs = [];
    this.activeKey = null;
    const server = new TreeNode(
      `s${info.id}`,
      'server',
      `${info.user}@${info.host}:${info.port}`,
    );
    this.root = server;
    this.selected = server;
    void this.expand(server);
    if (saved && saved.tabs.length > 0) {
      this.tabs = saved.tabs.map((t) => {
        const tab = new QueryTab(this.nextKey++, info.id, t.database || info.database, t.sql, true);
        tab.title = t.title;
        return tab;
      });
      const active = this.tabs[saved.active ?? 0] ?? this.tabs[0];
      this.activeKey = active.key;
      active.ensureCompletion();
      this.flash(`Restored ${saved.tabs.length} tab${saved.tabs.length === 1 ? '' : 's'}`);
    } else {
      this.newTab(info.database);
    }
    this.restoring = false;
    this.applyTitle();
  }

  async disconnect() {
    const info = this.connection;
    const tabs = this.tabs;
    await this.flushSave();
    this.historyOpen = false;
    this.connection = null;
    this.applyTitle();
    this.root = null;
    this.selected = null;
    this.tabs = [];
    this.activeKey = null;
    if (!info) return;
    // Cancel anything still running, then drop every connection of this server.
    await Promise.all(tabs.map((t) => t.cancel()));
    try {
      await api.disconnect(info.id);
    } catch {
      /* already gone */
    }
  }

  newTab(database?: string, sql = '', title?: string) {
    const info = this.connection;
    if (!info) return;
    const db = database ?? this.selectedDatabase() ?? info.database;
    const tab = new QueryTab(this.nextKey++, info.id, db, sql);
    tab.title = title ?? `${db} #${tab.key}`;
    this.tabs = [...this.tabs, tab];
    this.activeKey = tab.key;
    return tab;
  }

  async closeTab(key: number) {
    const idx = this.tabs.findIndex((t) => t.key === key);
    if (idx < 0) return;
    const tab = this.tabs[idx];
    this.tabs = this.tabs.filter((t) => t.key !== key);
    if (this.activeKey === key) {
      this.activeKey = this.tabs[Math.min(idx, this.tabs.length - 1)]?.key ?? null;
    }
    await tab.close();
  }

  selectedDatabase(): string | undefined {
    return this.selected?.extra.database;
  }

  /** Open a tab that browses the first rows of a table or view. */
  browseRelation(node: TreeNode) {
    const { database, schema, relation } = node.extra;
    if (!database || !schema || !relation) return;
    const sql = `SELECT * FROM ${quoteIdent(schema)}.${quoteIdent(relation.name)} LIMIT 1000;`;
    const tab = this.newTab(database, sql, relation.name);
    if (tab) void tab.run(sql);
  }

  /* ---- relation actions (context menu) */

  notice = $state<string | null>(null);
  private noticeTimer: ReturnType<typeof setTimeout> | undefined;

  flash(message: string) {
    this.notice = message;
    clearTimeout(this.noticeTimer);
    this.noticeTimer = setTimeout(() => (this.notice = null), 2200);
  }

  /** The relation node a node belongs to (itself, or the table above a column/index/constraint). */
  async detailsOf(node: TreeNode): Promise<TableDetails | null> {
    const info = this.connection;
    const { database, schema, relation } = node.extra;
    if (!info || !database || !schema || !relation) return null;
    if (node.details) return node.details;
    try {
      node.details = await api.tableDetails(info.id, database, schema, relation.name);
      return node.details;
    } catch (e) {
      this.flash(toPgError(e).message);
      return null;
    }
  }

  async copyDdl(node: TreeNode) {
    const d = await this.detailsOf(node);
    if (!d) return;
    this.flash((await copyText(d.ddl)) ? 'Copied CREATE script' : 'Copy failed: clipboard not available');
  }

  async openDdl(node: TreeNode) {
    const d = await this.detailsOf(node);
    if (!d) return;
    this.newTab(node.extra.database, d.ddl, `${d.name} DDL`);
  }

  async openSelect(node: TreeNode) {
    const d = await this.detailsOf(node);
    if (!d) return;
    const cols = d.columns.map((c) => `    ${c.quotedName}`).join(',\n');
    this.newTab(node.extra.database, `SELECT\n${cols}\nFROM ${d.qualifiedName}\nLIMIT 100;`, d.name);
  }

  async copyName(node: TreeNode, text?: string) {
    let value = text;
    if (value === undefined) {
      const d = node.kind === 'relation' ? await this.detailsOf(node) : null;
      value = d?.qualifiedName ?? quoteIfNeeded(node.label);
    }
    this.flash((await copyText(value)) ? `Copied ${value}` : 'Copy failed: clipboard not available');
  }

  /* ---- lazy tree loading */

  async expand(node: TreeNode) {
    node.expanded = true;
    if (node.children !== null || node.loading) return;
    await this.load(node);
  }

  collapse(node: TreeNode) {
    node.expanded = false;
  }

  async toggle(node: TreeNode) {
    if (!node.expandable) return;
    if (node.expanded) this.collapse(node);
    else await this.expand(node);
  }

  /** Discard what was loaded below `node` and fetch it again. */
  async refresh(node: TreeNode) {
    node.children = null;
    node.details = null;
    node.error = null;
    if (node.expanded) await this.load(node);
  }

  private async load(node: TreeNode) {
    const info = this.connection;
    if (!info) return;
    node.loading = true;
    node.error = null;
    try {
      switch (node.kind) {
        case 'server': {
          const dbs = await api.listDatabases(info.id);
          node.children = dbs.map(
            (d) => new TreeNode(`${node.id}/d:${d}`, 'database', d, { database: d }),
          );
          break;
        }
        case 'database': {
          const database = node.extra.database!;
          const schemas = await api.listSchemas(info.id, database);
          node.children = schemas.map(
            (s) => new TreeNode(`${node.id}/s:${s}`, 'schema', s, { database, schema: s }),
          );
          break;
        }
        case 'schema': {
          const { database, schema } = node.extra as { database: string; schema: string };
          const rels = await api.listRelations(info.id, database, schema);
          const tables = rels.filter((r) => r.kind === 'table' || r.kind === 'foreignTable');
          const views = rels.filter((r) => r.kind === 'view' || r.kind === 'materializedView');
          const group = (g: 'tables' | 'views', label: string, list: Relation[]) => {
            const n = new TreeNode(`${node.id}/g:${g}`, 'group', `${label} (${list.length})`, {
              database,
              schema,
              group: g,
            });
            n.children = list.map(
              (r) =>
                new TreeNode(`${n.id}/r:${r.name}`, 'relation', r.name, {
                  database,
                  schema,
                  relation: r,
                }),
            );
            return n;
          };
          node.children = [
            ...(tables.length ? [group('tables', 'Tables', tables)] : []),
            ...(views.length ? [group('views', 'Views', views)] : []),
          ];
          break;
        }
        case 'relation': {
          const { database, schema, relation } = node.extra as {
            database: string;
            schema: string;
            relation: Relation;
          };
          const d = await api.tableDetails(info.id, database, schema, relation.name);
          node.details = d;
          const part = (p: DetailPart, label: string, items: TreeNode[]) => {
            const n = new TreeNode(`${node.id}/p:${p}`, 'detailGroup', `${label} (${items.length})`, {
              database,
              schema,
              relation,
              part: p,
            });
            n.children = items;
            return n;
          };
          const leaf = (id: string, label: string, extra: TreeNode['extra']) =>
            new TreeNode(`${node.id}/${id}`, 'detail', label, { database, schema, relation, ...extra });
          const groups: TreeNode[] = [
            part(
              'columns',
              'Columns',
              d.columns.map((c) => leaf(`c:${c.name}`, c.name, { part: 'columns', column: c })),
            ),
          ];
          if (d.indexes.length) {
            groups.push(
              part(
                'indexes',
                'Indexes',
                d.indexes.map((i) => leaf(`i:${i.name}`, i.name, { part: 'indexes', index: i })),
              ),
            );
          }
          if (d.constraints.length) {
            groups.push(
              part(
                'constraints',
                'Constraints',
                d.constraints.map((c) => leaf(`k:${c.name}`, c.name, { part: 'constraints', constraint: c })),
              ),
            );
          }
          if (d.triggers.length) {
            groups.push(
              part(
                'triggers',
                'Triggers',
                d.triggers.map((t) => leaf(`t:${t.name}`, t.name, { part: 'triggers', trigger: t })),
              ),
            );
          }
          if (d.sequences.length) {
            groups.push(
              part(
                'sequences',
                'Sequences',
                d.sequences.map((q) =>
                  leaf(`s:${q.schema}.${q.name}`, q.name, { part: 'sequences', sequence: q }),
                ),
              ),
            );
          }
          node.children = groups;
          // Columns are what people look for first.
          groups[0].expanded = true;
          break;
        }
        default:
          node.children = [];
      }
    } catch (e) {
      node.error = toPgError(e).message;
      node.children = null;
    } finally {
      node.loading = false;
    }
  }
}

const HISTORY_KEY = 'slon.historyEnabled';

function readFlag(key: string, fallback: boolean): boolean {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : v === '1';
  } catch {
    return fallback;
  }
}

function writeFlag(key: string, on: boolean) {
  try {
    localStorage.setItem(key, on ? '1' : '0');
  } catch {
    /* not remembered */
  }
}

export const app = new AppState();
