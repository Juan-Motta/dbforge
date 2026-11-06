mod connections;
mod drivers;
mod error;
mod explorer;
mod query;

use tauri::Manager;

use connections::commands;
use connections::store::ConnectionStore;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("connections.json");
            app.manage(ConnectionStore::load(path)?);
            app.manage(query::QuerySessions::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_connections,
            commands::save_connection,
            commands::delete_connection,
            commands::test_connection,
            explorer::list_objects,
            explorer::fetch_rows,
            query::run_query,
            query::cancel_query,
            query::close_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DBForge");
}
