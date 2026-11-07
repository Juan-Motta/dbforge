//! Microsoft SQL Server over TDS, through tiberius (sqlx has no SQL Server
//! driver). Like PostgreSQL, a server holds databases that hold schemas, and a
//! connection works inside one database at a time.
//!
//! Every value is turned into text here, the way SQL Server Management Studio
//! shows it, so results look the same in the grid, the console and exports.

mod edit;
mod explain;
mod structure;

pub use edit::{apply_changes, preview_changes};
pub use explain::explain;
pub use structure::{schema_diagram, table_structure};

use std::sync::Arc;
use std::time::Instant;

use futures_util::TryStreamExt;
use tiberius::time::chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use tiberius::{AuthMethod, ColumnData, ColumnType, Config, FromSql, Query, QueryItem, Row};
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use super::{
    Column, DbObject, ObjectKind, QueryOutcome, Relation, RelationList, RowSink, RowsPage, StatementResult, TableRef,
    with_timeout,
};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

type Client = tiberius::Client<Compat<TcpStream>>;

/// Opens a connection, to `database` when given, else the configured one, else
/// the login's default database.
async fn connect(connection: &Connection, password: Option<&str>, database: Option<&str>) -> Result<Client> {
    let mut config = Config::new();
    config.host(&connection.host);
    config.port(connection.port);
    config.application_name("DBForge");
    let database = database.unwrap_or(&connection.database);
    if !database.is_empty() {
        config.database(database);
    }
    config.authentication(AuthMethod::sql_server(&connection.username, password.unwrap_or_default()));
    // Traffic is encrypted, but the certificate isn't checked: servers usually
    // present a self-signed one. PostgreSQL and MySQL connections do the same.
    config.trust_cert();

    let tcp = TcpStream::connect(config.get_addr())
        .await
        .map_err(|e| AppError::Invalid(format!("Could not reach {}:{}: {e}", connection.host, connection.port)))?;
    tcp.set_nodelay(true).map_err(|e| AppError::Invalid(e.to_string()))?;
    Ok(Client::connect(config, tcp.compat_write()).await?)
}

/// Runs a query that returns rows and turns every value into text.
async fn text_rows(client: &mut Client, query: Query<'_>) -> Result<Vec<Vec<Option<String>>>> {
    let rows = query.query(client).await?.into_first_result().await?;
    Ok(rows.iter().map(text_values).collect())
}

/// One text column of every row.
async fn strings(client: &mut Client, query: Query<'_>) -> Result<Vec<String>> {
    Ok(text_rows(client, query)
        .await?
        .into_iter()
        .map(|row| row.into_iter().next().flatten().unwrap_or_default())
        .collect())
}

fn query<'a>(sql: impl Into<std::borrow::Cow<'a, str>>, params: &[&'a str]) -> Query<'a> {
    let mut query = Query::new(sql);
    for param in params {
        query.bind(*param);
    }
    query
}

pub async fn server_version(connection: &Connection, password: Option<&str>) -> Result<String> {
    let mut client = connect(connection, password, None).await?;
    let version = strings(&mut client, query("SELECT CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(128))", &[]))
        .await?
        .concat();
    client.close().await?;
    Ok(format!("SQL Server {version}"))
}

/// `[]` → databases, `[db]` → schemas, `[db, schema]` → tables and views.
pub async fn list_objects(connection: &Connection, password: Option<&str>, path: &[String]) -> Result<Vec<DbObject>> {
    let objects = match path {
        [] => {
            let mut client = connect(connection, password, None).await?;
            // tempdb and model (the template for new databases) hold nothing to browse.
            let names = strings(
                &mut client,
                query(
                    "SELECT name FROM sys.databases
                     WHERE name NOT IN ('tempdb', 'model') AND state_desc = 'ONLINE' AND HAS_DBACCESS(name) = 1
                     ORDER BY name",
                    &[],
                ),
            )
            .await?;
            client.close().await?;
            names.into_iter().map(|name| DbObject::new(name, ObjectKind::Database)).collect()
        }
        [database] => {
            let mut client = connect(connection, password, Some(database)).await?;
            // Ids from 16384 are the schemas of the fixed database roles (db_owner, …).
            let names = strings(
                &mut client,
                query(
                    "SELECT name FROM sys.schemas
                     WHERE schema_id < 16384 AND name NOT IN ('sys', 'INFORMATION_SCHEMA', 'guest')
                     ORDER BY name",
                    &[],
                ),
            )
            .await?;
            client.close().await?;
            names.into_iter().map(|name| DbObject::new(name, ObjectKind::Schema)).collect()
        }
        [database, schema] => {
            let mut client = connect(connection, password, Some(database)).await?;
            // The heap or clustered index of a table counts its rows.
            let rows = text_rows(
                &mut client,
                query(
                    "SELECT o.name, o.type,
                            (SELECT SUM(p.rows) FROM sys.partitions p
                             WHERE p.object_id = o.object_id AND p.index_id IN (0, 1))
                     FROM sys.objects o
                     WHERE o.schema_id = SCHEMA_ID(@P1) AND o.type IN ('U', 'V') AND o.is_ms_shipped = 0
                     ORDER BY o.name",
                    &[schema],
                ),
            )
            .await?;
            client.close().await?;
            rows.into_iter()
                .map(|row| {
                    let [name, kind, estimate] = <[Option<String>; 3]>::try_from(row).unwrap_or_default();
                    let is_view = kind.as_deref().is_some_and(|k| k.trim() == "V");
                    DbObject {
                        name: name.unwrap_or_default(),
                        kind: if is_view { ObjectKind::View } else { ObjectKind::Table },
                        row_estimate: estimate.and_then(|e| e.parse().ok()),
                    }
                })
                .collect()
        }
        _ => return Err(AppError::Invalid(format!("Unexpected path: {}", path.join("/")))),
    };
    Ok(objects)
}

/// Every table and view of `database` (the connection's own when `None`), for search.
pub async fn list_relations(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
) -> Result<RelationList> {
    let mut client = connect(connection, password, database).await?;
    let database = strings(&mut client, query("SELECT DB_NAME()", &[])).await?.concat();
    let rows = text_rows(
        &mut client,
        query(
            "SELECT SCHEMA_NAME(schema_id), name, type FROM sys.objects
             WHERE type IN ('U', 'V') AND is_ms_shipped = 0 ORDER BY SCHEMA_NAME(schema_id), name",
            &[],
        ),
    )
    .await?;
    client.close().await?;
    let relations = rows
        .into_iter()
        .map(|row| {
            let [schema, name, kind] = <[Option<String>; 3]>::try_from(row).unwrap_or_default();
            Relation {
                schema: schema.unwrap_or_default(),
                name: name.unwrap_or_default(),
                kind: if kind.as_deref().is_some_and(|k| k.trim() == "V") {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                },
            }
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
    let mut client = connect(connection, password, Some(&table.database)).await?;
    let columns = table_columns(&mut client, table).await?;
    // Fetch one extra row to know whether another page exists.
    let sql = format!("{} OFFSET @P1 ROWS FETCH NEXT @P2 ROWS ONLY", select(&columns, table));
    let mut page = Query::new(sql);
    page.bind(offset as i64);
    page.bind(i64::from(limit) + 1);
    let mut rows = text_rows(&mut client, page).await?;
    client.close().await?;

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
    let mut client = with_timeout(connect(connection, password, Some(&table.database))).await?;
    let columns = with_timeout(table_columns(&mut client, table)).await?;
    sink.columns(&columns)?;

    let mut stream = client.simple_query(select(&columns, table)).await?;
    while let Some(item) = stream.try_next().await? {
        if let QueryItem::Row(row) = item {
            sink.row(&text_values(&row))?;
        }
    }
    drop(stream);
    client.close().await?;
    Ok(())
}

/// The columns of a table or view with their full types, primary key and
/// whether the server computes them: computed, identity and rowversion columns
/// can't be given a value.
async fn table_columns(client: &mut Client, table: &TableRef) -> Result<Vec<Column>> {
    let rows = text_rows(
        client,
        query(
            "SELECT c.name, TYPE_NAME(c.user_type_id), c.max_length, c.precision, c.scale,
                    CASE WHEN EXISTS (
                        SELECT 1 FROM sys.indexes i
                        JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id
                        WHERE i.object_id = c.object_id AND i.is_primary_key = 1 AND ic.column_id = c.column_id
                    ) THEN 1 ELSE 0 END,
                    CASE WHEN c.is_computed = 1 OR c.is_identity = 1 OR c.system_type_id = 189 THEN 1 ELSE 0 END
             FROM sys.columns c
             WHERE c.object_id = OBJECT_ID(QUOTENAME(@P1) + '.' + QUOTENAME(@P2))
             ORDER BY c.column_id",
            &[&table.schema, &table.name],
        ),
    )
    .await?;
    if rows.is_empty() {
        return Err(AppError::Invalid(format!("Table {}.{} not found", table.schema, table.name)));
    }
    Ok(rows
        .into_iter()
        .map(|row| {
            let text = |i: usize| row[i].clone().unwrap_or_default();
            Column {
                name: text(0),
                data_type: type_name(&text(1), &text(2), &text(3), &text(4)),
                primary_key: text(5) == "1",
                generated: text(6) == "1",
            }
        })
        .collect())
}

/// The type as it is declared, e.g. `nvarchar(80)` or `decimal(10,2)`, from
/// sys.columns: lengths are in bytes, and -1 means `max`.
fn type_name(name: &str, max_length: &str, precision: &str, scale: &str) -> String {
    let length = || match max_length.parse::<i32>().unwrap_or(0) {
        -1 => "max".to_string(),
        bytes if name.starts_with('n') => (bytes / 2).to_string(),
        bytes => bytes.to_string(),
    };
    match name {
        "varchar" | "char" | "varbinary" | "binary" | "nvarchar" | "nchar" => format!("{name}({})", length()),
        "decimal" | "numeric" => format!("{name}({precision},{scale})"),
        "datetime2" | "time" | "datetimeoffset" => format!("{name}({scale})"),
        _ => name.to_string(),
    }
}

/// `SELECT` of every column, ordered by primary key; `OFFSET` needs an order,
/// so a table without a key gets an arbitrary one.
fn select(columns: &[Column], table: &TableRef) -> String {
    let names: Vec<String> = columns.iter().map(|c| quote_ident(&c.name)).collect();
    let key: Vec<String> = columns.iter().filter(|c| c.primary_key).map(|c| quote_ident(&c.name)).collect();
    format!(
        "SELECT {} FROM {}.{} ORDER BY {}",
        names.join(", "),
        quote_ident(&table.schema),
        quote_ident(&table.name),
        if key.is_empty() { "(SELECT NULL)".to_string() } else { key.join(", ") },
    )
}

/// A connection kept open between runs, so transactions, `SET` and temporary
/// tables carry over from one run to the next.
pub struct Session {
    client: Client,
    /// Wakes a running query to cancel it: TDS cancels on the connection that
    /// runs the query, not from another one.
    cancel: Arc<Notify>,
    /// Set once a query was cancelled: tiberius doesn't read the server's whole
    /// acknowledgement, so the next reply on this connection could belong to
    /// the cancelled request. The session is never used again.
    reset: bool,
}

pub async fn open_session(connection: &Connection, password: Option<&str>, database: Option<&str>) -> Result<Session> {
    Ok(Session { client: connect(connection, password, database).await?, cancel: Arc::default(), reset: false })
}

impl Session {
    pub fn canceller(&self) -> Arc<Notify> {
        self.cancel.clone()
    }

    /// Runs a script of batches separated by `GO` lines, as SQL Server tools do.
    /// Every result set comes back with its rows; all rows are read so the batch
    /// completes, but only `row_limit` are kept.
    ///
    /// tiberius doesn't report per-statement row counts alongside result sets,
    /// so a batch without any result set reports the count of its last statement.
    pub async fn run(&mut self, sql: &str, row_limit: usize) -> Result<QueryOutcome> {
        if self.reset {
            return Err(AppError::Invalid("This session was reset; run the query again.".into()));
        }
        let started = Instant::now();
        // Registered before the query starts, so a cancel can't slip in between.
        let cancelled = self.cancel.notified();
        tokio::pin!(cancelled);
        cancelled.as_mut().enable();

        let finished = tokio::select! {
            results = run_batches(&mut self.client, sql, row_limit) => Some(results),
            () = &mut cancelled => None,
        };
        let Some(results) = finished else {
            // Tells the server to stop. Its acknowledgement isn't fully read
            // (see `reset`), so the error is expected and the session is retired.
            let _ = self.client.cancel_query().await;
            self.reset = true;
            return Err(AppError::Invalid(
                "Query cancelled. The console's connection was reset: open transactions were rolled back and \
                 temporary tables are gone."
                    .into(),
            ));
        };
        Ok(QueryOutcome { results: results?, elapsed_ms: started.elapsed().as_millis() as u64 })
    }

    pub async fn is_alive(&mut self) -> bool {
        !self.reset && self.client.simple_query("SELECT 1").await.is_ok()
    }
}

async fn run_batches(client: &mut Client, sql: &str, row_limit: usize) -> Result<Vec<StatementResult>> {
    let mut results = Vec::new();
    for batch in batches(sql) {
        let mut batch_results = read_results(client, &batch, row_limit).await?;
        if batch_results.is_empty() {
            let count = strings(client, query("SELECT @@ROWCOUNT", &[])).await?.concat();
            batch_results.push(StatementResult { rows_affected: count.parse().unwrap_or(0), ..Default::default() });
        }
        results.append(&mut batch_results);
    }
    Ok(results)
}

/// Splits a script at lines holding only `GO`, which tools understand and the
/// server doesn't; blank batches are skipped.
fn batches(sql: &str) -> Vec<String> {
    let mut batches = vec![String::new()];
    for line in sql.lines() {
        if line.trim().eq_ignore_ascii_case("go") {
            batches.push(String::new());
        } else {
            let current = batches.last_mut().expect("never empty");
            current.push_str(line);
            current.push('\n');
        }
    }
    batches.retain(|batch| !batch.trim().is_empty());
    batches
}

async fn read_results(client: &mut Client, sql: &str, row_limit: usize) -> Result<Vec<StatementResult>> {
    let mut results: Vec<StatementResult> = Vec::new();
    let mut stream = client.simple_query(sql).await?;
    while let Some(item) = stream.try_next().await? {
        match item {
            // Starts each result set, even one without rows.
            QueryItem::Metadata(metadata) => results.push(StatementResult {
                columns: metadata
                    .columns()
                    .iter()
                    .map(|c| Column {
                        name: c.name().to_string(),
                        data_type: column_type(c.column_type()).into(),
                        primary_key: false,
                        generated: false,
                    })
                    .collect(),
                ..Default::default()
            }),
            QueryItem::Row(row) => {
                let Some(current) = results.last_mut() else { continue };
                current.rows_affected += 1;
                if current.rows.len() < row_limit {
                    current.rows.push(text_values(&row));
                } else {
                    current.truncated = true;
                }
            }
        }
    }
    Ok(results)
}

/// Wakes the session's running query so it cancels itself. Harmless when idle:
/// nothing is waiting, and nothing is kept for the next run.
pub fn cancel(canceller: &Notify) {
    canceller.notify_waiters();
}

/// The type name of a result column, as far as the protocol tells it.
fn column_type(column_type: ColumnType) -> &'static str {
    match column_type {
        ColumnType::Null => "null",
        ColumnType::Bit | ColumnType::Bitn => "bit",
        ColumnType::Int1 => "tinyint",
        ColumnType::Int2 => "smallint",
        ColumnType::Int4 | ColumnType::Intn => "int",
        ColumnType::Int8 => "bigint",
        ColumnType::Float4 => "real",
        ColumnType::Float8 | ColumnType::Floatn => "float",
        ColumnType::Money => "money",
        ColumnType::Money4 => "smallmoney",
        ColumnType::Datetime | ColumnType::Datetimen => "datetime",
        ColumnType::Datetime4 => "smalldatetime",
        ColumnType::Guid => "uniqueidentifier",
        ColumnType::Decimaln => "decimal",
        ColumnType::Numericn => "numeric",
        ColumnType::Daten => "date",
        ColumnType::Timen => "time",
        ColumnType::Datetime2 => "datetime2",
        ColumnType::DatetimeOffsetn => "datetimeoffset",
        ColumnType::BigVarBin => "varbinary",
        ColumnType::BigBinary => "binary",
        ColumnType::BigVarChar => "varchar",
        ColumnType::BigChar => "char",
        ColumnType::NVarchar => "nvarchar",
        ColumnType::NChar => "nchar",
        ColumnType::Xml => "xml",
        ColumnType::Udt => "udt",
        ColumnType::Text => "text",
        ColumnType::Image => "image",
        ColumnType::NText => "ntext",
        ColumnType::SSVariant => "sql_variant",
    }
}

fn text_values(row: &Row) -> Vec<Option<String>> {
    row.cells().map(|(_, data)| text(data)).collect()
}

/// A value as text: numbers and dates in the forms SQL Server reads back,
/// binary as `0x…` hex, and `bit` as 1 or 0.
fn text(data: &ColumnData<'static>) -> Option<String> {
    let date = |format: &str| -> Option<String> {
        let value = NaiveDateTime::from_sql(data).ok()??;
        Some(value.format(format).to_string())
    };
    match data {
        ColumnData::U8(v) => v.map(|v| v.to_string()),
        ColumnData::I16(v) => v.map(|v| v.to_string()),
        ColumnData::I32(v) => v.map(|v| v.to_string()),
        ColumnData::I64(v) => v.map(|v| v.to_string()),
        ColumnData::F32(v) => v.map(|v| v.to_string()),
        ColumnData::F64(v) => v.map(|v| v.to_string()),
        ColumnData::Bit(v) => v.map(|v| if v { "1" } else { "0" }.to_string()),
        ColumnData::String(v) => v.as_ref().map(|v| v.to_string()),
        ColumnData::Guid(v) => v.map(|v| v.to_string().to_uppercase()),
        ColumnData::Binary(v) => {
            v.as_ref().map(|bytes| format!("0x{}", bytes.iter().map(|b| format!("{b:02X}")).collect::<String>()))
        }
        ColumnData::Numeric(v) => v.map(|v| decimal(v.value(), v.scale())),
        ColumnData::Xml(v) => v.as_ref().map(|v| v.to_string()),
        // datetime keeps 1/300 s; three decimals is how it's written and read back.
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) => date("%Y-%m-%d %H:%M:%S%.3f"),
        ColumnData::DateTime2(_) => date("%Y-%m-%d %H:%M:%S%.f"),
        ColumnData::Date(_) => NaiveDate::from_sql(data).ok()?.map(|v| v.format("%Y-%m-%d").to_string()),
        ColumnData::Time(_) => NaiveTime::from_sql(data).ok()?.map(|v| v.format("%H:%M:%S%.f").to_string()),
        ColumnData::DateTimeOffset(_) => {
            DateTime::<FixedOffset>::from_sql(data).ok()?.map(|v| v.format("%Y-%m-%d %H:%M:%S%.f %:z").to_string())
        }
    }
}

/// `value` with `scale` digits after the point, e.g. (-5, 2) → `-0.05`.
fn decimal(value: i128, scale: u8) -> String {
    let digits = value.unsigned_abs().to_string();
    let scale = usize::from(scale);
    let sign = if value < 0 { "-" } else { "" };
    if scale == 0 {
        return format!("{sign}{digits}");
    }
    let digits = format!("{digits:0>width$}", width = scale + 1);
    let (whole, fraction) = digits.split_at(digits.len() - scale);
    format!("{sign}{whole}.{fraction}")
}

/// Quotes an identifier so any table or column name is safe inside SQL.
fn quote_ident(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

/// Runs against a real server. Start one and run `cargo test -- --ignored`:
/// `docker run -d --name dbforge-test-mssql -e ACCEPT_EULA=1 -e 'MSSQL_SA_PASSWORD=DbForge#2026' -p 51433:1433 mcr.microsoft.com/azure-sql-edge`
/// (Azure SQL Edge runs natively on Apple silicon; SQL Server images don't start
/// under Rosetta.) Each test creates its own database.
#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::connections::model::Engine;

    pub const PASSWORD: Option<&str> = Some("DbForge#2026");

    pub fn local() -> Connection {
        Connection {
            id: "test".into(),
            name: "test".into(),
            engine: Engine::Sqlserver,
            environment: None,
            // Not `localhost`: Docker on macOS answers slowly on IPv6, which is tried first.
            host: "127.0.0.1".into(),
            port: 51433,
            database: String::new(),
            username: "sa".into(),
            tls: false,
            allow_self_signed: false,
            srv: false,
        }
    }

    /// A fresh database named `name` holding `script`, and a connection to it.
    pub async fn database(name: &str, script: &str) -> Connection {
        let mut client = connect(&local(), PASSWORD, Some("master")).await.unwrap();
        client
            .simple_query(format!(
                "IF DB_ID('{name}') IS NOT NULL BEGIN
                     ALTER DATABASE [{name}] SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE [{name}]
                 END;
                 CREATE DATABASE [{name}]"
            ))
            .await
            .unwrap()
            .into_results()
            .await
            .unwrap();
        client.close().await.unwrap();
        let connection = Connection { database: name.into(), ..local() };
        let mut client = connect(&connection, PASSWORD, None).await.unwrap();
        for batch in batches(script) {
            client.simple_query(batch).await.unwrap().into_results().await.unwrap();
        }
        client.close().await.unwrap();
        connection
    }

    pub fn table(database: &str, name: &str) -> TableRef {
        TableRef { database: database.into(), schema: "dbo".into(), name: name.into() }
    }

    const SHOP: &str = "
        CREATE SCHEMA sales;
        GO
        CREATE TABLE sales.items (
            id int IDENTITY PRIMARY KEY,
            name nvarchar(40) NOT NULL,
            price decimal(10,2),
            seen datetime2(3),
            flag bit,
            blob varbinary(8),
            ref uniqueidentifier,
            doubled AS price * 2
        );
        GO
        CREATE VIEW sales.cheap AS SELECT id, name FROM sales.items WHERE price < 10;";

    fn names(objects: &[DbObject]) -> Vec<&str> {
        objects.iter().map(|o| o.name.as_str()).collect()
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn lists_databases_schemas_and_tables() {
        let connection = database("dbforge_list", SHOP).await;
        assert!(server_version(&connection, PASSWORD).await.unwrap().starts_with("SQL Server "));

        let databases = list_objects(&connection, PASSWORD, &[]).await.unwrap();
        assert!(names(&databases).contains(&"dbforge_list"));
        assert!(!names(&databases).contains(&"tempdb"));

        let schemas = list_objects(&connection, PASSWORD, &["dbforge_list".into()]).await.unwrap();
        assert_eq!(names(&schemas), ["dbo", "sales"]);

        let objects = list_objects(&connection, PASSWORD, &["dbforge_list".into(), "sales".into()]).await.unwrap();
        assert_eq!(
            objects,
            [
                DbObject { name: "cheap".into(), kind: ObjectKind::View, row_estimate: None },
                DbObject { name: "items".into(), kind: ObjectKind::Table, row_estimate: Some(0) },
            ]
        );

        let relations = list_relations(&connection, PASSWORD, None).await.unwrap();
        assert_eq!(relations.database, "dbforge_list");
        let found: Vec<_> = relations.relations.iter().map(|r| (r.schema.as_str(), r.name.as_str())).collect();
        assert_eq!(found, [("sales", "cheap"), ("sales", "items")]);
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn reads_pages_of_rows_as_text() {
        let connection = database(
            "dbforge_rows",
            &format!(
                "{SHOP}
                 GO
                 INSERT INTO sales.items (name, price, seen, flag, blob, ref) VALUES
                     (N'café', 9.50, '2024-01-02 10:00:00.125', 1, 0x00FF, 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'),
                     (N'two', -0.05, NULL, 0, NULL, NULL),
                     (N'three', 100, NULL, NULL, NULL, NULL);"
            ),
        )
        .await;
        let items = TableRef { schema: "sales".into(), ..table("dbforge_rows", "items") };

        let page = fetch_rows(&connection, PASSWORD, &items, 0, 2).await.unwrap();
        let columns: Vec<_> =
            page.columns.iter().map(|c| (c.name.as_str(), c.data_type.as_str(), c.primary_key, c.generated)).collect();
        assert_eq!(
            columns,
            [
                ("id", "int", true, true),
                ("name", "nvarchar(40)", false, false),
                ("price", "decimal(10,2)", false, false),
                ("seen", "datetime2(3)", false, false),
                ("flag", "bit", false, false),
                ("blob", "varbinary(8)", false, false),
                ("ref", "uniqueidentifier", false, false),
                ("doubled", "decimal(12,2)", false, true),
            ]
        );
        let text = |v: &[&str]| v.iter().map(|s| (!s.is_empty()).then(|| s.to_string())).collect::<Vec<_>>();
        assert_eq!(
            page.rows,
            [
                text(&[
                    "1",
                    "café",
                    "9.50",
                    "2024-01-02 10:00:00.125",
                    "1",
                    "0x00FF",
                    "A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11",
                    "19.00"
                ]),
                text(&["2", "two", "-0.05", "", "0", "", "", "-0.10"]),
            ]
        );
        assert!(page.has_more);

        let last = fetch_rows(&connection, PASSWORD, &items, 2, 2).await.unwrap();
        assert_eq!((last.rows.len(), last.has_more), (1, false));

        let view = TableRef { name: "cheap".into(), ..items.clone() };
        assert_eq!(fetch_rows(&connection, PASSWORD, &view, 0, 10).await.unwrap().rows.len(), 2);
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn sessions_keep_state_and_report_every_result_set() {
        let connection = database("dbforge_session", "CREATE TABLE t (id int PRIMARY KEY)").await;
        let mut session = open_session(&connection, PASSWORD, None).await.unwrap();

        let outcome = session.run("INSERT INTO t VALUES (1), (2), (3)", 100).await.unwrap();
        assert_eq!(outcome.results.len(), 1);
        assert_eq!((outcome.results[0].columns.len(), outcome.results[0].rows_affected), (0, 3));

        // A temporary table lives as long as the session.
        session.run("CREATE TABLE #seen (id int); INSERT INTO #seen VALUES (7)", 100).await.unwrap();
        let outcome = session.run("SELECT id FROM t ORDER BY id; SELECT id FROM #seen WHERE id = 0", 2).await.unwrap();
        let [first, second] = outcome.results.as_slice() else { panic!("{:?}", outcome.results) };
        assert_eq!(first.rows, [[Some("1".to_string())], [Some("2".to_string())]]);
        assert!(first.truncated);
        assert_eq!(first.rows_affected, 3);
        assert_eq!((second.columns[0].name.as_str(), second.rows.len()), ("id", 0));

        let error = session.run("SELECT * FROM missing", 10).await.unwrap_err();
        assert_eq!(error.to_string(), "Invalid object name 'missing'.");
        assert!(session.is_alive().await);
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn cancel_stops_the_running_query_and_retires_the_session() {
        let connection = database("dbforge_cancel", "SELECT 1").await;
        let mut session = open_session(&connection, PASSWORD, None).await.unwrap();
        let canceller = session.canceller();
        // Harmless while idle, and not remembered for the next run.
        cancel(&canceller);

        let started = Instant::now();
        let stop = async {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            cancel(&canceller);
        };
        let (outcome, ()) = tokio::join!(session.run("WAITFOR DELAY '00:00:20'", 10), stop);
        assert!(outcome.unwrap_err().to_string().starts_with("Query cancelled."));
        assert!(started.elapsed().as_secs() < 5);
        // Retired, so the console opens a new session for the next run.
        assert!(!session.is_alive().await);
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn runs_scripts_split_by_go() {
        let connection = database("dbforge_go", "SELECT 1").await;
        let mut session = open_session(&connection, PASSWORD, None).await.unwrap();
        let script = "CREATE TABLE t (id int)\ngo\nCREATE VIEW v AS SELECT id FROM t\n  GO  \nINSERT INTO t VALUES (1), (2)\nGO\nSELECT * FROM v";
        let outcome = session.run(script, 10).await.unwrap();
        let summary: Vec<_> = outcome.results.iter().map(|r| (r.columns.len(), r.rows_affected)).collect();
        assert_eq!(summary, [(0, 0), (0, 0), (0, 2), (1, 2)]);
    }

    #[test]
    fn go_lines_split_batches() {
        assert_eq!(batches("SELECT 1\nGO\n\ngo\nSELECT 'GO'\n"), ["SELECT 1\n", "SELECT 'GO'\n"]);
        assert_eq!(batches("SELECT 1 -- GO"), ["SELECT 1 -- GO\n"]);
    }

    #[test]
    fn decimals_keep_their_scale() {
        assert_eq!(decimal(950, 2), "9.50");
        assert_eq!(decimal(-5, 2), "-0.05");
        assert_eq!(decimal(42, 0), "42");
        assert_eq!(decimal(0, 3), "0.000");
    }

    #[test]
    fn declared_types_are_rebuilt() {
        assert_eq!(type_name("nvarchar", "80", "0", "0"), "nvarchar(40)");
        assert_eq!(type_name("varbinary", "-1", "0", "0"), "varbinary(max)");
        assert_eq!(type_name("decimal", "9", "10", "2"), "decimal(10,2)");
        assert_eq!(type_name("datetime2", "8", "27", "7"), "datetime2(7)");
        assert_eq!(type_name("int", "4", "10", "0"), "int");
    }
}
