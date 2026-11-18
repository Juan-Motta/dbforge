//! DBForge: the desktop app, or, started with `--mcp-stdio`, an agent's MCP
//! server over stdio that never opens a window.

fn main() {
    // Agents start this executable with the flag and talk to it on stdio.
    if std::env::args().nth(1).as_deref() == Some(dbforge_core::MCP_STDIO_FLAG) {
        std::process::exit(dbforge_core::run_mcp_stdio());
    }
    std::process::exit(dbforge::run());
}
