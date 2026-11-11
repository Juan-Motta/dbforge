mod admin;
mod backup;
mod connections;
mod credentials;
mod cursor;
mod drivers;
mod dump;
mod edit;
mod error;
mod explorer;
mod export;
mod files;
mod history;
mod import;
mod jobs;
mod keyspace;
mod mcp;
mod query;
mod statement_log;

use tauri::menu::{Menu, MenuItem, MenuItemKind};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use connections::commands;
use connections::store::ConnectionStore;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .menu(app_menu)
        .on_menu_event(|app, event| {
            if event.id() == CHECK_FOR_UPDATES {
                let _ = app.emit(CHECK_FOR_UPDATES, ());
            }
        })
        .setup(|app| {
            cursor::install();
            let config = app.path().app_config_dir()?;
            let credentials = credentials::Credentials::load(config.clone())?;
            app.manage(ConnectionStore::load(config.join("connections.json"), credentials)?);
            app.manage(connections::tags::TagStore::load(config.join("tags.json"))?);
            app.manage(dump::ToolPaths::load(config.join("tools.json"))?);
            app.manage(query::QuerySessions::default());
            app.manage(jobs::Jobs::default());
            let mcp_settings = mcp::SettingsFile::load(config.join("mcp.json"))?;
            let settings = mcp_settings.get();
            app.manage(mcp_settings);
            app.manage(mcp::McpServer::default());
            app.manage(mcp::Approvals::default());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                mcp::commands::apply(&handle, settings).await;
            });
            app.manage(statement_log::StatementLog::new(2000, statement_log::EventSink(app.handle().clone())));
            // Files that couldn't be read and were set aside, for the window to tell the user.
            let set_aside = [
                app.state::<ConnectionStore>().set_aside(),
                app.state::<connections::tags::TagStore>().set_aside(),
                app.state::<dump::ToolPaths>().set_aside(),
                app.state::<mcp::SettingsFile>().set_aside(),
            ]
            .into_iter()
            .flatten()
            .map(|path| path.display().to_string())
            .collect();
            app.manage(files::SetAside(set_aside));
            let history_path = app.path().app_data_dir()?.join("history.jsonl");
            app.manage(history::HistoryStore::load(history_path)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            files::set_aside_files,
            files::reveal_file,
            commands::list_connections,
            commands::save_connection,
            commands::delete_connection,
            commands::duplicate_connection,
            commands::list_tags,
            commands::save_tags,
            commands::test_connection,
            commands::credential_storage,
            commands::set_credential_storage,
            admin::create_object,
            explorer::list_objects,
            explorer::list_relations,
            explorer::completion_schema,
            explorer::fetch_rows,
            edit::apply_changes,
            edit::preview_changes,
            explorer::table_structure,
            explorer::schema_diagram,
            query::run_query,
            query::explain_query,
            query::cancel_query,
            query::close_session,
            history::list_history,
            keyspace::redis_overview,
            keyspace::redis_scan,
            keyspace::redis_read_key,
            keyspace::redis_edit_key,
            keyspace::redis_create_key,
            export::format_rows,
            export::export_rows,
            export::export_table,
            import::preview_import,
            import::import_table,
            dump::export_database,
            dump::import_database,
            jobs::cancel_job,
            dump::dump_tool,
            dump::set_dump_tool,
            dump::dump_file_exists,
            dump::export_folder_usage,
            dump::clear_export_folder,
            cursor::keep_cursor,
            statement_log::commands::list_statements,
            statement_log::commands::clear_statements,
            mcp::commands::mcp_state,
            mcp::commands::set_mcp_settings,
            mcp::commands::mcp_token,
            mcp::commands::regenerate_mcp_token,
            mcp::commands::answer_mcp_approval,
            backup::commands::export_backup,
            backup::commands::read_backup,
            backup::commands::import_backup,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DBForge");
}

/// The menu item, and the event the window hears, that look for a new version of DBForge.
const CHECK_FOR_UPDATES: &str = "check-for-updates";

/// The system's usual menu, with "Check for Updates…" under About in the app's own menu.
fn app_menu<R: Runtime>(handle: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(handle)?;
    if let Some(MenuItemKind::Submenu(app)) = menu.items()?.first() {
        let check = MenuItem::with_id(handle, CHECK_FOR_UPDATES, "Check for Updates…", true, None::<&str>)?;
        app.insert(&check, 1)?;
    }
    Ok(menu)
}
