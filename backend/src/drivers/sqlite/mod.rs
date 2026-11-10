//! SQLite. A connection is a database file, given in `Connection::database`;
//! host, port and user don't apply. A file holds tables directly, so the
//! explorer lists them under the connection, and a table's `TableRef` uses
//! `main`, SQLite's name for the file's own schema, as database and schema.

mod edit;
mod read_only;
mod structure;

pub use edit::{apply_changes, preview_changes};
pub use read_only::run_read_only;
pub use structure::{schema_diagram, table_structure};

use std::collections::BTreeMap;
use std::time::Instant;

use either::Either;
use futures_util::TryStreamExt;
use futures_util::stream::BoxStream;
use serde_json::{Value, json};
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection, SqliteQueryResult, SqliteRow};
use sqlx::{AssertSqlSafe, Connection as _, Row};

use super::logged::Logged;
use super::row_filter::{RowFilter, sql_clauses};
use super::{
    Column, DbObject, ObjectKind, QueryOutcome, Relation, RelationList, RowSink, RowsPage, StatementResult, TableRef,
    with_timeout,
};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// SQLite's name for the schema of the opened file.
pub const MAIN: &str = "main";

/// A SQLite connection whose statements are recorded in the statement log.
pub(super) type Conn = Logged<SqliteConnection>;

/// Opens the file, which must already exist: a typo must not create an empty database.
async fn connect(connection: &Connection) -> Result<Conn> {
    connect_with(connection, |options| options).await
}

/// [`connect`], with `adjust` applied to the options first, e.g. to open the file read-only.
async fn connect_with(
    connection: &Connection,
    adjust: impl FnOnce(SqliteConnectOptions) -> SqliteConnectOptions,
) -> Result<Conn> {
    if connection.database.is_empty() {
        return Err(AppError::Invalid("Choose the SQLite database file.".into()));
    }
    let options = adjust(SqliteConnectOptions::new().filename(&connection.database).create_if_missing(false));
    let conn = SqliteConnection::connect_with(&options).await.map_err(|error| match error {
        sqlx::Error::Database(_) => AppError::Invalid(format!("Could not open {}: {error}", connection.database)),
        other => AppError::from(other),
    })?;
    Ok(Logged::new(conn, Some(MAIN)))
}

pub async fn server_version(connection: &Connection) -> Result<String> {
    let mut conn = connect(connection).await?;
    let version: String = sqlx::query_scalar("SELECT sqlite_version()").fetch_one(&mut conn).await?;
    conn.close().await?;
    Ok(format!("SQLite {version}"))
}

/// `[]` → the file's tables and views.
pub async fn list_objects(connection: &Connection, path: &[String]) -> Result<Vec<DbObject>> {
    if !path.is_empty() {
        return Err(AppError::Invalid(format!("Unexpected path: {}", path.join("/"))));
    }
    Ok(relations(connection)
        .await?
        .into_iter()
        .map(|(name, kind)| DbObject { name, kind, row_estimate: None })
        .collect())
}

pub async fn list_relations(connection: &Connection) -> Result<RelationList> {
    let relations = relations(connection)
        .await?
        .into_iter()
        .map(|(name, kind)| Relation { schema: MAIN.into(), name, kind })
        .collect();
    Ok(RelationList { database: MAIN.into(), relations })
}

/// Tables and views, leaving out SQLite's own (`sqlite_…`).
async fn relations(connection: &Connection) -> Result<Vec<(String, ObjectKind)>> {
    let mut conn = connect(connection).await?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, type FROM sqlite_schema
         WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name",
    )
    .fetch_all(&mut conn)
    .await?;
    conn.close().await?;
    Ok(rows
        .into_iter()
        .map(|(name, kind)| (name, if kind == "view" { ObjectKind::View } else { ObjectKind::Table }))
        .collect())
}

pub async fn fetch_rows(
    connection: &Connection,
    table: &TableRef,
    filter: &RowFilter,
    offset: u64,
    limit: u32,
) -> Result<RowsPage> {
    let mut conn = connect(connection).await?;
    let columns = table_columns(&mut conn, &table.name).await?;
    let sql = format!("{} LIMIT ? OFFSET ?", select_all(&columns, &table.name, filter));
    let mut query = sqlx::query(AssertSqlSafe(sql)).bind(i64::from(limit) + 1).bind(offset as i64).fetch(&mut conn);
    let mut rows = Vec::new();
    while let Some(row) = query.try_next().await? {
        rows.push(text_values(&row));
    }
    drop(query);
    conn.close().await?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    Ok(RowsPage { columns, rows, has_more, documents: None })
}

pub async fn stream_table(
    connection: &Connection,
    table: &TableRef,
    filter: &RowFilter,
    sink: &mut impl RowSink,
) -> Result<()> {
    let mut conn = with_timeout(connect(connection)).await?;
    let columns = with_timeout(table_columns(&mut conn, &table.name)).await?;
    sink.columns(&columns)?;
    let sql = select_all(&columns, &table.name, filter);
    let mut rows = sqlx::query(AssertSqlSafe(sql)).fetch(&mut conn);
    while let Some(row) = rows.try_next().await? {
        sink.row(&text_values(&row))?;
    }
    drop(rows);
    conn.close().await?;
    Ok(())
}

/// Columns from `table_xinfo`, which also lists generated columns. Its `hidden`
/// is 2 or 3 for those, and 1 for virtual-table internals, which are left out.
/// `dflt_value` is the DEFAULT as SQL text, which edits write out to restore it.
async fn table_columns(conn: &mut Conn, table: &str) -> Result<Vec<Column>> {
    let rows: Vec<(String, String, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT name, type, pk, hidden, dflt_value FROM pragma_table_xinfo(?) WHERE hidden <> 1 ORDER BY cid",
    )
    .bind(table)
    .fetch_all(conn)
    .await?;
    if rows.is_empty() {
        return Err(AppError::Invalid(format!("Table {table} not found")));
    }
    Ok(rows
        .into_iter()
        .map(|(name, data_type, pk, hidden, default)| Column {
            name,
            // Columns may have no declared type at all.
            data_type: if data_type.is_empty() { "any".into() } else { data_type.to_lowercase() },
            primary_key: pk > 0,
            generated: hidden >= 2,
            default,
        })
        .collect())
}

fn select_all(columns: &[Column], table: &str, filter: &RowFilter) -> String {
    let names: Vec<String> = columns.iter().map(|c| quote_ident(&c.name)).collect();
    let order_by: Vec<String> = columns.iter().filter(|c| c.primary_key).map(|c| quote_ident(&c.name)).collect();
    format!("SELECT {} FROM {}{}", names.join(", "), quote_ident(table), sql_clauses(filter, &order_by))
}

/// A connection kept open between runs, so transactions and temporary tables
/// carry over. SQLite has no way to stop a query from another connection, so
/// sessions can't be cancelled.
pub struct Session {
    conn: Conn,
}

pub async fn open_session(connection: &Connection) -> Result<Session> {
    Ok(Session { conn: connect(connection).await? })
}

impl Session {
    /// Runs every statement of a script.
    pub async fn run(&mut self, sql: &str, row_limit: usize) -> Result<QueryOutcome> {
        let started = Instant::now();
        let results = read_results(sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&mut self.conn), row_limit).await?;
        Ok(QueryOutcome { results, elapsed_ms: started.elapsed().as_millis() as u64 })
    }

    pub async fn is_alive(&mut self) -> bool {
        self.conn.ping().await.is_ok()
    }
}

/// `EXPLAIN QUERY PLAN` as a tree: SQLite reports how it reads each table, with
/// no costs or row counts, and can't measure a run (no Analyze).
pub async fn explain(connection: &Connection, sql: &str, analyze: bool) -> Result<Value> {
    if analyze {
        return Err(AppError::Invalid(
            "Analyze isn't available for SQLite. Turn it off to see how SQLite reads the tables.".into(),
        ));
    }
    let statement = sql.trim().trim_end_matches(';');
    // SQLite runs every statement in a string, so a script could commit and go on
    // to change data; only one statement is explained.
    if has_several_statements(statement) {
        return Err(AppError::Invalid("Explain one statement at a time.".into()));
    }
    let mut conn = with_timeout(connect(connection)).await?;
    let mut transaction = conn.begin().await?;
    // Columns: id, parent, notused, detail.
    let rows: std::result::Result<Vec<(i64, i64, i64, String)>, _> =
        sqlx::query_as(AssertSqlSafe(format!("EXPLAIN QUERY PLAN {statement}"))).fetch_all(&mut transaction).await;
    transaction.rollback().await?;
    conn.close().await?;
    let rows: Vec<(i64, i64, String)> = rows?.into_iter().map(|(id, parent, _, detail)| (id, parent, detail)).collect();
    Ok(query_plan_tree(&rows))
}

/// Turns `(id, parent, detail)` rows into PostgreSQL's plan shape; parent 0 is the top.
fn query_plan_tree(rows: &[(i64, i64, String)]) -> Value {
    let mut children: BTreeMap<i64, Vec<&(i64, i64, String)>> = BTreeMap::new();
    for row in rows {
        children.entry(row.1).or_default().push(row);
    }
    fn build(id: i64, detail: &str, children: &BTreeMap<i64, Vec<&(i64, i64, String)>>) -> Value {
        // "SCAN orders" or "SEARCH orders USING INDEX ix (a=?)": the verb is the node type.
        let (node_type, rest) = match detail.split_once(' ') {
            Some((verb @ ("SCAN" | "SEARCH"), rest)) => (verb, Some(rest)),
            _ => (detail, None),
        };
        let mut node = json!({ "Node Type": node_type });
        if let Some(rest) = rest {
            node["Detail"] = json!(rest);
        }
        let plans: Vec<Value> =
            children.get(&id).into_iter().flatten().map(|child| build(child.0, &child.2, children)).collect();
        if !plans.is_empty() {
            node["Plans"] = json!(plans);
        }
        node
    }
    let roots: Vec<Value> = children.get(&0).into_iter().flatten().map(|row| build(row.0, &row.2, &children)).collect();
    let root = match <[Value; 1]>::try_from(roots) {
        Ok([only]) => only,
        Err(roots) => json!({ "Node Type": "Query", "Plans": roots }),
    };
    json!([{ "Plan": root }])
}

/// Whether `sql` holds a `;` outside strings, quoted names and comments.
fn has_several_statements(sql: &str) -> bool {
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ';' => return chars.any(|rest| !rest.is_whitespace()),
            '\'' | '"' | '`' | '[' => {
                let close = if c == '[' { ']' } else { c };
                chars.by_ref().find(|&next| next == close);
            }
            '-' if chars.peek() == Some(&'-') => {
                chars.by_ref().find(|&next| next == '\n');
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = ' ';
                for next in chars.by_ref() {
                    if previous == '*' && next == '/' {
                        break;
                    }
                    previous = next;
                }
            }
            _ => {}
        }
    }
    false
}

/// Values as text. SQLite turns numbers into their text form when asked for
/// bytes; binary data that isn't UTF-8 is shown lossily.
fn text_values(row: &SqliteRow) -> Vec<Option<String>> {
    (0..row.len())
        .map(|i| {
            row.try_get_unchecked::<Option<Vec<u8>>, _>(i)
                .ok()
                .flatten()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        })
        .collect()
}

/// What a script returns, with values read as this engine writes them as text.
async fn read_results(
    stream: BoxStream<'_, sqlx::Result<Either<SqliteQueryResult, SqliteRow>>>,
    row_limit: usize,
) -> Result<Vec<StatementResult>> {
    super::sqlx_results::read_results::<sqlx::Sqlite>(stream, row_limit, |row| Ok(text_values(row))).await
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::model::Engine;

    /// A database file in a fresh temporary directory, created with `sql`.
    pub(super) async fn database(sql: &str) -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let options = SqliteConnectOptions::new().filename(&path).create_if_missing(true);
        let mut conn = SqliteConnection::connect_with(&options).await.unwrap();
        sqlx::raw_sql(AssertSqlSafe(sql)).execute(&mut conn).await.unwrap();
        conn.close().await.unwrap();
        let connection = Connection {
            id: "test-sqlite".into(),
            name: "test".into(),
            engine: Engine::Sqlite,
            environment: None,
            host: String::new(),
            port: 0,
            database: path.to_string_lossy().into_owned(),
            username: String::new(),
            tls: false,
            allow_self_signed: false,
            srv: false,
            agent_access: Default::default(),
        };
        (dir, connection)
    }

    pub(super) fn table(name: &str) -> TableRef {
        TableRef { database: MAIN.into(), schema: MAIN.into(), name: name.into() }
    }

    const SHOP: &str = "
        CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
        CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers (id), total REAL,
                             doubled REAL GENERATED ALWAYS AS (total * 2) VIRTUAL, note);
        CREATE INDEX ix_orders_customer ON orders (customer_id);
        CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 100;
        INSERT INTO customers VALUES (1, 'Ann'), (2, 'Bo');
        INSERT INTO orders (id, customer_id, total, note) VALUES (3, 1, 150.5, x'00ff'), (1, 2, 20, NULL), (2, 1, 5, 'hi');";

    #[tokio::test]
    async fn opens_existing_files_only() {
        let (_dir, connection) = database(SHOP).await;
        assert!(server_version(&connection).await.unwrap().starts_with("SQLite 3."));

        let missing = Connection { database: format!("{}.missing", connection.database), ..connection };
        let error = server_version(&missing).await.unwrap_err();
        assert!(error.to_string().contains("Could not open"), "{error}");
        assert!(!std::path::Path::new(&missing.database).exists(), "no empty file was created");
    }

    #[tokio::test]
    async fn lists_tables_and_views_but_not_sqlite_internals() {
        let (_dir, connection) = database(&format!("{SHOP}; CREATE TABLE t (a); DROP TABLE t; ANALYZE;")).await;
        let objects = list_objects(&connection, &[]).await.unwrap();
        let names: Vec<_> = objects.iter().map(|o| (o.name.as_str(), o.kind)).collect();
        assert_eq!(
            names,
            [("big_orders", ObjectKind::View), ("customers", ObjectKind::Table), ("orders", ObjectKind::Table)]
        );
        assert_eq!(list_relations(&connection).await.unwrap().relations[0].schema, "main");
    }

    #[tokio::test]
    async fn pages_rows_as_text_in_primary_key_order() {
        let (_dir, connection) = database(SHOP).await;
        let page =
            fetch_rows(&connection, &table("orders"), &crate::drivers::RowFilter::default(), 0, 2).await.unwrap();
        let columns: Vec<_> =
            page.columns.iter().map(|c| (c.name.as_str(), c.data_type.as_str(), c.generated)).collect();
        assert_eq!(
            columns,
            [
                ("id", "integer", false),
                ("customer_id", "integer", false),
                ("total", "real", false),
                ("doubled", "real", true),
                ("note", "any", false)
            ]
        );
        assert_eq!(page.rows[0], [Some("1".into()), Some("2".into()), Some("20.0".into()), Some("40.0".into()), None]);
        assert_eq!(page.rows[1][4].as_deref(), Some("hi"));
        assert!(page.has_more);
    }

    #[tokio::test]
    async fn sessions_run_scripts_and_keep_state() {
        let (_dir, connection) = database(SHOP).await;
        let mut session = open_session(&connection).await.unwrap();
        let outcome = session
            .run("CREATE TEMP TABLE t (a); INSERT INTO t VALUES (1), (2); SELECT count(*) FROM t; SELECT * FROM customers", 1)
            .await
            .unwrap();
        let [_, insert, count, customers] = outcome.results.as_slice() else { panic!("{:?}", outcome.results) };
        assert_eq!(insert.rows_affected, 2);
        assert_eq!(count.rows, [vec![Some("2".into())]]);
        assert!(customers.truncated);
        assert_eq!(customers.rows_affected, 2);
        let again = session.run("SELECT count(*) FROM t", 10).await.unwrap();
        assert_eq!(again.results[0].rows, [vec![Some("2".into())]]);
    }

    #[tokio::test]
    async fn explains_as_a_tree_and_refuses_scripts() {
        let (_dir, connection) = database(SHOP).await;
        let plan = explain(
            &connection,
            "SELECT * FROM orders o JOIN customers c ON c.id = o.customer_id WHERE o.customer_id = 1",
            false,
        )
        .await
        .unwrap();
        let text = plan.to_string();
        assert!(text.contains("\"SEARCH\"") || text.contains("\"SCAN\""), "{plan}");
        assert!(text.contains("ix_orders_customer") || text.contains("PRIMARY KEY"), "{plan}");

        let error = explain(&connection, "SELECT 1; DROP TABLE orders", false).await.unwrap_err();
        assert!(error.to_string().contains("one statement"), "{error}");
        let error = explain(&connection, "SELECT 1", true).await.unwrap_err();
        assert!(error.to_string().contains("Analyze isn't available"), "{error}");
        assert_eq!(
            fetch_rows(&connection, &table("orders"), &crate::drivers::RowFilter::default(), 0, 10)
                .await
                .unwrap()
                .rows
                .len(),
            3
        );
    }

    #[test]
    fn finds_statement_separators_outside_quotes_and_comments() {
        assert!(!has_several_statements("SELECT ';' AS a, \"x;y\", [a;b] -- c;\n FROM t /* ; */"));
        assert!(!has_several_statements("SELECT 1;  "));
        assert!(has_several_statements("SELECT 1; DROP TABLE t"));
        assert!(has_several_statements("SELECT 'a'; COMMIT"));
    }

    #[test]
    fn builds_the_tree_from_parent_ids() {
        let rows = vec![
            (2, 0, "SCAN o".to_string()),
            (5, 0, "SEARCH c USING INTEGER PRIMARY KEY (rowid=?)".to_string()),
            (9, 0, "USE TEMP B-TREE FOR ORDER BY".to_string()),
        ];
        let plan = query_plan_tree(&rows);
        let root = &plan[0]["Plan"];
        assert_eq!(root["Node Type"], "Query");
        assert_eq!(root["Plans"][1]["Node Type"], "SEARCH");
        assert_eq!(root["Plans"][1]["Detail"], "c USING INTEGER PRIMARY KEY (rowid=?)");
        assert_eq!(root["Plans"][2]["Node Type"], "USE TEMP B-TREE FOR ORDER BY");
    }
    #[tokio::test]
    async fn filters_and_orders_rows_and_exports_only_those() {
        let (_dir, connection) = database(SHOP).await;
        let f = |filter: &str, order: &str| crate::drivers::RowFilter { filter: filter.into(), order: order.into() };
        let ids = |page: &RowsPage| page.rows.iter().map(|r| r[0].clone().unwrap()).collect::<Vec<_>>();

        let page =
            fetch_rows(&connection, &table("orders"), &f("total > 10 -- big ones", "total DESC"), 0, 10).await.unwrap();
        assert_eq!(ids(&page), ["3", "1"]);
        let page = fetch_rows(&connection, &table("orders"), &f("id = 1 OR id = 3", ""), 0, 1).await.unwrap();
        assert_eq!((ids(&page), page.has_more), (vec!["1".to_string()], true));
        let error = fetch_rows(&connection, &table("orders"), &f("nope = 1", ""), 0, 10).await.unwrap_err();
        assert!(error.to_string().contains("nope"), "{error}");

        struct Ids(Vec<String>);
        impl RowSink for Ids {
            fn columns(&mut self, _: &[Column]) -> Result<()> {
                Ok(())
            }
            fn row(&mut self, values: &[Option<String>]) -> Result<()> {
                self.0.push(values[0].clone().unwrap());
                Ok(())
            }
        }
        let mut sink = Ids(Vec::new());
        stream_table(&connection, &table("orders"), &f("customer_id = 1", "id"), &mut sink).await.unwrap();
        assert_eq!(sink.0, ["2", "3"]);
    }
}
