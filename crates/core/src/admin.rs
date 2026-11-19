use serde::Deserialize;

use crate::connections::store::ConnectionStore;
use crate::drivers::{self, NewObject};
use crate::error::Result;
use crate::statement_log::{Origin, StatementLog};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequest {
    connection_id: String,
    object: NewObject,
}

impl CreateRequest {
    /// A request to create `object` on the server of connection `connection_id`.
    pub fn new(connection_id: impl Into<String>, object: NewObject) -> Self {
        Self { connection_id: connection_id.into(), object }
    }
}

/// Creates a database, role or user on a saved connection's server (see `drivers::create_object`).
pub async fn create_object(store: &ConnectionStore, log: &StatementLog, request: CreateRequest) -> Result<()> {
    store
        .open(&request.connection_id)?
        .run(log, Origin::Admin, async |connection, password| {
            drivers::create_object(connection, password, &request.object).await
        })
        .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropRequest {
    connection_id: String,
    database: String,
}

impl DropRequest {
    /// A request to drop `database` from the server of connection `connection_id`.
    pub fn new(connection_id: impl Into<String>, database: impl Into<String>) -> Self {
        Self { connection_id: connection_id.into(), database: database.into() }
    }
}

/// Drops a database from a saved connection's server (see `drivers::drop_database`).
pub async fn drop_database(store: &ConnectionStore, log: &StatementLog, request: DropRequest) -> Result<()> {
    store
        .open(&request.connection_id)?
        .run(log, Origin::Admin, async |connection, password| {
            drivers::drop_database(connection, password, &request.database).await
        })
        .await
}
