//! Databases, roles and logins created from a connection's menu.

use super::{connect, quote_ident};
use crate::connections::model::Connection;
use crate::drivers::NewObject;
use crate::error::Result;

/// Creates `object` at the server level: a database, a server role, or a
/// login with SQL Server authentication.
pub async fn create_object(connection: &Connection, password: Option<&str>, object: &NewObject) -> Result<()> {
    let statement = match object {
        NewObject::Database { name, .. } => format!("CREATE DATABASE {}", quote_ident(name)),
        NewObject::Role { name, .. } => format!("CREATE SERVER ROLE {}", quote_ident(name)),
        NewObject::User { name, password: secret, .. } => {
            format!("CREATE LOGIN {} WITH PASSWORD = {}", quote_ident(name), quote_literal(secret))
        }
    };
    let mut client = connect(connection, password, Some("master")).await?;
    client.simple_query(statement).await?.into_results().await?;
    client.close().await?;
    Ok(())
}

/// A Unicode string literal; inside one only `'` needs escaping.
fn quote_literal(text: &str) -> String {
    format!("N'{}'", text.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, local};
    use super::*;
    use crate::drivers::create_object as create;

    const CLEANUP: &str = "
        IF DB_ID('admin test db') IS NOT NULL DROP DATABASE [admin test db];
        IF SUSER_ID('admin_test_role') IS NOT NULL DROP SERVER ROLE [admin_test_role];
        IF SUSER_ID('admin]test') IS NOT NULL DROP LOGIN [admin]]test];";

    async fn run(sql: &str) {
        let mut client = connect(&local(), PASSWORD, Some("master")).await.unwrap();
        client.simple_query(sql).await.unwrap().into_results().await.unwrap();
    }

    async fn exists(sql: String) -> bool {
        let mut client = connect(&local(), PASSWORD, Some("master")).await.unwrap();
        let row = client.simple_query(sql).await.unwrap().into_row().await.unwrap().unwrap();
        row.get::<i32, _>(0) == Some(1)
    }

    #[test]
    fn literals_escape_quotes() {
        assert_eq!(quote_literal("it's"), "N'it''s'");
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn creates_a_database_a_server_role_and_a_login_that_can_sign_in() {
        run(CLEANUP).await;
        let secret = "It's \\ Secret#2026";
        let new = |object| async move { create(&local(), PASSWORD, &object).await };
        new(NewObject::Database { name: "admin test db".into(), collection: None }).await.unwrap();
        new(NewObject::Role { name: "admin_test_role".into(), database: None }).await.unwrap();
        let login = NewObject::User { name: "admin]test".into(), password: secret.into(), host: None, database: None };
        new(login.clone()).await.unwrap();

        assert!(exists("SELECT CASE WHEN DB_ID('admin test db') IS NULL THEN 0 ELSE 1 END".into()).await);
        let principal = |name: &str, kind: &str| {
            format!("SELECT COUNT(*) FROM sys.server_principals WHERE name = N'{name}' AND type = '{kind}'")
        };
        assert!(exists(principal("admin_test_role", "R")).await);
        assert!(exists(principal("admin]test", "S")).await);
        // The login signs in with exactly the password it was given.
        let mut as_login = local();
        as_login.username = "admin]test".into();
        connect(&as_login, Some(secret), Some("master")).await.unwrap();

        let again = new(login).await.unwrap_err();
        assert!(again.to_string().contains("already exists"), "{again}");
        run(CLEANUP).await;
    }
}
