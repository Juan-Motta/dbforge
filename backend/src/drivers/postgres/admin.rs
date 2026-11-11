//! Databases, roles and users created from a connection's menu, and databases dropped from theirs.

use sqlx::AssertSqlSafe;

use super::{connect, quote_ident};
use crate::connections::model::Connection;
use crate::drivers::NewObject;
use crate::error::Result;
use crate::statement_log;

/// Creates `object`. The server quotes names and passwords itself (`format`
/// with `%I` and `%L`), whatever settings change how literals read. A user's
/// password is recorded in the statement log as `'***'`, never as written.
pub async fn create_object(connection: &Connection, password: Option<&str>, object: &NewObject) -> Result<()> {
    let mut conn = connect(connection, password, None).await?;
    let (statement, shown): (String, String) = match object {
        NewObject::Database { name, .. } => {
            let statement: String = sqlx::query_scalar("SELECT format('CREATE DATABASE %I', $1::text)")
                .bind(name)
                .fetch_one(&mut conn)
                .await?;
            (statement.clone(), statement)
        }
        NewObject::Role { name, .. } => {
            let statement: String =
                sqlx::query_scalar("SELECT format('CREATE ROLE %I', $1::text)").bind(name).fetch_one(&mut conn).await?;
            (statement.clone(), statement)
        }
        NewObject::User { name, password: secret, .. } => {
            let (role, secret): (String, String) =
                sqlx::query_as("SELECT format('CREATE ROLE %I LOGIN PASSWORD', $1::text), format('%L', $2::text)")
                    .bind(name)
                    .bind(secret)
                    .fetch_one(&mut conn)
                    .await?;
            (format!("{role} {secret}"), format!("{role} '***'"))
        }
    };
    statement_log::shown_as(shown, sqlx::raw_sql(AssertSqlSafe(statement)).execute(&mut conn)).await?;
    conn.close().await?;
    Ok(())
}

/// Drops the database `name` and everything in it. PostgreSQL can't drop the
/// database a session is on, so this one connects to another; sessions still
/// on `name` make the server refuse, saying so.
pub async fn drop_database(connection: &Connection, password: Option<&str>, name: &str) -> Result<()> {
    let mut conn = connect(connection, password, Some(other_database(connection, name))).await?;
    sqlx::raw_sql(AssertSqlSafe(format!("DROP DATABASE {}", quote_ident(name)))).execute(&mut conn).await?;
    conn.close().await?;
    Ok(())
}

/// A database to connect to other than `name`: the connection's own, else
/// `postgres`, which every server has (or `template1` when dropping that).
fn other_database<'a>(connection: &'a Connection, name: &str) -> &'a str {
    let own = connection.database.trim();
    if !own.is_empty() && own != name {
        own
    } else if name != "postgres" {
        "postgres"
    } else {
        "template1"
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, local};
    use super::*;
    use crate::drivers::create_object as create;
    use crate::drivers::drop_database as drop;
    use crate::statement_log::store::tests::Collected;
    use crate::statement_log::{Origin, StatementLog};

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

        let log = StatementLog::new(50, Collected::default());
        let new = |object| {
            let log = log.clone();
            async move { statement_log::scoped(&log, "pg", Origin::Admin, create(&local(), PASSWORD, &object)).await }
        };
        new(NewObject::Database { name: database.into(), collection: None }).await.unwrap();
        new(NewObject::Role { name: role.into(), database: None }).await.unwrap();
        new(NewObject::User { name: user.into(), password: "it's \\ secret".into(), host: None, database: None })
            .await
            .unwrap();

        let texts: Vec<_> = log.entries().into_iter().map(|e| e.text).collect();
        assert!(texts.iter().any(|t| t.starts_with("CREATE ROLE") && t.ends_with("LOGIN PASSWORD '***'")), "{texts:?}");
        assert!(texts.iter().all(|text| !text.contains("secret")), "{texts:?}");
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

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn drops_a_database_also_the_one_the_connection_opens() {
        let name = "Drop \"test\" db";
        run("DROP DATABASE IF EXISTS \"Drop \"\"test\"\" db\"").await;
        run("CREATE DATABASE \"Drop \"\"test\"\" db\"").await;
        let exists = "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)";

        let log = StatementLog::new(50, Collected::default());
        let mut on_it = local();
        on_it.database = name.into();
        statement_log::scoped(&log, "pg", Origin::Admin, drop(&on_it, PASSWORD, name)).await.unwrap();
        assert!(!scalar(exists, name).await);
        let texts: Vec<_> = log.entries().into_iter().map(|e| e.text).collect();
        assert!(texts.contains(&"DROP DATABASE \"Drop \"\"test\"\" db\"".to_string()), "{texts:?}");

        let gone = drop(&local(), PASSWORD, name).await.unwrap_err();
        assert!(gone.to_string().contains("does not exist"), "{gone}");
    }

    #[test]
    fn connects_to_another_database_than_the_one_dropped() {
        let mut connection = local();
        connection.database = "app".into();
        assert_eq!(other_database(&connection, "reports"), "app");
        assert_eq!(other_database(&connection, "app"), "postgres");
        connection.database = " ".into();
        assert_eq!(other_database(&connection, "postgres"), "template1");
    }
}
