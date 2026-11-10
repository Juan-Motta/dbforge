use std::time::Instant;

use either::Either;
use futures_util::TryStreamExt;
use futures_util::stream::BoxStream;
use sqlx::postgres::{PgConnectOptions, PgConnection, PgQueryResult, PgRow};
use sqlx::{AssertSqlSafe, Column as _, Connection as _, Row, TypeInfo, ValueRef};

mod admin;
mod diagram;
mod edit;
mod read_only;
mod structure;

pub use admin::create_object;
pub use diagram::schema_diagram;
pub use edit::{apply_changes, preview_changes};
pub use read_only::run_read_only;
pub use structure::table_structure;

use super::logged::Logged;
use super::row_filter::{RowFilter, sql_clauses};
use super::{
    Column, DbObject, ObjectKind, QueryOutcome, Relation, RelationList, RowSink, RowsPage, StatementResult, TableRef,
    with_timeout,
};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};
use crate::statement_log;

/// A PostgreSQL connection whose statements are recorded in the statement log.
pub(super) type Conn = Logged<PgConnection>;

/// Opens a connection, optionally to a database other than the configured one.
/// PostgreSQL can only read a database's schemas from a connection to that database.
async fn connect(connection: &Connection, password: Option<&str>, database: Option<&str>) -> Result<Conn> {
    connect_with(connection, password, database, |options| options).await
}

/// [`connect`], with `adjust` applied to the options first, e.g. to set session parameters.
async fn connect_with(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    adjust: impl FnOnce(PgConnectOptions) -> PgConnectOptions,
) -> Result<Conn> {
    let mut options =
        PgConnectOptions::new().host(&connection.host).port(connection.port).username(&connection.username);
    let database = database.unwrap_or(&connection.database);
    if !database.is_empty() {
        options = options.database(database);
    }
    if let Some(password) = password {
        options = options.password(password);
    }
    Ok(Logged::new(PgConnection::connect_with(&adjust(options)).await?, Some(database)))
}

/// Whether the server has a database named `name`.
pub async fn database_exists(connection: &Connection, password: Option<&str>, name: &str) -> Result<bool> {
    let mut conn = connect(connection, password, None).await?;
    let exists = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
        .bind(name)
        .fetch_one(&mut conn)
        .await?;
    conn.close().await?;
    Ok(exists)
}

/// Creates an empty database named `name`, owned by the connection's user.
pub async fn create_database(connection: &Connection, password: Option<&str>, name: &str) -> Result<()> {
    run_on_server(connection, password, &format!("CREATE DATABASE {}", quote_ident(name))).await
}

/// Drops `name`, first ending the sessions still on it (a tool that was
/// killed leaves its session running on the server until then).
pub async fn drop_database(connection: &Connection, password: Option<&str>, name: &str) -> Result<()> {
    let mut conn = connect(connection, password, None).await?;
    sqlx::query(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
    )
    .bind(name)
    .execute(&mut conn)
    .await?;
    sqlx::raw_sql(AssertSqlSafe(format!("DROP DATABASE IF EXISTS {}", quote_ident(name)))).execute(&mut conn).await?;
    conn.close().await?;
    Ok(())
}

/// Ends the sessions a tool opened as `application_name`, so a cancelled
/// pg_dump or psql stops working on the server too.
pub async fn end_sessions(connection: &Connection, password: Option<&str>, application_name: &str) -> Result<()> {
    let mut conn = connect(connection, password, None).await?;
    sqlx::query(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = $1 AND pid <> pg_backend_pid()",
    )
    .bind(application_name)
    .execute(&mut conn)
    .await?;
    conn.close().await?;
    Ok(())
}

/// Runs a statement outside any transaction, on the connection's own database.
async fn run_on_server(connection: &Connection, password: Option<&str>, statement: &str) -> Result<()> {
    let mut conn = connect(connection, password, None).await?;
    sqlx::raw_sql(AssertSqlSafe(statement.to_string())).execute(&mut conn).await?;
    conn.close().await?;
    Ok(())
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
        _ => {
            return Err(AppError::Invalid(format!("Unexpected path: {}", path.join("/"))));
        }
    };
    Ok(objects)
}

/// Every table and view in `database` (the connection's own when `None`), for search.
pub async fn list_relations(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
) -> Result<RelationList> {
    let mut conn = connect(connection, password, database).await?;
    // The connection's own database may be unset; ask the server which one it picked.
    let database: String = sqlx::query_scalar("SELECT current_database()::text").fetch_one(&mut conn).await?;
    let rows: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT n.nspname::text, c.relname::text, c.relkind IN ('r', 'p')
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE c.relkind IN ('r', 'p', 'v', 'm')
           AND n.nspname <> 'information_schema' AND n.nspname NOT LIKE 'pg\\_%'
         ORDER BY n.nspname, c.relname",
    )
    .fetch_all(&mut conn)
    .await?;
    conn.close().await?;

    let relations = rows
        .into_iter()
        .map(|(schema, name, is_table)| Relation {
            schema,
            name,
            kind: if is_table { ObjectKind::Table } else { ObjectKind::View },
        })
        .collect();
    Ok(RelationList { database, relations })
}

/// Reads one page of a table or view, ordered by primary key when there is one.
pub async fn fetch_rows(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    filter: &RowFilter,
    offset: u64,
    limit: u32,
) -> Result<RowsPage> {
    let mut conn = connect(connection, password, Some(&table.database)).await?;
    let columns = table_columns(&mut conn, table).await?;

    // Fetch one extra row to know whether another page exists.
    let sql = format!("{} LIMIT $1 OFFSET $2", select_as_text(&columns, table, filter));
    let mut rows: Vec<Vec<Option<String>>> = sqlx::query_scalar(AssertSqlSafe(sql))
        .bind(i64::from(limit) + 1)
        .bind(offset as i64)
        .fetch_all(&mut conn)
        .await?;
    conn.close().await?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    Ok(RowsPage { columns, rows, has_more, documents: None })
}

/// Sends every row to `sink` as it arrives, so memory stays flat for any table size.
pub async fn stream_table(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    filter: &RowFilter,
    sink: &mut impl RowSink,
) -> Result<()> {
    let mut conn = with_timeout(connect(connection, password, Some(&table.database))).await?;
    let columns = with_timeout(table_columns(&mut conn, table)).await?;
    sink.columns(&columns)?;

    let sql = select_as_text(&columns, table, filter);
    let mut rows = sqlx::query_scalar::<_, Vec<Option<String>>>(AssertSqlSafe(sql)).fetch(&mut conn);
    while let Some(values) = rows.try_next().await? {
        sink.row(&values)?;
    }
    drop(rows);
    conn.close().await?;
    Ok(())
}

async fn table_columns(conn: &mut Conn, table: &TableRef) -> Result<Vec<Column>> {
    let columns: Vec<Column> = sqlx::query_as::<_, (String, String, bool, bool)>(
        "SELECT a.attname, format_type(a.atttypid, a.atttypmod), COALESCE(a.attnum = ANY((i.indkey::int2[])[0:i.indnkeyatts - 1]), false),
                (a.attgenerated <> '' OR a.attidentity = 'a')
         FROM pg_attribute a
         JOIN pg_class c ON c.oid = a.attrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         LEFT JOIN pg_index i ON i.indrelid = c.oid AND i.indisprimary
         WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped
         ORDER BY a.attnum",
    )
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_all(conn)
    .await?
    .into_iter()
    .map(|(name, data_type, primary_key, generated)| Column { name, data_type, primary_key, generated, default: None })
    .collect();

    if columns.is_empty() {
        return Err(AppError::Invalid(format!("Table {}.{} not found", table.schema, table.name)));
    }
    Ok(columns)
}

/// Every value is cast to text so any column type can be shown or exported as-is.
fn select_as_text(columns: &[Column], table: &TableRef, filter: &RowFilter) -> String {
    let values: Vec<String> = columns.iter().map(|c| format!("{}::text", quote_ident(&c.name))).collect();
    let order_by: Vec<String> = columns.iter().filter(|c| c.primary_key).map(|c| quote_ident(&c.name)).collect();
    format!(
        "SELECT ARRAY[{}] FROM {}.{}{}",
        values.join(", "),
        quote_ident(&table.schema),
        quote_ident(&table.name),
        sql_clauses(filter, &order_by),
    )
}

/// A connection kept open between runs, so transactions, `SET` and temporary
/// tables carry over from one run to the next.
pub struct Session {
    conn: Conn,
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
        let results = read_results(sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&mut self.conn), row_limit).await?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(QueryOutcome { results, elapsed_ms })
    }

    pub async fn is_alive(&mut self) -> bool {
        self.conn.ping().await.is_ok()
    }
}

/// Reads what the simple query protocol returns, one result per statement.
/// All rows are read so every statement completes, but only the first
/// `row_limit` of each result are kept.
async fn read_results(
    mut stream: BoxStream<'_, sqlx::Result<Either<PgQueryResult, PgRow>>>,
    row_limit: usize,
) -> Result<Vec<StatementResult>> {
    let mut results = Vec::new();
    let mut current = StatementResult::default();
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
    Ok(results)
}

/// The server's plan for one statement, as `EXPLAIN (FORMAT JSON)` returns it.
///
/// With `analyze` the statement really runs, so it runs on its own connection
/// inside a transaction that is always rolled back: an `UPDATE` explained this way
/// changes nothing, and the console's session and open transaction are untouched.
/// The extended protocol accepts a single statement only, so a script like
/// `SELECT 1; COMMIT; DELETE …` is rejected instead of escaping the rollback.
pub async fn explain(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    sql: &str,
    analyze: bool,
) -> Result<serde_json::Value> {
    let mut conn = with_timeout(connect(connection, password, database)).await?;
    let options = if analyze { "FORMAT JSON, ANALYZE, BUFFERS" } else { "FORMAT JSON" };
    let statement = format!("EXPLAIN ({options}) {}", sql.trim().trim_end_matches(';'));

    let mut transaction = conn.begin().await?;
    let row = sqlx::query(AssertSqlSafe(statement)).fetch_one(&mut transaction).await;
    transaction.rollback().await?;
    conn.close().await?;

    let row = row?;
    let text = row.try_get_raw(0)?.as_str().map_err(|e| AppError::Invalid(e.to_string()))?.to_string();
    Ok(serde_json::from_str(&text)?)
}

/// Asks the server to cancel what process `pid` is running. The request goes
/// over a separate connection because the session's own one is busy.
pub async fn cancel(connection: &Connection, password: Option<&str>, database: Option<&str>, pid: i32) -> Result<()> {
    let mut conn = connect(connection, password, database).await?;
    let cancel = sqlx::query("SELECT pg_cancel_backend($1)").bind(pid).execute(&mut conn);
    // Recorded with the process written in, so it can be run again as is.
    statement_log::shown_as(format!("SELECT pg_cancel_backend({pid})"), cancel).await?;
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
            generated: false,
            default: None,
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
pub(crate) mod tests {
    use super::*;
    use crate::connections::model::Engine;

    pub(crate) const PASSWORD: Option<&str> = Some("dbforge");

    pub(crate) fn local() -> Connection {
        Connection {
            id: "test".into(),
            name: "test".into(),
            engine: Engine::Postgres,
            environment: None,
            // Not `localhost`: Docker on macOS answers slowly on IPv6, which is tried first.
            host: "127.0.0.1".into(),
            port: 55432,
            database: "shop".into(),
            username: "postgres".into(),
            tls: false,
            allow_self_signed: false,
            srv: false,
            agent_access: Default::default(),
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

        let first = fetch_rows(&local(), PASSWORD, &table, &crate::drivers::RowFilter::default(), 0, 2).await.unwrap();
        let columns: Vec<_> =
            first.columns.iter().map(|c| (c.name.as_str(), c.data_type.as_str(), c.primary_key)).collect();
        assert_eq!(
            columns,
            [("id", "integer", true), ("label", "character varying(20)", false), ("paid", "numeric(10,2)", false)]
        );
        let s = |v: &str| Some(v.to_string());
        assert_eq!(first.rows, [vec![s("1"), s("a"), s("12.50")], vec![s("2"), None, s("0.00")]]);
        assert!(first.has_more);

        let last = fetch_rows(&local(), PASSWORD, &table, &crate::drivers::RowFilter::default(), 2, 2).await.unwrap();
        assert_eq!(last.rows, [vec![s("3"), s("c"), None]]);
        assert!(!last.has_more);

        let missing = TableRef { name: "nope".into(), ..table };
        assert!(fetch_rows(&local(), PASSWORD, &missing, &crate::drivers::RowFilter::default(), 0, 2).await.is_err());

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
    async fn lists_every_table_and_view_of_a_database() {
        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        sqlx::raw_sql(
            "DROP SCHEMA IF EXISTS dbforge_search CASCADE;
             CREATE SCHEMA dbforge_search;
             CREATE TABLE dbforge_search.orders (id int);
             CREATE VIEW dbforge_search.open_orders AS SELECT id FROM dbforge_search.orders;",
        )
        .execute(&mut conn)
        .await
        .unwrap();

        let list = list_relations(&local(), PASSWORD, None).await.unwrap();
        assert_eq!(list.database, "shop");
        let found: Vec<_> =
            list.relations.iter().filter(|r| r.schema == "dbforge_search").map(|r| (r.name.as_str(), r.kind)).collect();
        assert_eq!(found, [("open_orders", ObjectKind::View), ("orders", ObjectKind::Table)]);
        assert!(list.relations.iter().all(|r| !r.schema.starts_with("pg_")));

        sqlx::query("DROP SCHEMA dbforge_search CASCADE").execute(&mut conn).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn streams_every_row_of_a_table() {
        #[derive(Default)]
        struct Collect(Vec<String>, Vec<Vec<Option<String>>>);
        impl RowSink for Collect {
            fn columns(&mut self, columns: &[Column]) -> Result<()> {
                self.0 = columns.iter().map(|c| c.name.clone()).collect();
                Ok(())
            }
            fn row(&mut self, values: &[Option<String>]) -> Result<()> {
                self.1.push(values.to_vec());
                Ok(())
            }
        }

        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        sqlx::raw_sql(
            "DROP SCHEMA IF EXISTS dbforge_export CASCADE;
             CREATE SCHEMA dbforge_export;
             CREATE TABLE dbforge_export.items (id int PRIMARY KEY, label text);
             INSERT INTO dbforge_export.items SELECT g, CASE WHEN g % 2 = 0 THEN 'item ' || g END
             FROM generate_series(1500, 1, -1) g;",
        )
        .execute(&mut conn)
        .await
        .unwrap();

        let table = TableRef { database: "shop".into(), schema: "dbforge_export".into(), name: "items".into() };
        let mut sink = Collect::default();
        stream_table(&local(), PASSWORD, &table, &crate::drivers::RowFilter::default(), &mut sink).await.unwrap();

        assert_eq!(sink.0, ["id", "label"]);
        assert_eq!(sink.1.len(), 1500, "more than any page size");
        assert_eq!(sink.1[0], [Some("1".into()), None], "ordered by primary key");
        assert_eq!(sink.1[1], [Some("2".into()), Some("item 2".into())]);

        sqlx::query("DROP SCHEMA dbforge_export CASCADE").execute(&mut conn).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn explain_returns_the_plan_and_analyze_changes_nothing() {
        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        sqlx::raw_sql(
            "DROP SCHEMA IF EXISTS dbforge_explain CASCADE;
             CREATE SCHEMA dbforge_explain;
             CREATE TABLE dbforge_explain.items AS SELECT g AS id FROM generate_series(1, 10) g;",
        )
        .execute(&mut conn)
        .await
        .unwrap();

        let plan = explain(&local(), PASSWORD, None, "SELECT * FROM dbforge_explain.items;", false).await.unwrap();
        assert_eq!(plan[0]["Plan"]["Node Type"], "Seq Scan");
        assert!(plan[0].get("Execution Time").is_none(), "no ANALYZE, no execution");

        let analyzed = explain(&local(), PASSWORD, None, "DELETE FROM dbforge_explain.items", true).await.unwrap();
        assert_eq!(analyzed[0]["Plan"]["Plans"][0]["Actual Rows"], 10);
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM dbforge_explain.items").fetch_one(&mut conn).await.unwrap();
        assert_eq!(count, 10, "the analyzed DELETE was rolled back");

        let error = explain(&local(), PASSWORD, None, "SELECT 1; COMMIT; DROP SCHEMA dbforge_explain CASCADE", true)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("multiple commands"), "{error}");
        let still_there: i64 =
            sqlx::query_scalar("SELECT count(*) FROM dbforge_explain.items").fetch_one(&mut conn).await.unwrap();
        assert_eq!(still_there, 10);

        sqlx::query("DROP SCHEMA dbforge_explain CASCADE").execute(&mut conn).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn flags_columns_the_database_computes() {
        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        sqlx::raw_sql(
            "DROP SCHEMA IF EXISTS dbforge_generated CASCADE;
             CREATE SCHEMA dbforge_generated;
             CREATE TABLE dbforge_generated.items (
                 id int GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                 seq int GENERATED BY DEFAULT AS IDENTITY,
                 qty int DEFAULT 1,
                 doubled int GENERATED ALWAYS AS (qty * 2) STORED
             );",
        )
        .execute(&mut conn)
        .await
        .unwrap();

        let table = TableRef { database: "shop".into(), schema: "dbforge_generated".into(), name: "items".into() };
        let page = fetch_rows(&local(), PASSWORD, &table, &crate::drivers::RowFilter::default(), 0, 1).await.unwrap();
        let flags: Vec<_> = page.columns.iter().map(|c| (c.name.as_str(), c.generated)).collect();
        // BY DEFAULT identities and plain defaults can still be given a value.
        assert_eq!(flags, [("id", true), ("seq", false), ("qty", false), ("doubled", true)]);

        sqlx::query("DROP SCHEMA dbforge_generated CASCADE").execute(&mut conn).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn wrong_password_is_reported() {
        let error = server_version(&local(), Some("wrong")).await.unwrap_err();
        assert!(error.to_string().contains("password authentication failed"), "{error}");
    }
    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn filters_and_orders_rows_as_typed() {
        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        for statement in [
            "DROP SCHEMA IF EXISTS dbforge_filter CASCADE",
            "CREATE SCHEMA dbforge_filter",
            "CREATE TABLE dbforge_filter.items (id int PRIMARY KEY, paid numeric(10,2))",
            "INSERT INTO dbforge_filter.items VALUES (1, 12.50), (2, 0), (3, NULL)",
        ] {
            sqlx::query(statement).execute(&mut conn).await.unwrap();
        }
        let items = TableRef { database: "shop".into(), schema: "dbforge_filter".into(), name: "items".into() };
        let f = |filter: &str, order: &str| crate::drivers::RowFilter { filter: filter.into(), order: order.into() };
        let ids = |page: &RowsPage| page.rows.iter().map(|r| r[0].clone().unwrap()).collect::<Vec<_>>();

        let page = fetch_rows(&local(), PASSWORD, &items, &f("paid IS NOT NULL -- priced only", "paid DESC"), 0, 10)
            .await
            .unwrap();
        assert_eq!(ids(&page), ["1", "2"]);
        // The OR stays inside the filter: the page size still applies.
        let page = fetch_rows(&local(), PASSWORD, &items, &f("id = 1 OR id = 3", ""), 0, 1).await.unwrap();
        assert_eq!((ids(&page), page.has_more), (vec!["1".to_string()], true));
        let error = fetch_rows(&local(), PASSWORD, &items, &f("nope = 1", ""), 0, 10).await.unwrap_err();
        assert!(error.to_string().contains("nope"), "{error}");
    }
}
