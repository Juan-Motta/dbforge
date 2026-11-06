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
