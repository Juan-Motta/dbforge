//! Databases created from a connection's menu (Cosmos DB users and roles are
//! managed in Azure, not through the account's key).

use reqwest::Method;
use serde_json::json;

use super::client::{Client, Target};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// Creates an empty database, without throughput of its own.
pub async fn create_database(connection: &Connection, password: Option<&str>, name: &str) -> Result<()> {
    let client = Client::new(connection, password)?;
    // The server's conflict reply speaks of items; say what already exists.
    let databases = client.list(&Target::databases(), "Databases").await?;
    if databases.iter().any(|database| database["id"] == name) {
        return Err(AppError::Invalid(format!("Database {name} already exists.")));
    }
    client.send(Method::POST, &Target::databases(), &[], Some(&json!({ "id": name }))).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{KEY, local};
    use super::*;
    use crate::drivers::{NewObject, create_object};

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn creates_a_database_once() {
        let client = Client::new(&local(), KEY).unwrap();
        let _ = client.send(Method::DELETE, &Target::database("admin_test_db"), &[], None).await;

        let database = NewObject::Database { name: "admin_test_db".into(), collection: None };
        create_object(&local(), KEY, &database).await.unwrap();
        let names: Vec<String> = client
            .list(&Target::databases(), "Databases")
            .await
            .unwrap()
            .iter()
            .filter_map(|d| d["id"].as_str().map(String::from))
            .collect();
        assert!(names.contains(&"admin_test_db".to_string()), "{names:?}");

        let again = create_object(&local(), KEY, &database).await.unwrap_err();
        assert_eq!(again.to_string(), "Database admin_test_db already exists.");
        let user = NewObject::User { name: "u".into(), password: "p".into(), host: None, database: None };
        let refused = create_object(&local(), KEY, &user).await.unwrap_err();
        assert_eq!(refused.to_string(), "This connection can't create a user.");

        client.send(Method::DELETE, &Target::database("admin_test_db"), &[], None).await.unwrap();
    }
}
