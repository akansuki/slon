use pg_core::{
    CompletionSchema, ConnectionConfig, ConnectionInfo, ExplainResult, PgError, QueryEvent,
    QueryOptions, Registry, Relation, TableDetails,
};
use pg_profiles::{
    HistoryEntry, HistoryInput, HistoryStore, KeyringSecrets, MemorySecrets, Profile, ProfileInput,
    ProfileStore, SecretStore, Workspace, WorkspaceStore,
};
mod export;

use serde::Serialize;
use tauri::{ipc::Channel, Manager, State};

/// What build this is, for the footer and for bug reports.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BuildInfo {
    /// The version in tauri.conf.json, which is also the one in the installer's name.
    version: String,
    os: String,
    arch: String,
    debug: bool,
}

#[tauri::command]
fn build_info(app: tauri::AppHandle) -> BuildInfo {
    BuildInfo {
        version: app.package_info().version.to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        debug: cfg!(debug_assertions),
    }
}

/// The title bar carries the connection, so it says which server this window talks to.
#[tauri::command]
fn set_window_title(app: tauri::AppHandle, title: String) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&title);
    }
}

/// Name under which passwords appear in the Windows Credential Manager.
const CREDENTIAL_SERVICE: &str = "slon";

struct AppState {
    registry: Registry,
    profiles: ProfileStore,
    history: HistoryStore,
    workspaces: WorkspaceStore,
}

/// Connects. With `profile_id` and an empty password, the saved password of that profile
/// is used (only while host, port and user still match the profile).
#[tauri::command]
async fn connect(
    state: State<'_, AppState>,
    mut config: ConnectionConfig,
    profile_id: Option<String>,
) -> Result<ConnectionInfo, PgError> {
    if let (Some(id), true) = (&profile_id, config.password.is_empty()) {
        if let Some(saved) =
            state
                .profiles
                .password_for_connect(id, &config.host, config.port, &config.user)?
        {
            config.password = saved;
        }
    }
    let info = state.registry.connect(config).await?;
    if let Some(id) = profile_id {
        // Remembering the last-used time is a convenience; never fail a connect over it.
        let _ = state.profiles.touch(&id);
    }
    Ok(info)
}

#[tauri::command]
fn list_profiles(state: State<'_, AppState>) -> Result<Vec<Profile>, PgError> {
    state.profiles.list()
}

#[tauri::command]
fn save_profile(state: State<'_, AppState>, input: ProfileInput) -> Result<Profile, PgError> {
    state.profiles.save(input)
}

#[tauri::command]
fn delete_profile(state: State<'_, AppState>, id: String) -> Result<(), PgError> {
    state.profiles.delete(&id)
}

#[tauri::command]
fn disconnect(state: State<'_, AppState>, connection_id: u64) -> Result<(), PgError> {
    state.registry.disconnect(connection_id)
}

#[tauri::command]
async fn list_databases(
    state: State<'_, AppState>,
    connection_id: u64,
) -> Result<Vec<String>, PgError> {
    state.registry.list_databases(connection_id).await
}

#[tauri::command]
async fn list_schemas(
    state: State<'_, AppState>,
    connection_id: u64,
    database: String,
) -> Result<Vec<String>, PgError> {
    state.registry.list_schemas(connection_id, &database).await
}

#[tauri::command]
async fn list_relations(
    state: State<'_, AppState>,
    connection_id: u64,
    database: String,
    schema: String,
) -> Result<Vec<Relation>, PgError> {
    state
        .registry
        .list_relations(connection_id, &database, &schema)
        .await
}

#[tauri::command]
async fn table_details(
    state: State<'_, AppState>,
    connection_id: u64,
    database: String,
    schema: String,
    name: String,
) -> Result<TableDetails, PgError> {
    state
        .registry
        .table_details(connection_id, &database, &schema, &name)
        .await
}

#[tauri::command]
async fn completion_schema(
    state: State<'_, AppState>,
    connection_id: u64,
    database: String,
) -> Result<CompletionSchema, PgError> {
    state
        .registry
        .completion_schema(connection_id, &database)
        .await
}

#[tauri::command]
async fn open_tab(
    state: State<'_, AppState>,
    connection_id: u64,
    database: Option<String>,
) -> Result<u64, PgError> {
    state
        .registry
        .open_tab(connection_id, database.as_deref())
        .await
}

#[tauri::command]
fn close_tab(state: State<'_, AppState>, tab_id: u64) -> Result<(), PgError> {
    state.registry.close_tab(tab_id)
}

/// Streams `QueryEvent`s over `on_event`. Completion is signalled by a final
/// `finished` / `failed` event on the same channel, so the frontend sees it strictly
/// after the last row batch. The command itself only errors if the channel is gone.
#[tauri::command]
async fn run_query(
    state: State<'_, AppState>,
    tab_id: u64,
    sql: String,
    on_event: Channel<QueryEvent>,
) -> Result<(), PgError> {
    let result = state
        .registry
        .run_query(tab_id, &sql, &QueryOptions::default(), |event| {
            // The frontend may have navigated away; losing events then is fine.
            let _ = on_event.send(event);
        })
        .await;
    let terminal = match result {
        Ok(summary) => QueryEvent::Finished { summary },
        Err(error) => QueryEvent::Failed { error },
    };
    let _ = on_event.send(terminal);
    Ok(())
}

#[tauri::command]
async fn explain(
    state: State<'_, AppState>,
    tab_id: u64,
    sql: String,
    analyze: bool,
) -> Result<ExplainResult, PgError> {
    state.registry.explain(tab_id, &sql, analyze).await
}

#[tauri::command]
fn history_add(state: State<'_, AppState>, input: HistoryInput) -> Result<bool, PgError> {
    state.history.add(input)
}

#[tauri::command]
fn history_list(
    state: State<'_, AppState>,
    search: Option<String>,
    server: Option<String>,
    limit: Option<usize>,
) -> Vec<HistoryEntry> {
    state.history.list(
        search.as_deref(),
        server.as_deref(),
        limit.unwrap_or(200).min(1000),
    )
}

#[tauri::command]
fn history_delete(state: State<'_, AppState>, id: u64) -> Result<(), PgError> {
    state.history.delete(id)
}

#[tauri::command]
fn history_clear(state: State<'_, AppState>, server: Option<String>) -> Result<(), PgError> {
    state.history.clear(server.as_deref())
}

#[tauri::command]
fn workspace_load(state: State<'_, AppState>, server: String) -> Option<Workspace> {
    state.workspaces.load(&server)
}

#[tauri::command]
fn workspace_save(
    state: State<'_, AppState>,
    server: String,
    workspace: Workspace,
) -> Result<(), PgError> {
    state.workspaces.save(&server, workspace)
}

#[tauri::command]
async fn cancel_query(state: State<'_, AppState>, tab_id: u64) -> Result<(), PgError> {
    state.registry.cancel_query(tab_id).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(export::Exports::default())
        .setup(|app| {
            let dir = app
                .path()
                .app_config_dir()
                .map_err(|e| format!("no config folder: {e}"))?;
            // Debug builds can keep passwords in memory instead of the real credential
            // store (used by the end-to-end test so it never touches the user's vault).
            let secrets: Box<dyn SecretStore> =
                if cfg!(debug_assertions) && std::env::var_os("SLON_MEMORY_SECRETS").is_some() {
                    Box::new(MemorySecrets::new())
                } else {
                    Box::new(KeyringSecrets::new(CREDENTIAL_SERVICE))
                };
            app.manage(AppState {
                registry: Registry::default(),
                profiles: ProfileStore::new(dir.join("profiles.json"), secrets),
                history: HistoryStore::new(dir.join("history.json")),
                workspaces: WorkspaceStore::new(dir.join("workspaces.json")),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            save_profile,
            delete_profile,
            connect,
            disconnect,
            list_databases,
            list_schemas,
            list_relations,
            table_details,
            completion_schema,
            open_tab,
            close_tab,
            run_query,
            explain,
            cancel_query,
            build_info,
            set_window_title,
            history_add,
            history_list,
            history_delete,
            history_clear,
            workspace_load,
            workspace_save,
            export::choose_export_path,
            export::write_export,
        ])
        .run(tauri::generate_context!())
        .expect("error while running slon");
}
