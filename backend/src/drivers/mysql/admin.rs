//! Databases, roles and users created from a connection's menu.

use sqlx::AssertSqlSafe;

use super::Conn;
use super::{connect, quote_ident};
use crate::connections::model::Connection;
use crate::drivers::{NewObject, or_default};
use crate::error::Result;
use crate::statement_log;

/// Creates `object`. Account names and passwords are quoted by the server
/// (`QUOTE`), which escapes quotes with a backslash. A user's password is
/// recorded in the statement log as `'***'`, never as written.
pub async fn create_object(connection: &Connection, password: Option<&str>, object: &NewObject) -> Result<()> {
    let mut conn = connect(connection, password, None).await?;
    create_on(&mut conn, object).await?;
    conn.close().await?;
    Ok(())
}

async fn create_on(conn: &mut Conn, object: &NewObject) -> Result<()> {
    backslashes_escape(conn).await?;
    let (statement, shown): (String, String) = match object {
        NewObject::Database { name, .. } => {
            let statement = format!("CREATE DATABASE {}", quote_ident(name));
            (statement.clone(), statement)
        }
        NewObject::Role { name, .. } => {
            let statement: String =
                sqlx::query_scalar("SELECT CONCAT('CREATE ROLE ', QUOTE(?))").bind(name).fetch_one(&mut *conn).await?;
            (statement.clone(), statement)
        }
        NewObject::User { name, password: secret, host, .. } => {
            let (account, secret): (String, String) =
                sqlx::query_as("SELECT CONCAT('CREATE USER ', QUOTE(?), '@', QUOTE(?), ' IDENTIFIED BY'), QUOTE(?)")
                    .bind(name)
                    .bind(or_default(host, "%"))
                    .bind(secret)
                    .fetch_one(&mut *conn)
                    .await?;
            (format!("{account} {secret}"), format!("{account} '***'"))
        }
    };
    statement_log::shown_as(shown, sqlx::raw_sql(AssertSqlSafe(statement)).execute(&mut *conn)).await?;
    Ok(())
}

/// `QUOTE` writes `'` as `\'`, which only escapes while backslashes do:
/// a server running with NO_BACKSLASH_ESCAPES would end the literal there.
/// This session drops that mode and keeps the others.
async fn backslashes_escape(conn: &mut Conn) -> Result<()> {
    let modes: String = sqlx::query_scalar("SELECT @@SESSION.sql_mode").fetch_one(&mut *conn).await?;
    if modes.split(',').any(|mode| mode.eq_ignore_ascii_case("NO_BACKSLASH_ESCAPES")) {
        let kept: Vec<&str> =
            modes.split(',').filter(|mode| !mode.eq_ignore_ascii_case("NO_BACKSLASH_ESCAPES")).collect();
        sqlx::query("SET SESSION sql_mode = ?").bind(kept.join(",")).execute(&mut *conn).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, local, mariadb};
    use super::*;
    use crate::drivers::create_object as create;

    async fn run(connection: &Connection, sql: &str) {
        let mut conn = connect(connection, PASSWORD, None).await.unwrap();
        sqlx::raw_sql(AssertSqlSafe(sql.to_string())).execute(&mut conn).await.unwrap();
    }

    async fn count(connection: &Connection, sql: &str, name: &str) -> i64 {
        let mut conn = connect(connection, PASSWORD, None).await.unwrap();
        sqlx::query_scalar(AssertSqlSafe(sql.to_string())).bind(name).fetch_one(&mut conn).await.unwrap()
    }

    async fn creates_everything(server: Connection) {
        let cleanup = "DROP DATABASE IF EXISTS `admin test db`; DROP ROLE IF EXISTS 'admin_test_role'; \
                       DROP USER IF EXISTS 'admin''test'@'%'";
        run(&server, cleanup).await;

        let new = |object| {
            let server = server.clone();
            async move { create(&server, PASSWORD, &object).await }
        };
        new(NewObject::Database { name: "admin test db".into(), collection: None }).await.unwrap();
        new(NewObject::Role { name: "admin_test_role".into(), database: None }).await.unwrap();
        let user = NewObject::User {
            name: "admin'test".into(),
            password: "it's \\ secret".into(),
            host: Some(" ".into()),
            database: None,
        };
        new(user.clone()).await.unwrap();

        let schemata = "SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name = ?";
        assert_eq!(count(&server, schemata, "admin test db").await, 1);
        // MariaDB keeps roles with an empty host, MySQL with `%`.
        assert_eq!(count(&server, "SELECT COUNT(*) FROM mysql.user WHERE user = ?", "admin_test_role").await, 1);
        let users = "SELECT COUNT(*) FROM mysql.user WHERE user = ? AND host = '%'";
        assert_eq!(count(&server, users, "admin'test").await, 1);
        // The user signs in with exactly the password it was given.
        let mut as_user = server.clone();
        as_user.username = "admin'test".into();
        as_user.database = String::new();
        connect(&as_user, Some("it's \\ secret"), None).await.unwrap();

        let again = new(user).await.unwrap_err();
        assert!(again.to_string().contains("CREATE USER failed"), "{again}");
        run(&server, cleanup).await;
    }

    /// With NO_BACKSLASH_ESCAPES a backslash is an ordinary character, so a
    /// quote escaped as `\'` would end the literal.
    async fn quotes_hold_without_backslash_escapes(server: Connection) {
        let cleanup = "DROP DATABASE IF EXISTS admin_injected; DROP USER IF EXISTS 'admin_nbe'@'%'";
        run(&server, cleanup).await;
        let mut conn = connect(&server, PASSWORD, None).await.unwrap();
        sqlx::raw_sql("SET SESSION sql_mode = CONCAT(@@SESSION.sql_mode, ',NO_BACKSLASH_ESCAPES')")
            .execute(&mut conn)
            .await
            .unwrap();
        let secret = "'; CREATE DATABASE admin_injected; -- \\ it's";
        let user = NewObject::User { name: "admin_nbe".into(), password: secret.into(), host: None, database: None };
        create_on(&mut conn, &user).await.unwrap();

        let schemata = "SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name = ?";
        assert_eq!(count(&server, schemata, "admin_injected").await, 0);
        let mut as_user = server.clone();
        as_user.username = "admin_nbe".into();
        as_user.database = String::new();
        connect(&as_user, Some(secret), None).await.unwrap();
        run(&server, cleanup).await;
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn mysql_quotes_hold_without_backslash_escapes() {
        quotes_hold_without_backslash_escapes(local()).await;
    }

    #[tokio::test]
    #[ignore = "needs a local MariaDB on port 53307"]
    async fn mariadb_quotes_hold_without_backslash_escapes() {
        quotes_hold_without_backslash_escapes(mariadb()).await;
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn mysql_creates_a_database_a_role_and_a_user_that_can_sign_in() {
        creates_everything(local()).await;
    }

    #[tokio::test]
    #[ignore = "needs a local MariaDB on port 53307"]
    async fn mariadb_creates_a_database_a_role_and_a_user_that_can_sign_in() {
        creates_everything(mariadb()).await;
    }
}
