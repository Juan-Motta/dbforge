//! Imports the rows of a CSV, TSV or JSON file into a table. The file is
//! read on its own thread a batch at a time, while the batches before are
//! inserted, so a file of any size takes little memory. SQL engines insert
//! everything in one transaction; document databases a batch at a time.

mod reader;

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;

pub use reader::{ImportFormat, Preview, ReadOptions};

use crate::connections::store::ConnectionStore;
use crate::drivers::{self, TableRef};
use crate::error::{AppError, Result};
use crate::jobs::{Every, Jobs, unless_cancelled};
use crate::statement_log::{Origin, StatementLog};

/// Rows a preview shows.
const PREVIEW_ROWS: usize = 20;
/// Rows read before they are handed on to be inserted.
const BATCH: usize = 1000;
/// Batches read ahead of the inserts at most.
const READ_AHEAD: usize = 2;

/// The file's fields and its first rows, for the import dialog to map.
#[tauri::command(async)]
pub fn preview_import(path: PathBuf, format: ImportFormat, options: Option<ReadOptions>) -> Result<Preview> {
    reader::fields_of(format, &options.unwrap_or_default(), open(&path)?, PREVIEW_ROWS)
}

fn open(path: &Path) -> Result<BufReader<File>> {
    File::open(path)
        .map(BufReader::new)
        .map_err(|error| AppError::Invalid(format!("{} can't be opened: {error}", path.display())))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub job_id: String,
    pub connection_id: String,
    pub table: TableRef,
    pub path: PathBuf,
    pub format: ImportFormat,
    pub options: Option<ReadOptions>,
    /// The file's fields, as the preview read them.
    pub fields: Vec<String>,
    /// The column each field goes into, in the same order; `None` leaves it out.
    pub columns: Vec<Option<String>>,
}

/// How many rows an import has added so far, for its dialog.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress<'a> {
    job_id: &'a str,
    rows: u64,
}

const PROGRESS: &str = "import-progress";
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// Imports the file into the table and returns how many rows went in. Job
/// `job_id` reports its rows as it goes and can be cancelled: on a SQL
/// engine that rolls everything back.
#[tauri::command]
pub async fn import_table(
    app: AppHandle,
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    jobs: State<'_, Jobs>,
    request: ImportRequest,
) -> Result<u64> {
    let (picks, columns) = mapping(&request.fields, &request.columns)?;
    let opened = store.open(&request.connection_id)?;
    let input = open(&request.path)?;
    let job = jobs.start(&request.job_id);
    let mut batches = read_in_batches(request.clone(), input, picks);
    let mut every = Every::new(PROGRESS_EVERY);
    let mut report = |rows: u64| {
        if every.due() {
            let _ = app.emit(PROGRESS, Progress { job_id: &request.job_id, rows });
        }
    };
    let import = opened.run(&log, Origin::Import, async |connection, password| {
        drivers::import_rows(connection, password, &request.table, &columns, &mut batches, &mut report).await
    });
    unless_cancelled(job.token(), import).await
}

/// Which fields are imported, and the column each goes into.
fn mapping(fields: &[String], columns: &[Option<String>]) -> Result<(Vec<usize>, Vec<String>)> {
    if fields.len() != columns.len() {
        return Err(AppError::Invalid("Every field of the file needs a column, or to be left out.".into()));
    }
    let (picks, columns): (Vec<usize>, Vec<String>) =
        columns.iter().enumerate().filter_map(|(field, column)| Some((field, column.clone()?))).unzip();
    if columns.is_empty() {
        return Err(AppError::Invalid("Choose at least one column to import into.".into()));
    }
    Ok((picks, columns))
}

/// Reads the file on a thread of its own, sending its rows (only the `picks`
/// fields) in batches; a read error is sent last. Reading stops when the
/// receiver is dropped: the import failed or was cancelled.
fn read_in_batches(request: ImportRequest, input: BufReader<File>, picks: Vec<usize>) -> drivers::ImportBatches {
    let (sender, receiver) = mpsc::channel(READ_AHEAD);
    tokio::task::spawn_blocking(move || {
        let options = request.options.unwrap_or_default();
        let mut batch = Vec::with_capacity(BATCH);
        let read = reader::read_rows(request.format, &options, &request.fields, input, |row| {
            batch.push(picks.iter().map(|&field| row[field].clone()).collect());
            if batch.len() < BATCH {
                return Ok(true);
            }
            let full = std::mem::replace(&mut batch, Vec::with_capacity(BATCH));
            Ok(sender.blocking_send(Ok(full)).is_ok())
        });
        let last = match read {
            Ok(()) if batch.is_empty() => return,
            Ok(()) => Ok(batch),
            Err(error) => Err(error),
        };
        let _ = sender.blocking_send(last);
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_fields_given_a_column_are_imported() {
        let fields = ["a".to_string(), "b".into(), "c".into()];
        let (picks, columns) = mapping(&fields, &[Some("x".into()), None, Some("z".into())]).unwrap();
        assert_eq!((picks, columns), (vec![0, 2], vec!["x".to_string(), "z".into()]));
        let none = mapping(&fields, &[None, None, None]).unwrap_err();
        assert_eq!(none.to_string(), "Choose at least one column to import into.");
        assert!(mapping(&fields, &[None]).is_err());
    }

    /// A SQLite file holding `sql`, as a saved connection.
    async fn sqlite(dir: &Path, sql: &str) -> crate::connections::model::Connection {
        use sqlx::{AssertSqlSafe, ConnectOptions};
        let path = dir.join("test.db");
        let mut conn =
            sqlx::sqlite::SqliteConnectOptions::new().filename(&path).create_if_missing(true).connect().await.unwrap();
        sqlx::raw_sql(AssertSqlSafe(sql)).execute(&mut conn).await.unwrap();
        crate::connections::model::Connection {
            id: "import".into(),
            name: "import".into(),
            engine: crate::connections::model::Engine::Sqlite,
            environment: None,
            host: String::new(),
            port: 0,
            database: path.to_string_lossy().into_owned(),
            username: String::new(),
            tls: false,
            allow_self_signed: false,
            srv: false,
            agent_access: Default::default(),
        }
    }

    /// What `import_table` does, without the app around it.
    async fn import_file(connection: &crate::connections::model::Connection, request: ImportRequest) -> Result<u64> {
        let (picks, columns) = mapping(&request.fields, &request.columns)?;
        let input = open(&request.path)?;
        let table = request.table.clone();
        let mut batches = read_in_batches(request, input, picks);
        drivers::import_rows(connection, None, &table, &columns, &mut batches, &mut |_| {}).await
    }

    #[tokio::test]
    async fn an_exported_file_imports_back_and_a_bad_row_adds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let connection = sqlite(dir.path(), "CREATE TABLE people (id INTEGER, name TEXT, note TEXT);").await;
        let table = TableRef { database: "main".into(), schema: "main".into(), name: "people".into() };
        // As export writes CSV: quoted where needed, NULL left empty.
        let mut text = String::from("id,name,note\n");
        for n in 0..2500 {
            text.push_str(&format!("{n},\"Name, {n}\",\n"));
        }
        let path = dir.path().join("people.csv");
        std::fs::write(&path, &text).unwrap();
        let request = |path: &Path| ImportRequest {
            job_id: "j".into(),
            connection_id: "c".into(),
            table: table.clone(),
            path: path.to_path_buf(),
            format: ImportFormat::Csv,
            options: None,
            fields: vec!["id".into(), "name".into(), "note".into()],
            columns: vec![Some("id".into()), Some("name".into()), Some("note".into())],
        };
        assert_eq!(import_file(&connection, request(&path)).await.unwrap(), 2500);

        // A row of the wrong width near the end: the rows read before it don't stay.
        text.push_str("1,2\n");
        let bad = dir.path().join("bad.csv");
        std::fs::write(&bad, &text).unwrap();
        let error = import_file(&connection, request(&bad)).await.unwrap_err();
        assert_eq!(error.to_string(), "Row 2502 has 2 values, but the file has 3 columns.");

        let rows = drivers::fetch_rows(&connection, None, &table, &Default::default(), 2499, 10).await.unwrap();
        assert_eq!(rows.rows, [vec![Some("2499".into()), Some("Name, 2499".into()), None]]);
    }

    #[tokio::test]
    async fn a_file_is_read_in_batches_and_a_read_error_comes_last() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rows.csv");
        let mut text = String::from("id,name\n");
        for n in 0..BATCH + 5 {
            text.push_str(&format!("{n},row {n}\n"));
        }
        text.push_str("broken\n");
        std::fs::write(&path, text).unwrap();
        let request = ImportRequest {
            job_id: "j".into(),
            connection_id: "c".into(),
            table: TableRef { database: "d".into(), schema: "s".into(), name: "t".into() },
            path: path.clone(),
            format: ImportFormat::Csv,
            options: None,
            fields: vec!["id".into(), "name".into()],
            columns: vec![None, Some("name".into())],
        };
        let mut batches = read_in_batches(request, open(&path).unwrap(), vec![1]);
        let first = batches.recv().await.unwrap().unwrap();
        assert_eq!((first.len(), first[0].clone()), (BATCH, vec![Some("row 0".to_string())]));
        let error = batches.recv().await.unwrap().unwrap_err();
        assert_eq!(error.to_string(), format!("Row {} has 1 value, but the file has 2 columns.", BATCH + 7));
        assert!(batches.recv().await.is_none());
    }
}
