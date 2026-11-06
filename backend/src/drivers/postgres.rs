use std::time::Instant;

use either::Either;
use futures_util::TryStreamExt;
use sqlx::postgres::{PgConnectOptions, PgConnection, PgRow};
use sqlx::{AssertSqlSafe, Column as _, Connection as _, Row, TypeInfo, ValueRef};

use super::{Column, DbObject, ObjectKind, QueryOutcome, RowsPage, StatementResult, TableRef};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// Opens a connection, optionally to a database other than the configured one.
/// PostgreSQL can only read a database's schemas from a connection to that database.
async fn connect(connection: &Connection, password: Option<&str>, database: Option<&str>) -> Result<PgConnection> {
    let mut options = PgConnectOptions::new()
        .host(&connection.host)
        .port(connection.port)
        .username(&connection.username);
    let database = database.unwrap_or(&connection.database);
    if !database.is_empty() {
        options = options.database(database);
    }
    if let Some(password) = password {
        options = options.password(password);
    }
    Ok(PgConnection::connect_with(&options).await?)
}

pub async fn server_version(connection: &Connection, password: Option<&str>) -> Result<String> {
    let mut conn = connect(connection, password, None).await?;
    let version: String = sqlx::query_scalar("SHOW server_version").fetch_one(&mut conn).await?;
    conn.close().await?;
    Ok(format!("PostgreSQL {version}"))
}

/// Lists the children of `path`: `[]` → databases, `[db]` → schemas,
/// `[db, schema]` → tables and views.
pub async fn list_objects(connection: &Connection, password: Option<&str>, path: &[String]) -> Result<Vec<DbObject>> {
    let objects = match path {
        [] => {
            let mut conn = connect(connection, password, None).await?;
            let names: Vec<String> = sqlx::query_scalar(
                "SELECT datname FROM pg_database WHERE NOT datistemplate AND datallowconn ORDER BY datname",
            )
            .fetch_all(&mut conn)
            .await?;
            conn.close().await?;
            names.into_iter().map(|name| DbObject::new(name, ObjectKind::Database)).collect()
        }
        [database] => {
            let mut conn = connect(connection, password, Some(database)).await?;
            let names: Vec<String> = sqlx::query_scalar(
                "SELECT nspname FROM pg_namespace
                 WHERE nspname <> 'information_schema' AND nspname NOT LIKE 'pg\\_%'
                 ORDER BY nspname",
            )
            .fetch_all(&mut conn)
            .await?;
            conn.close().await?;
            names.into_iter().map(|name| DbObject::new(name, ObjectKind::Schema)).collect()
        }
        [database, schema] => {
            let mut conn = connect(connection, password, Some(database)).await?;
            // reltuples is the planner's row estimate: cheap, and -1 when never analyzed.
            let rows: Vec<(String, bool, f64)> = sqlx::query_as(
                "SELECT c.relname, c.relkind IN ('r', 'p') AS is_table, c.reltuples::float8
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm')
                 ORDER BY c.relname",
            )
            .bind(schema)
            .fetch_all(&mut conn)
            .await?;
            conn.close().await?;
            rows.into_iter()
                .map(|(name, is_table, estimate)| DbObject {
                    name,
                    kind: if is_table { ObjectKind::Table } else { ObjectKind::View },
                    row_estimate: (estimate >= 0.0).then_some(estimate as i64),
                })
                .collect()
        }
        _ => return Err(AppError::Invalid(format!("Unexpected path: {}", path.join("/")))),
    };
    Ok(objects)
}

/// Reads one page of a table or view, ordered by primary key when there is one.
pub async fn fetch_rows(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    offset: u64,
    limit: u32,
) -> Result<RowsPage> {
    let mut conn = connect(connection, password, Some(&table.database)).await?;

    let columns: Vec<Column> = sqlx::query_as::<_, (String, String, bool)>(
        "SELECT a.attname, format_type(a.atttypid, a.atttypmod), COALESCE(a.attnum = ANY(i.indkey), false)
         FROM pg_attribute a
         JOIN pg_class c ON c.oid = a.attrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         LEFT JOIN pg_index i ON i.indrelid = c.oid AND i.indisprimary
         WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped
         ORDER BY a.attnum",
    )
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_all(&mut conn)
    .await?
    .into_iter()
    .map(|(name, data_type, primary_key)| Column { name, data_type, primary_key })
    .collect();

    if columns.is_empty() {
        return Err(AppError::Invalid(format!("Table {}.{} not found", table.schema, table.name)));
    }

    // Every value is cast to text so any column type can be shown as-is.
    let values = columns.iter().map(|c| format!("{}::text", quote_ident(&c.name)));
    let order_by: Vec<String> = columns.iter().filter(|c| c.primary_key).map(|c| quote_ident(&c.name)).collect();
    let sql = format!(
        "SELECT ARRAY[{}] FROM {}.{}{} LIMIT $1 OFFSET $2",
        values.collect::<Vec<_>>().join(", "),
        quote_ident(&table.schema),
        quote_ident(&table.name),
        if order_by.is_empty() { String::new() } else { format!(" ORDER BY {}", order_by.join(", ")) },
    );

    // Fetch one extra row to know whether another page exists.
    let mut rows: Vec<Vec<Option<String>>> = sqlx::query_scalar(AssertSqlSafe(sql))
        .bind(i64::from(limit) + 1)
        .bind(offset as i64)
        .fetch_all(&mut conn)
        .await?;
    conn.close().await?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    Ok(RowsPage { columns, rows, has_more })
}

/// A connection kept open between runs, so transactions, `SET` and temporary
/// tables carry over from one run to the next.
pub struct Session {
    conn: PgConnection,
    /// The server process serving this connection; cancel requests target it.
    pid: i32,
}

pub async fn open_session(connection: &Connection, password: Option<&str>, database: Option<&str>) -> Result<Session> {
    let mut conn = connect(connection, password, database).await?;
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut conn).await?;
    Ok(Session { conn, pid })
}

impl Session {
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Runs `sql` with the simple query protocol, which allows several statements
    /// and returns every value in its text form. All rows are read so the whole
    /// script completes, but only the first `row_limit` of each result are kept.
    pub async fn run(&mut self, sql: &str, row_limit: usize) -> Result<QueryOutcome> {
        let started = Instant::now();
        let mut results = Vec::new();
        let mut current = StatementResult::default();
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&mut self.conn);
        while let Some(item) = stream.try_next().await? {
            match item {
                Either::Right(row) => {
                    if current.columns.is_empty() {
                        current.columns = columns_of(&row);
                    }
                    if current.rows.len() < row_limit {
                        current.rows.push(text_values(&row)?);
                    } else {
                        current.truncated = true;
                    }
                }
                // Marks the end of one statement.
                Either::Left(done) => {
                    current.rows_affected = done.rows_affected();
                    results.push(std::mem::take(&mut current));
                }
            }
        }
        let elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(QueryOutcome { results, elapsed_ms })
    }

    pub async fn is_alive(&mut self) -> bool {
        self.conn.ping().await.is_ok()
    }
}

/// Asks the server to cancel what process `pid` is running. The request goes
/// over a separate connection because the session's own one is busy.
pub async fn cancel(connection: &Connection, password: Option<&str>, database: Option<&str>, pid: i32) -> Result<()> {
    let mut conn = connect(connection, password, database).await?;
    sqlx::query("SELECT pg_cancel_backend($1)").bind(pid).execute(&mut conn).await?;
    conn.close().await?;
    Ok(())
}

fn columns_of(row: &PgRow) -> Vec<Column> {
    row.columns()
        .iter()
        .map(|c| Column {
            name: c.name().to_string(),
            data_type: c.type_info().name().to_lowercase(),
            primary_key: false,
        })
        .collect()
}

fn text_values(row: &PgRow) -> Result<Vec<Option<String>>> {
    (0..row.len())
        .map(|i| {
            let value = row.try_get_raw(i)?;
            if value.is_null() {
                return Ok(None);
            }
            let text = value.as_str().map_err(|e| AppError::Invalid(e.to_string()))?;
            Ok(Some(text.to_string()))
        })
        .collect()
}

/// Quotes an identifier so any table or column name is safe inside SQL.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Runs against a real server. Start one and run `cargo test -- --ignored`:
/// `docker run --rm -e POSTGRES_PASSWORD=dbforge -e POSTGRES_DB=shop -p 55432:5432 postgres:17-alpine`
/// The test creates the tables it needs.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::model::Engine;

    const PASSWORD: Option<&str> = Some("dbforge");

    fn local() -> Connection {
        Connection {
            id: "test".into(),
            name: "test".into(),
            engine: Engine::Postgres,
            environment: None,
            host: "localhost".into(),
            port: 55432,
            database: "shop".into(),
            username: "postgres".into(),
        }
    }

    fn names(objects: &[DbObject]) -> Vec<&str> {
        objects.iter().map(|o| o.name.as_str()).collect()
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn lists_databases_schemas_and_tables() {
        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        for statement in [
            "DROP SCHEMA IF EXISTS dbforge_it CASCADE",
            "CREATE SCHEMA dbforge_it",
            "CREATE TABLE dbforge_it.items AS SELECT g AS id FROM generate_series(1, 500) g",
            "CREATE VIEW dbforge_it.some_items AS SELECT * FROM dbforge_it.items",
            "ANALYZE dbforge_it.items",
        ] {
            sqlx::query(statement).execute(&mut conn).await.unwrap();
        }

        assert!(server_version(&local(), PASSWORD).await.unwrap().starts_with("PostgreSQL "));

        let databases = list_objects(&local(), PASSWORD, &[]).await.unwrap();
        assert!(names(&databases).contains(&"shop"));
        assert!(!names(&databases).contains(&"template0"));

        let schemas = list_objects(&local(), PASSWORD, &["shop".into()]).await.unwrap();
        assert!(names(&schemas).contains(&"dbforge_it"));
        assert!(!names(&schemas).iter().any(|n| n.starts_with("pg_") || *n == "information_schema"));

        let objects = list_objects(&local(), PASSWORD, &["shop".into(), "dbforge_it".into()]).await.unwrap();
        assert_eq!(
            objects,
            [
                DbObject { name: "items".into(), kind: ObjectKind::Table, row_estimate: Some(500) },
                DbObject { name: "some_items".into(), kind: ObjectKind::View, row_estimate: None },
            ]
        );

        sqlx::query("DROP SCHEMA dbforge_it CASCADE").execute(&mut conn).await.unwrap();
    }

    #[test]
    fn identifiers_are_quoted_and_escaped() {
        assert_eq!(quote_ident("orders"), "\"orders\"");
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn fetches_pages_of_rows_with_nulls_and_odd_names() {
        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        for statement in [
            "DROP SCHEMA IF EXISTS dbforge_rows CASCADE",
            "CREATE SCHEMA dbforge_rows",
            r#"CREATE TABLE dbforge_rows."Odd ""Name""" (id int PRIMARY KEY, label varchar(20), paid numeric(10,2))"#,
            r#"INSERT INTO dbforge_rows."Odd ""Name""" VALUES (3, 'c', NULL), (1, 'a', 12.50), (2, NULL, 0)"#,
        ] {
            sqlx::query(statement).execute(&mut conn).await.unwrap();
        }
        let table = TableRef { database: "shop".into(), schema: "dbforge_rows".into(), name: r#"Odd "Name""#.into() };

        let first = fetch_rows(&local(), PASSWORD, &table, 0, 2).await.unwrap();
        let columns: Vec<_> = first.columns.iter().map(|c| (c.name.as_str(), c.data_type.as_str(), c.primary_key)).collect();
        assert_eq!(columns, [("id", "integer", true), ("label", "character varying(20)", false), ("paid", "numeric(10,2)", false)]);
        let s = |v: &str| Some(v.to_string());
        assert_eq!(first.rows, [vec![s("1"), s("a"), s("12.50")], vec![s("2"), None, s("0.00")]]);
        assert!(first.has_more);

        let last = fetch_rows(&local(), PASSWORD, &table, 2, 2).await.unwrap();
        assert_eq!(last.rows, [vec![s("3"), s("c"), None]]);
        assert!(!last.has_more);

        let missing = TableRef { name: "nope".into(), ..table };
        assert!(fetch_rows(&local(), PASSWORD, &missing, 0, 2).await.is_err());

        sqlx::query("DROP SCHEMA dbforge_rows CASCADE").execute(&mut conn).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn runs_scripts_and_returns_each_statement() {
        let sql = "CREATE TEMP TABLE t (a int, b text);
                   INSERT INTO t VALUES (1, 'x'), (2, NULL);
                   SELECT * FROM t ORDER BY a;
                   SELECT g FROM generate_series(1, 5) g";
        let mut session = open_session(&local(), PASSWORD, None).await.unwrap();
        let outcome = session.run(sql, 3).await.unwrap();
        let [create, insert, select, series] = outcome.results.as_slice() else {
            panic!("expected 4 results, got {:?}", outcome.results);
        };

        assert!(create.columns.is_empty());
        assert_eq!(insert.rows_affected, 2);

        let columns: Vec<_> = select.columns.iter().map(|c| (c.name.as_str(), c.data_type.as_str())).collect();
        assert_eq!(columns, [("a", "int4"), ("b", "text")]);
        assert_eq!(select.rows, [vec![Some("1".into()), Some("x".into())], vec![Some("2".into()), None]]);
        assert!(!select.truncated);

        assert_eq!(series.rows.len(), 3);
        assert!(series.truncated);
        assert_eq!(series.rows_affected, 5);
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn query_errors_carry_the_server_message() {
        let mut session = open_session(&local(), PASSWORD, None).await.unwrap();
        let error = session.run("SELECT 1/0", 10).await.unwrap_err();
        assert_eq!(error.to_string(), "division by zero");

        let duplicate = "CREATE TEMP TABLE uniq (a int PRIMARY KEY); INSERT INTO uniq VALUES (1), (1)";
        let error = session.run(duplicate, 10).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "duplicate key value violates unique constraint \"uniq_pkey\"\nDetail: Key (a)=(1) already exists."
        );
        assert!(session.is_alive().await, "a failed statement must not break the session");
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn sessions_keep_state_between_runs() {
        let mut session = open_session(&local(), PASSWORD, None).await.unwrap();
        session.run("BEGIN; CREATE TEMP TABLE kept (a int); SET application_name = 'dbforge'", 10).await.unwrap();
        session.run("INSERT INTO kept VALUES (1); COMMIT", 10).await.unwrap();

        let outcome = session.run("SELECT count(*) FROM kept; SHOW application_name", 10).await.unwrap();
        assert_eq!(outcome.results[0].rows, [vec![Some("1".into())]]);
        assert_eq!(outcome.results[1].rows, [vec![Some("dbforge".into())]]);
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn running_queries_can_be_cancelled() {
        let mut session = open_session(&local(), PASSWORD, None).await.unwrap();
        let pid = session.pid();
        let cancel_soon = async {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            cancel(&local(), PASSWORD, None, pid).await
        };

        let (outcome, cancelled) = tokio::join!(session.run("SELECT pg_sleep(30)", 10), cancel_soon);
        cancelled.unwrap();
        let error = outcome.unwrap_err();
        assert!(error.to_string().contains("canceling statement"), "{error}");
        assert!(session.run("SELECT 1", 10).await.is_ok(), "the session stays usable after a cancel");
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn wrong_password_is_reported() {
        let error = server_version(&local(), Some("wrong")).await.unwrap_err();
        assert!(error.to_string().contains("password authentication failed"), "{error}");
    }
}
