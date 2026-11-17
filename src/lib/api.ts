import { Channel, invoke } from '@tauri-apps/api/core';
import type { ExportOptions } from './export';
import type {
  BuildInfo,
  Cell,
  Column,
  DeleteOutcome,
  DeletePlan,
  DeleteRequest,
  CompletionSchema,
  ConnectionConfig,
  ConnectionInfo,
  ExplainResult,
  HistoryEntry,
  HistoryInput,
  PgError,
  Profile,
  ProfileInput,
  QueryEvent,
  QuerySummary,
  Relation,
  RowIdentity,
  TableDetails,
  UpdateOutcome,
  UpdateRequest,
  Workspace,
} from './types';

export type { Cell };

/** Backend errors arrive as serialized `PgError`s; anything else is wrapped. */
export function toPgError(e: unknown): PgError {
  if (e && typeof e === 'object' && 'kind' in e && 'message' in e) return e as PgError;
  return { kind: 'connection', message: e instanceof Error ? e.message : String(e) };
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toPgError(e);
  }
}

export type RowEvent = Exclude<QueryEvent, { type: 'finished' } | { type: 'failed' }>;

/** Tells `runQuery` to stream every row to a file as well as to the grid. */
export interface ExportRequest {
  path: string;
  options: ExportOptions;
}

export interface ExportDone {
  path: string;
  rows: number;
}

export interface RunResult {
  summary: QuerySummary;
  /** Null unless the run was also an export. */
  exported: ExportDone | null;
}

export const api = {
  connect: (config: ConnectionConfig, profileId: string | null = null) =>
    call<ConnectionInfo>('connect', { config, profileId }),
  listProfiles: () => call<Profile[]>('list_profiles'),
  saveProfile: (input: ProfileInput) => call<Profile>('save_profile', { input }),
  deleteProfile: (id: string) => call<void>('delete_profile', { id }),
  disconnect: (connectionId: number) => call<void>('disconnect', { connectionId }),
  listDatabases: (connectionId: number) => call<string[]>('list_databases', { connectionId }),
  listSchemas: (connectionId: number, database: string) =>
    call<string[]>('list_schemas', { connectionId, database }),
  listRelations: (connectionId: number, database: string, schema: string) =>
    call<Relation[]>('list_relations', { connectionId, database, schema }),
  tableDetails: (connectionId: number, database: string, schema: string, name: string) =>
    call<TableDetails>('table_details', { connectionId, database, schema, name }),
  completionSchema: (connectionId: number, database: string) =>
    call<CompletionSchema>('completion_schema', { connectionId, database }),
  openTab: (connectionId: number, database: string | null) =>
    call<number>('open_tab', { connectionId, database }),
  closeTab: (tabId: number) => call<void>('close_tab', { tabId }),
  cancelQuery: (tabId: number) => call<void>('cancel_query', { tabId }),
  /** Opens the native save dialog; resolves to null when it is cancelled. */
  chooseExportPath: (suggestedName: string, label: string, extensions: string[]) =>
    call<string | null>('choose_export_path', { suggestedName, label, extensions }),
  /**
   * Hands the backend a chunk of the grid's rows to format and append. `start` is the index
   * of the first row in the whole file; `start === 0` begins it and `last` closes it.
   */
  writeExportRows: (
    path: string,
    columns: Column[],
    rows: Cell[][],
    options: ExportOptions,
    start: number,
    last: boolean,
  ) => call<void>('write_export_rows', { path, columns, rows, options, start, last }),
  buildInfo: () => call<BuildInfo>('build_info'),
  setWindowTitle: (title: string) => call<void>('set_window_title', { title }),
  rowIdentity: (tabId: number, tableOid: number) =>
    call<RowIdentity>('row_identity', { tabId, tableOid }),
  planDelete: (tabId: number, request: DeleteRequest) =>
    call<DeletePlan>('plan_delete', { tabId, request }),
  deleteRows: (tabId: number, request: DeleteRequest) =>
    call<DeleteOutcome>('delete_rows', { tabId, request }),
  updateCell: (tabId: number, request: UpdateRequest) =>
    call<UpdateOutcome>('update_cell', { tabId, request }),
  historyAdd: (input: HistoryInput) => call<boolean>('history_add', { input }),
  historyList: (search: string | null, server: string | null, limit = 200) =>
    call<HistoryEntry[]>('history_list', { search, server, limit }),
  historyDelete: (id: number) => call<void>('history_delete', { id }),
  historyClear: (server: string | null) => call<void>('history_clear', { server }),
  workspaceLoad: (server: string) => call<Workspace | null>('workspace_load', { server }),
  workspaceSave: (server: string, workspace: Workspace) =>
    call<void>('workspace_save', { server, workspace }),
  explain: (tabId: number, sql: string, analyze: boolean) =>
    call<ExplainResult>('explain', { tabId, sql, analyze }),

  /**
   * Runs `sql`, calling `onEvent` for every column/row/result event in order.
   * Resolves on the terminal `finished` event and rejects on `failed`; both travel
   * on the same ordered channel as the rows, so nothing can arrive after them.
   */
  async runQuery(
    tabId: number,
    sql: string,
    onEvent: (e: RowEvent) => void,
    exportTo: ExportRequest | null = null,
  ): Promise<RunResult> {
    const channel = new Channel<QueryEvent>();
    const terminal = new Promise<QuerySummary>((resolve, reject) => {
      channel.onmessage = (event) => {
        if (event.type === 'finished') resolve(event.summary);
        else if (event.type === 'failed') reject(event.error);
        else onEvent(event);
      };
    });
    // The summary arrives on the channel, after the last row; whether anything was exported
    // comes back as the command's value, which can overtake it. Wait for both.
    const returned = invoke<ExportDone | null>('run_query', {
      tabId,
      sql,
      export: exportTo,
      onEvent: channel,
    }).catch((e) => {
      throw toPgError(e);
    });
    const [summary, exported] = await Promise.all([terminal, returned]);
    return { summary, exported };
  },
};
