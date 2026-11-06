//! MySQL 8 and MariaDB, which speak the same protocol and share information_schema;
//! where they differ, `is_mariadb` picks the way. A MySQL database is what
//! PostgreSQL calls a schema, so the explorer
//! lists databases and then their tables, and a table's `TableRef` carries the
//! database name in both `database` and `schema`.

mod diagram;
mod edit;
mod explain;
mod structure;

pub use diagram::schema_diagram;
pub use edit::{apply_changes, preview_changes};
pub use explain::explain;
pub use structure::table_structure;

use std::time::Instant;

use either::Either;
use futures_util::TryStreamExt;
use sqlx::mysql::{MySqlConnectOptions, MySqlConnection, MySqlRow};
use sqlx::{AssertSqlSafe, Column as _, Connection as _, Row, TypeInfo};

use super::{
    Column, DbObject, ObjectKind, QueryOutcome, Relation, RelationList, RowSink, RowsPage, StatementResult, TableRef,
    with_timeout,
};
use crate::connections::model::{Connection, Engine};
use crate::error::{AppError, Result};

/// Schemas the server keeps for itself; never shown.
const SYSTEM_DATABASES: &str = "('mysql', 'information_schema', 'performance_schema', 'sys')";

async fn connect(connection: &Connection, password: Option<&str>, database: Option<&str>) -> Result<MySqlConnection> {
    let mut options =
        MySqlConnectOptions::new().host(&connection.host).port(connection.port).username(&connection.username);
    let database = database.unwrap_or(&connection.database);
    if !database.is_empty() {
        options = options.database(database);
    }
    if let Some(password) = password {
        options = options.password(password);
    }
    Ok(MySqlConnection::connect_with(&options).await?)
}

/// MariaDB reports a version like `11.4.13-MariaDB-ubu2404`; only the number is kept.
pub async fn server_version(connection: &Connection, password: Option<&str>) -> Result<String> {
    let mut conn = connect(connection, password, None).await?;
    let version: String = sqlx::query_scalar("SELECT CAST(VERSION() AS CHAR)").fetch_one(&mut conn).await?;
    conn.close().await?;
    let number = version.split('-').next().unwrap_or(&version);
    Ok(format!("{} {number}", connection.engine.label()))
}

fn is_mariadb(connection: &Connection) -> bool {
    connection.engine == Engine::Mariadb
}

/// `[]` → databases, `[database]` → its tables and views.
pub async fn list_objects(connection: &Connection, password: Option<&str>, path: &[String]) -> Result<Vec<DbObject>> {
    let mut conn = connect(connection, password, None).await?;
    let objects = match path {
        [] => {
            let names: Vec<String> = sqlx::query_scalar(AssertSqlSafe(format!(
                "SELECT CAST(schema_name AS CHAR) FROM information_schema.schemata
                 WHERE schema_name NOT IN {SYSTEM_DATABASES} ORDER BY schema_name"
            )))
            .fetch_all(&mut conn)
            .await?;
            names.into_iter().map(|name| DbObject::new(name, ObjectKind::Database)).collect()
        }
        [database] => {
            // table_rows is InnoDB's estimate, like PostgreSQL's reltuples; NULL for views.
            let rows: Vec<(String, String, Option<i64>)> = sqlx::query_as(
                "SELECT CAST(table_name AS CHAR), CAST(table_type AS CHAR), CAST(table_rows AS SIGNED)
                 FROM information_schema.tables WHERE table_schema = ? ORDER BY table_name",
            )
            .bind(database)
            .fetch_all(&mut conn)
            .await?;
            rows.into_iter()
                .map(|(name, kind, rows)| DbObject {
                    name,
                    kind: if kind == "VIEW" { ObjectKind::View } else { ObjectKind::Table },
                    row_estimate: if kind == "VIEW" { None } else { rows },
                })
                .collect()
        }
        _ => return Err(AppError::Invalid(format!("Unexpected path: {}", path.join("/")))),
    };
    conn.close().await?;
    Ok(objects)
}

/// Every table and view of `database` (the connection's own when `None`), for search.
pub async fn list_relations(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
) -> Result<RelationList> {
    let mut conn = connect(connection, password, database).await?;
    let database: Option<String> = sqlx::query_scalar("SELECT CAST(DATABASE() AS CHAR)").fetch_one(&mut conn).await?;
    let database = database.ok_or_else(|| AppError::Invalid("Select a database to search its tables.".into()))?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT CAST(table_name AS CHAR), CAST(table_type AS CHAR)
         FROM information_schema.tables WHERE table_schema = ? ORDER BY table_name",
    )
    .bind(&database)
    .fetch_all(&mut conn)
    .await?;
    conn.close().await?;
    let relations = rows
        .into_iter()
        .map(|(name, kind)| Relation {
            schema: database.clone(),
            name,
            kind: if kind == "VIEW" { ObjectKind::View } else { ObjectKind::Table },
        })
        .collect();
    Ok(RelationList { database, relations })
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
    let columns = table_columns(&mut conn, table).await?;
    let sql = format!("{} LIMIT ? OFFSET ?", select_as_text(&columns, table));
    let mut query = sqlx::query(AssertSqlSafe(sql)).bind(u64::from(limit) + 1).bind(offset).fetch(&mut conn);
    let mut rows = Vec::new();
    while let Some(row) = query.try_next().await? {
        rows.push(text_values(&row));
    }
    drop(query);
    conn.close().await?;

    // One extra row was fetched to know whether another page exists.
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    Ok(RowsPage { columns, rows, has_more })
}

/// Sends every row to `sink` as it arrives, so memory stays flat for any table size.
pub async fn stream_table(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    sink: &mut impl RowSink,
) -> Result<()> {
    let mut conn = with_timeout(connect(connection, password, Some(&table.database))).await?;
    let columns = with_timeout(table_columns(&mut conn, table)).await?;
    sink.columns(&columns)?;
    let sql = select_as_text(&columns, table);
    let mut rows = sqlx::query(AssertSqlSafe(sql)).fetch(&mut conn);
    while let Some(row) = rows.try_next().await? {
        sink.row(&text_values(&row))?;
    }
    drop(rows);
    conn.close().await?;
    Ok(())
}

async fn table_columns(conn: &mut MySqlConnection, table: &TableRef) -> Result<Vec<Column>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT CAST(column_name AS CHAR), CAST(column_type AS CHAR), CAST(column_key AS CHAR)
         FROM information_schema.columns
         WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
    )
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_all(conn)
    .await?;
    if rows.is_empty() {
        return Err(AppError::Invalid(format!("Table {}.{} not found", table.schema, table.name)));
    }
    Ok(rows.into_iter().map(|(name, data_type, key)| Column { name, data_type, primary_key: key == "PRI" }).collect())
}

/// Every value cast to text so any column type can be shown or exported as-is.
fn select_as_text(columns: &[Column], table: &TableRef) -> String {
    let values: Vec<String> = columns.iter().map(|c| format!("CAST({} AS CHAR)", quote_ident(&c.name))).collect();
    let order_by: Vec<String> = columns.iter().filter(|c| c.primary_key).map(|c| quote_ident(&c.name)).collect();
    format!(
        "SELECT {} FROM {}.{}{}",
        values.join(", "),
        quote_ident(&table.schema),
        quote_ident(&table.name),
        if order_by.is_empty() { String::new() } else { format!(" ORDER BY {}", order_by.join(", ")) },
    )
}

/// A connection kept open between runs, so transactions, variables and
/// temporary tables carry over from one run to the next.
pub struct Session {
    conn: MySqlConnection,
    /// The server thread serving this connection; `KILL QUERY` targets it.
    id: u64,
}

pub async fn open_session(connection: &Connection, password: Option<&str>, database: Option<&str>) -> Result<Session> {
    let mut conn = connect(connection, password, database).await?;
    let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()").fetch_one(&mut conn).await?;
    Ok(Session { conn, id })
}

impl Session {
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Runs a script with the text protocol, which allows several statements and
    /// returns every value as text. All rows are read so the script completes,
    /// but only the first `row_limit` of each result are kept.
    pub async fn run(&mut self, sql: &str, row_limit: usize) -> Result<QueryOutcome> {
        let started = Instant::now();
        let mut results = Vec::new();
        let mut current = StatementResult::default();
        // Rows the current statement returned, kept or not.
        let mut returned = 0u64;
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&mut self.conn);
        while let Some(item) = stream.try_next().await? {
            match item {
                Either::Right(row) => {
                    returned += 1;
                    if current.columns.is_empty() {
                        current.columns = columns_of(&row);
                    }
                    if current.rows.len() < row_limit {
                        current.rows.push(text_values(&row));
                    } else {
                        current.truncated = true;
                    }
                }
                // Marks the end of one statement.
                Either::Left(done) => {
                    // A SELECT reports no affected rows; count what it returned instead.
                    current.rows_affected = if current.columns.is_empty() { done.rows_affected() } else { returned };
                    results.push(std::mem::take(&mut current));
                    returned = 0;
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

/// Stops what connection `id` is running; the connection itself stays open.
pub async fn cancel(connection: &Connection, password: Option<&str>, id: u64) -> Result<()> {
    let mut conn = connect(connection, password, None).await?;
    sqlx::raw_sql(AssertSqlSafe(format!("KILL QUERY {id}"))).execute(&mut conn).await?;
    conn.close().await?;
    Ok(())
}

fn columns_of(row: &MySqlRow) -> Vec<Column> {
    row.columns()
        .iter()
        .map(|c| Column {
            name: c.name().to_string(),
            data_type: c.type_info().name().to_lowercase(),
            primary_key: false,
        })
        .collect()
}

/// Values in their text form. The text protocol sends every value as text, so the
/// bytes are read as such (unchecked); binary data that isn't UTF-8 is shown lossily.
fn text_values(row: &MySqlRow) -> Vec<Option<String>> {
    (0..row.len())
        .map(|i| {
            row.try_get_unchecked::<Option<Vec<u8>>, _>(i)
                .ok()
                .flatten()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        })
        .collect()
}

/// Quotes an identifier so any table or column name is safe inside SQL.
fn quote_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// Runs against a real server. Start one and run `cargo test -- --ignored`:
/// `docker run -d --name dbforge-test-mysql -e MYSQL_ROOT_PASSWORD=dbforge -e MYSQL_DATABASE=shop -p 53306:3306 mysql:8.4`
#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::model::Engine;

    pub(super) const PASSWORD: Option<&str> = Some("dbforge");

    pub(super) fn local() -> Connection {
        Connection {
            id: "test-mysql".into(),
            name: "test-mysql".into(),
            engine: Engine::Mysql,
            environment: None,
            // Not `localhost`: Docker on macOS answers slowly on IPv6, which is tried first.
            host: "127.0.0.1".into(),
            port: 53306,
            database: "shop".into(),
            username: "root".into(),
        }
    }

    /// MariaDB on 127.0.0.1:53307 (`docker run -d --name dbforge-test-mariadb
    /// -e MARIADB_ROOT_PASSWORD=dbforge -e MARIADB_DATABASE=shop -p 53307:3306 mariadb:11.4`).
    pub(super) fn mariadb() -> Connection {
        Connection {
            id: "test-mariadb".into(),
            name: "test-mariadb".into(),
            engine: Engine::Mariadb,
            port: 53307,
            ..local()
        }
    }

    /// The tests that hold for both engines run against each of these.
    pub(super) fn servers() -> [Connection; 2] {
        [local(), mariadb()]
    }

    pub(super) async fn run(server: &Connection, sql: &str) -> MySqlConnection {
        let mut conn = connect(server, PASSWORD, None).await.unwrap();
        sqlx::raw_sql(AssertSqlSafe(sql)).execute(&mut conn).await.unwrap();
        conn
    }

    pub(super) fn table(database: &str, name: &str) -> TableRef {
        TableRef { database: database.into(), schema: database.into(), name: name.into() }
    }

    #[test]
    fn identifiers_are_quoted_with_backticks() {
        assert_eq!(quote_ident("we`ird"), "`we``ird`");
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn reports_the_server_version() {
        assert!(server_version(&local(), PASSWORD).await.unwrap().starts_with("MySQL 8."));
        assert!(server_version(&mariadb(), PASSWORD).await.unwrap().starts_with("MariaDB 11."));
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn lists_databases_then_tables_and_views() {
        for server in servers() {
            // Each test reads `local()`; here it means the server under test.
            let local = || server.clone();
            let _conn = run(
                &server,
                "DROP DATABASE IF EXISTS dbforge_list;
             CREATE DATABASE dbforge_list;
             CREATE TABLE dbforge_list.orders (id int PRIMARY KEY);
             CREATE VIEW dbforge_list.recent AS SELECT id FROM dbforge_list.orders;",
            )
            .await;

            let databases = list_objects(&local(), PASSWORD, &[]).await.unwrap();
            let names: Vec<_> = databases.iter().map(|d| d.name.as_str()).collect();
            assert!(names.contains(&"dbforge_list") && !names.contains(&"mysql"), "{names:?}");

            let objects = list_objects(&local(), PASSWORD, &["dbforge_list".into()]).await.unwrap();
            let kinds: Vec<_> = objects.iter().map(|o| (o.name.as_str(), o.kind)).collect();
            assert_eq!(kinds, [("orders", ObjectKind::Table), ("recent", ObjectKind::View)]);

            let relations = list_relations(&local(), PASSWORD, Some("dbforge_list")).await.unwrap();
            assert_eq!(relations.database, "dbforge_list");
            assert_eq!(relations.relations[0].schema, "dbforge_list");
        }
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn pages_rows_as_text_in_primary_key_order() {
        for server in servers() {
            // Each test reads `local()`; here it means the server under test.
            let local = || server.clone();
            let _conn = run(&server,
            "DROP DATABASE IF EXISTS dbforge_rows;
             CREATE DATABASE dbforge_rows;
             CREATE TABLE dbforge_rows.items (id int PRIMARY KEY, label varchar(20), price decimal(6,2), at datetime);
             INSERT INTO dbforge_rows.items VALUES (3, 'c', 1.50, '2026-11-05 10:00:00'), (1, 'a', NULL, NULL), (2, 'b', 2, NULL);",
        )
        .await;
            let items = table("dbforge_rows", "items");

            let page = fetch_rows(&local(), PASSWORD, &items, 1, 1).await.unwrap();
            assert_eq!(page.rows, [vec![Some("2".into()), Some("b".into()), Some("2.00".into()), None]]);
            assert!(page.has_more);
            assert!(page.columns[0].primary_key);
            assert_eq!(page.columns[2].data_type, "decimal(6,2)");

            let last = fetch_rows(&local(), PASSWORD, &items, 2, 5).await.unwrap();
            assert_eq!(last.rows[0][3].as_deref(), Some("2026-11-05 10:00:00"));
            assert!(!last.has_more);
        }
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn sessions_run_scripts_keep_state_and_can_be_cancelled() {
        for server in servers() {
            // Each test reads `local()`; here it means the server under test.
            let local = || server.clone();
            let mut session = open_session(&local(), PASSWORD, None).await.unwrap();
            let outcome = session
            .run("CREATE TEMPORARY TABLE t (a int, b text); INSERT INTO t VALUES (1, 'x'), (2, NULL); SELECT * FROM t ORDER BY a", 10)
            .await
            .unwrap();
            let [_, insert, select] = outcome.results.as_slice() else { panic!("{:?}", outcome.results) };
            assert_eq!(insert.rows_affected, 2);
            assert_eq!(select.rows, [vec![Some("1".into()), Some("x".into())], vec![Some("2".into()), None]]);
            assert_eq!(select.rows_affected, 2);

            // The temporary table survives into the next run: same session.
            let again = session.run("SELECT count(*) FROM t", 10).await.unwrap();
            assert_eq!(again.results[0].rows, [vec![Some("2".into())]]);

            let id = session.id();
            let cancel_soon = async {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                cancel(&local(), PASSWORD, id).await
            };
            let (outcome, cancelled) = tokio::join!(session.run("SELECT SLEEP(30)", 10), cancel_soon);
            cancelled.unwrap();
            // MySQL ends an interrupted SLEEP early and returns 1 instead of failing.
            assert!(outcome.map_or(true, |o| o.results[0].rows == [vec![Some("1".into())]]));
            assert!(session.is_alive().await, "the session stays usable after a cancel");
        }
    }
}
