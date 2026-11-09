use serde::{Serialize, Serializer};

/// Errors returned to the UI. They are serialized as their message.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Connection not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0} is not supported yet")]
    Unsupported(&'static str),
    #[error("Timed out after {0} seconds")]
    Timeout(u64),
    #[error("Could not read or write the connections file: {0}")]
    Storage(#[from] std::io::Error),
    #[error("The connections file is invalid: {0}")]
    Format(#[from] serde_json::Error),
    #[error("Keychain error: {0}")]
    Keychain(#[from] keyring::Error),
    #[error("{}", database_message(.0))]
    Database(#[from] sqlx::Error),
    #[error("{}", sqlserver_message(.0))]
    SqlServer(#[from] tiberius::error::Error),
    #[error("{}", mongodb_message(.0))]
    MongoDb(#[from] mongodb::error::Error),
    #[error("{}", redis_message(.0))]
    Redis(#[from] redis::RedisError),
    #[error("{}", http_message(.0))]
    Http(#[from] reqwest::Error),
}

/// Why an HTTP request (Cosmos DB) never got an answer, from the innermost cause:
/// reqwest's own text only says that sending failed.
fn http_message(error: &reqwest::Error) -> String {
    let mut cause = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(inner) = source {
        cause = inner.to_string();
        source = inner.source();
    }
    if error.is_timeout() {
        "The server took too long to answer.".into()
    } else if cause.contains("failed to lookup address") || cause.contains("nodename nor servname") {
        "Could not find the host. Check its name, or that you are on the network that can see it.".into()
    } else if cause.contains("invalid peer certificate") || cause.contains("UnknownIssuer") {
        format!(
            "The server's TLS certificate isn't trusted ({cause}). If the server uses a self-signed \
             certificate, turn on Accept self-signed certificate."
        )
    } else if error.is_connect() {
        format!("Could not reach the server: {cause}")
    } else {
        cause
    }
}

/// The server's reply as redis-cli prints it (`WRONGTYPE Operation against…`).
fn redis_message(error: &redis::RedisError) -> String {
    match (error.code(), error.detail()) {
        (Some("NOAUTH"), _) => "This server requires a password. Enter it in Password.".into(),
        (Some(code), Some(detail)) => format!("{code} {detail}"),
        _ if error.kind() == redis::ErrorKind::AuthenticationFailed => {
            "Authentication failed: wrong user name or password.".into()
        }
        _ if error.to_string().contains("failed to lookup address") => {
            "Could not find the host. Check its name, or that you are on the network that can see it.".into()
        }
        _ if error.is_connection_refusal() => format!("Could not reach the server: {error}"),
        _ if error.to_string().contains("invalid peer certificate") => format!(
            "The server's TLS certificate isn't trusted ({error}). If the server uses a self-signed \
             certificate, turn on Accept self-signed certificate."
        ),
        _ => error.to_string(),
    }
}

/// The server's own message, without the driver's kind and labels.
fn mongodb_message(error: &mongodb::error::Error) -> String {
    use mongodb::error::{ErrorKind, WriteFailure};
    match error.kind.as_ref() {
        ErrorKind::Command(command) => command.message.clone(),
        ErrorKind::Write(WriteFailure::WriteError(write)) => write.message.clone(),
        ErrorKind::Authentication { message, .. } => format!("Authentication failed: {message}"),
        ErrorKind::ServerSelection { message, .. } => format!("Could not reach the server: {message}"),
        other => other.to_string(),
    }
}

/// The server's own message, without tiberius's token and server details.
fn sqlserver_message(error: &tiberius::error::Error) -> String {
    match error {
        tiberius::error::Error::Server(server) => server.message().to_string(),
        other => other.to_string(),
    }
}

/// The server's message with its detail and hint, as psql shows them. sqlx's own
/// text prefixes "error returned from database" and appends a line number that
/// points into PostgreSQL's source code, not into the user's query.
fn database_message(error: &sqlx::Error) -> String {
    let Some(server) = error.as_database_error() else {
        return error.to_string();
    };
    let mut message = server.message().to_string();
    if let Some(pg) = server.try_downcast_ref::<sqlx::postgres::PgDatabaseError>() {
        if let Some(detail) = pg.detail() {
            message.push_str(&format!("\nDetail: {detail}"));
        }
        if let Some(hint) = pg.hint() {
            message.push_str(&format!("\nHint: {hint}"));
        }
    }
    message
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
