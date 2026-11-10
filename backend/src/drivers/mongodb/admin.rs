//! Databases, roles and users created from a connection's menu.

use mongodb::bson::doc;

use super::connect;
use crate::connections::model::Connection;
use crate::drivers::{NewObject, or_default};
use crate::error::{AppError, Result};

/// Creates `object`. A database is created with its first collection, since
/// MongoDB keeps no empty ones; roles and users go in their database
/// (`admin` by default) without privileges.
pub async fn create_object(connection: &Connection, password: Option<&str>, object: &NewObject) -> Result<()> {
    let client = connect(connection, password).await?;
    match object {
        NewObject::Database { name, collection } => {
            let collection = collection.as_deref().map(str::trim).unwrap_or_default();
            if collection.is_empty() {
                return Err(AppError::Invalid("Name the database's first collection.".into()));
            }
            if client.list_database_names().await?.iter().any(|existing| existing == name) {
                return Err(AppError::Invalid(format!("Database {name} already exists.")));
            }
            client.database(name).create_collection(collection).await?;
        }
        NewObject::Role { name, database } => {
            let command = doc! { "createRole": name, "privileges": [], "roles": [] };
            client.database(or_default(database, "admin")).run_command(command).await?;
        }
        NewObject::User { name, password: secret, database, .. } => {
            let command = doc! { "createUser": name, "pwd": secret, "roles": [] };
            client.database(or_default(database, "admin")).run_command(command).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use mongodb::bson::Document;

    use super::super::tests::{PASSWORD, local};
    use super::*;
    use crate::drivers::create_object as create;

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn creates_a_database_a_role_and_a_user_that_can_sign_in() {
        let logged = connect(&local(), PASSWORD).await.unwrap();
        let client = logged.unlogged();
        let admin = client.database("admin");
        client.database("admin_test_db").drop().await.unwrap();
        let _ = admin.run_command(doc! { "dropRole": "admin_test_role" }).await;
        let _ = admin.run_command(doc! { "dropUser": "admin_test_user" }).await;

        let new = |object| async move { create(&local(), PASSWORD, &object).await };
        let database = NewObject::Database { name: "admin_test_db".into(), collection: Some(" orders ".into()) };
        new(database.clone()).await.unwrap();
        new(NewObject::Role { name: "admin_test_role".into(), database: Some(" ".into()) }).await.unwrap();
        new(NewObject::User {
            name: "admin_test_user".into(),
            password: "it's \\ secret".into(),
            host: None,
            database: None,
        })
        .await
        .unwrap();

        let collections = client.database("admin_test_db").list_collection_names().await.unwrap();
        assert_eq!(collections, ["orders"]);
        let roles = admin.run_command(doc! { "rolesInfo": "admin_test_role" }).await.unwrap();
        assert_eq!(roles.get_array("roles").unwrap().len(), 1);
        // The user signs in with exactly the password it was given.
        let mut as_user = local();
        as_user.username = "admin_test_user".into();
        let signed_in = connect(&as_user, Some("it's \\ secret")).await.unwrap();
        let status: Document =
            signed_in.unlogged().database("admin").run_command(doc! { "connectionStatus": 1 }).await.unwrap();
        let users = status.get_document("authInfo").unwrap().get_array("authenticatedUsers").unwrap();
        assert_eq!(users.len(), 1);

        let again = new(database).await.unwrap_err();
        assert_eq!(again.to_string(), "Database admin_test_db already exists.");
        let without = NewObject::Database { name: "x".into(), collection: None };
        assert_eq!(new(without).await.unwrap_err().to_string(), "Name the database's first collection.");

        client.database("admin_test_db").drop().await.unwrap();
        admin.run_command(doc! { "dropRole": "admin_test_role" }).await.unwrap();
        admin.run_command(doc! { "dropUser": "admin_test_user" }).await.unwrap();
    }
}
