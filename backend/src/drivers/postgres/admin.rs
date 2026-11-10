//! Databases, roles and users created from a connection's menu.

use sqlx::AssertSqlSafe;

use super::connect;
use crate::connections::model::Connection;
use crate::drivers::NewObject;
use crate::error::Result;

/// Creates `object`. The server quotes names and passwords itself (`format`
/// with `%I` and `%L`), whatever settings change how literals read.
pub async fn create_object(connection: &Connection, password: Option<&str>, object: &NewObject) -> Result<()> {
    let mut conn = connect(connection, password, None).await?;
    let statement: String = match object {
        NewObject::Database { name, .. } => {
            sqlx::query_scalar("SELECT format('CREATE DATABASE %I', $1::text)").bind(name).fetch_one(&mut conn).await?
        }
        NewObject::Role { name, .. } => {
            sqlx::query_scalar("SELECT format('CREATE ROLE %I', $1::text)").bind(name).fetch_one(&mut conn).await?
        }
        NewObject::User { name, password: secret, .. } => {
            sqlx::query_scalar("SELECT format('CREATE ROLE %I LOGIN PASSWORD %L', $1::text, $2::text)")
                .bind(name)
                .bind(secret)
                .fetch_one(&mut conn)
                .await?
        }
    };
    sqlx::raw_sql(AssertSqlSafe(statement)).execute(&mut conn).await?;
    conn.close().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, local};
    use super::*;
    use crate::drivers::create_object as create;

    async fn scalar(sql: &str, name: &str) -> bool {
        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        sqlx::query_scalar(AssertSqlSafe(sql.to_string())).bind(name).fetch_one(&mut conn).await.unwrap()
    }

    async fn run(sql: &str) {
        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        sqlx::raw_sql(AssertSqlSafe(sql.to_string())).execute(&mut conn).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn creates_a_database_a_role_and_a_user_that_can_sign_in() {
        let (database, role, user) = ("Admin Test DB", "admin_test_role", "admin test \"user\"");
        run(&format!("DROP DATABASE IF EXISTS \"{database}\"")).await;
        run(&format!("DROP ROLE IF EXISTS {role}")).await;
        run("DROP ROLE IF EXISTS \"admin test \"\"user\"\"\"").await;

        let new = |object| async move { create(&local(), PASSWORD, &object).await };
        new(NewObject::Database { name: database.into(), collection: None }).await.unwrap();
        new(NewObject::Role { name: role.into(), database: None }).await.unwrap();
        new(NewObject::User { name: user.into(), password: "it's \\ secret".into(), host: None, database: None })
            .await
            .unwrap();

        assert!(scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)", database).await);
        assert!(!scalar("SELECT rolcanlogin FROM pg_roles WHERE rolname = $1", role).await);
        assert!(scalar("SELECT rolcanlogin FROM pg_roles WHERE rolname = $1", user).await);
        // The user signs in with exactly the password it was given.
        let mut as_user = local();
        as_user.username = user.into();
        connect(&as_user, Some("it's \\ secret"), Some("postgres")).await.unwrap();

        let again = new(NewObject::Role { name: role.into(), database: None }).await.unwrap_err();
        assert!(again.to_string().contains("already exists"), "{again}");

        run(&format!("DROP DATABASE \"{database}\"")).await;
        run(&format!("DROP ROLE {role}")).await;
        run("DROP ROLE \"admin test \"\"user\"\"\"").await;
    }

    #[tokio::test]
    async fn refuses_an_empty_name_or_password_before_connecting() {
        let mut nowhere = local();
        nowhere.port = 1;
        let empty = create(&nowhere, None, &NewObject::Role { name: "  ".into(), database: None }).await.unwrap_err();
        assert_eq!(empty.to_string(), "Name the role.");
        let user = NewObject::User { name: "x".into(), password: String::new(), host: None, database: None };
        assert_eq!(create(&nowhere, None, &user).await.unwrap_err().to_string(), "Give the user a password.");
    }
}
