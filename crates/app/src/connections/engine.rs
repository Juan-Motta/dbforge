//! What the connection form knows of each engine beyond the core's
//! [`Engine`]: the order it lists them in, how a connection reaches it, its
//! default port and the group it is listed under. SSL support and tunnels
//! come from the core, so validation and the form agree.

use dbforge_core::connections::model::Engine;
use dbforge_core::connections::transport::SslMode;

/// Every engine, in the order the form lists them.
pub const ENGINES: [Engine; 8] = [
    Engine::Postgres,
    Engine::Mysql,
    Engine::Mariadb,
    Engine::Sqlite,
    Engine::Sqlserver,
    Engine::Mongodb,
    Engine::Cosmosdb,
    Engine::Redis,
];

/// What a connection is: a server, a file (no host, port or user), or an
/// account URI and key, as Azure gives them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Server,
    File,
    Account,
}

pub fn reach(engine: Engine) -> Reach {
    match engine {
        Engine::Sqlite => Reach::File,
        Engine::Cosmosdb => Reach::Account,
        _ => Reach::Server,
    }
}

pub fn is_file(engine: Engine) -> bool {
    reach(engine) == Reach::File
}

pub fn is_account(engine: Engine) -> bool {
    reach(engine) == Reach::Account
}

/// The server's port when the user gives none; 0 for a file.
pub fn default_port(engine: Engine) -> u16 {
    engine.default_port()
}

/// Listed under "Relational" rather than "NoSQL".
pub fn is_relational(engine: Engine) -> bool {
    matches!(engine, Engine::Postgres | Engine::Mysql | Engine::Mariadb | Engine::Sqlite | Engine::Sqlserver)
}

/// The SSL mode a new connection starts with: `prefer` where the engine has it, else none.
pub fn default_ssl_mode(engine: Engine) -> SslMode {
    if engine.ssl_support().modes.contains(&SslMode::Prefer) { SslMode::Prefer } else { SslMode::Disable }
}

/// Through an SSH tunnel the driver only sees the tunnel's local address, so
/// only some can check that the certificate names the server.
pub fn ssl_mode_works_through_ssh(engine: Engine, mode: SslMode) -> bool {
    mode != SslMode::VerifyFull || engine.ssl_support().verifies_name_through_ssh
}
