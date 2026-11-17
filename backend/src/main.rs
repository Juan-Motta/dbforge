// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // An agent starting DBForge as its MCP server over stdio: no window, no app.
    if std::env::args().nth(1).as_deref() == Some(dbforge_lib::MCP_STDIO_FLAG) {
        std::process::exit(dbforge_lib::run_mcp_stdio());
    }
    dbforge_lib::run();
}
