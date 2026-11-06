mod connections;
mod drivers;
mod error;
mod explorer;
mod export;
mod history;
mod query;

use tauri::Manager;

use connections::commands;
use connections::store::ConnectionStore;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("connections.json");
            app.manage(ConnectionStore::load(path)?);
            app.manage(query::QuerySessions::default());
            let history_path = app.path().app_data_dir()?.join("history.jsonl");
            app.manage(history::HistoryStore::load(history_path)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_connections,
            commands::save_connection,
            commands::delete_connection,
            commands::test_connection,
            explorer::list_objects,
            explorer::list_relations,
            explorer::fetch_rows,
            explorer::table_structure,
            query::run_query,
            query::cancel_query,
            query::close_session,
            history::list_history,
            export::format_rows,
            export::export_rows,
            export::export_table,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DBForge");
}
