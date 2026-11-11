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
