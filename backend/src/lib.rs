mod admin;
mod connections;
mod credentials;
mod cursor;
mod drivers;
mod dump;
mod edit;
mod error;
mod explorer;
mod export;
mod history;
mod keyspace;
mod query;

use tauri::Manager;

use connections::commands;
use connections::store::ConnectionStore;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            cursor::install();
            let config = app.path().app_config_dir()?;
            let credentials = credentials::Credentials::load(config.clone())?;
            app.manage(ConnectionStore::load(config.join("connections.json"), credentials)?);
            app.manage(query::QuerySessions::default());
            app.manage(dump::DumpJobs::default());
            let history_path = app.path().app_data_dir()?.join("history.jsonl");
            app.manage(history::HistoryStore::load(history_path)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_connections,
            commands::save_connection,
            commands::delete_connection,
            commands::duplicate_connection,
            commands::test_connection,
            commands::credential_storage,
            commands::set_credential_storage,
            admin::create_object,
            explorer::list_objects,
            explorer::list_relations,
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
            dump::export_database,
            dump::import_database,
            dump::cancel_dump,
            cursor::keep_cursor,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DBForge");
}
