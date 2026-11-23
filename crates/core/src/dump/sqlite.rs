//! SQLite, built in: no tool to install. An export is either a SQL script of
//! the schema and the data, in the shape `sqlite3 .dump` writes (so either one
//! restores the other's), or a consistent copy of the file (`VACUUM INTO`).
//!
//! A restore runs a script in one transaction: into the connection's own file,
//! where a failure or a cancel rolls everything back, or into a new file,
//! which is written beside its place first, removed when the restore fails,
//! and never moved over a file that appeared there meanwhile. A script that
//! commits or begins transactions of its own is refused.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use sqlx::{AssertSqlSafe, Connection as _};
use tokio_util::sync::CancellationToken;

use super::{Restored, blocking, stop_if};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};
use crate::files::partial_path;
use crate::jobs::cancelled;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SqliteFormat {
    /// A SQL script of the schema and the data.
    Script,
    /// A copy of the database file.
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqliteOptions {
    pub format: SqliteFormat,
}

/// What a SQLite database file starts with.
const FILE_HEADER: &[u8] = b"SQLite format 3\0";

/// Writes the connection's database to `file`.
pub(super) async fn dump(
    connection: &Connection,
    file: &Path,
    options: &SqliteOptions,
    cancel: &CancellationToken,
    mut on_line: impl FnMut(&str),
) -> Result<()> {
    stop_if(cancel)?;
    let mut conn = open(&connection.database, false).await?;
    let outcome = match options.format {
        SqliteFormat::File => {
            on_line("Copying the database file");
            let target = file.to_string_lossy().replace('\'', "''");
            sqlx::raw_sql(AssertSqlSafe(format!("VACUUM INTO '{target}'")))
                .execute(&mut conn)
                .await
                .map(|_| ())
                .map_err(AppError::from)
        }
        SqliteFormat::Script => write_script(&mut conn, file, cancel, on_line).await,
    };
    conn.close().await?;
    outcome
}

/// Opens the database file at `path`; a new one only when `create`.
async fn open(path: &str, create: bool) -> Result<SqliteConnection> {
    if path.is_empty() {
        return Err(AppError::Invalid("Choose the SQLite database file.".into()));
    }
    // Restored rows go in whatever order the file has them, as with sqlite3.
    let options = SqliteConnectOptions::new().filename(path).create_if_missing(create).foreign_keys(false);
    SqliteConnection::connect_with(&options).await.map_err(|error| match error {
        sqlx::Error::Database(_) => AppError::Invalid(format!("Could not open {path}: {error}")),
        other => AppError::from(other),
    })
}

/// The schema and data as SQL: tables with their rows, the AUTOINCREMENT
/// counters, then indexes, triggers and views in the order they were made, so
/// each comes after what it depends on and triggers don't fire on the rows.
async fn write_script(
    conn: &mut SqliteConnection,
    file: &Path,
    cancel: &CancellationToken,
    mut on_line: impl FnMut(&str),
) -> Result<()> {
    let mut out = BufWriter::new(std::fs::File::create(file)?);
    writeln!(out, "PRAGMA foreign_keys=OFF;\nBEGIN TRANSACTION;")?;
    let tables: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, sql FROM sqlite_master WHERE type = 'table' AND sql IS NOT NULL AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY rowid",
    )
    .fetch_all(&mut *conn)
    .await?;
    // A virtual table keeps its rows in tables named after it, which it creates itself.
    let virtual_tables: Vec<&str> = tables
        .iter()
        .filter(|(_, sql)| sql.trim_start().to_ascii_uppercase().starts_with("CREATE VIRTUAL TABLE"))
        .map(|(name, _)| name.as_str())
        .collect();
    let is_shadow = |name: &str| {
        virtual_tables.iter().any(|table| name.len() > table.len() + 1 && name.starts_with(&format!("{table}_")))
    };
    for (name, sql) in &tables {
        if is_shadow(name) {
            continue;
        }
        stop_if(cancel)?;
        on_line(&format!("Dumping table {name}"));
        writeln!(out, "{sql};")?;
        write_rows(conn, &mut out, name, cancel).await?;
    }
    let sequence: Option<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'sqlite_sequence'")
            .fetch_optional(&mut *conn)
            .await?;
    if sequence.is_some() {
        writeln!(out, "DELETE FROM sqlite_sequence;")?;
        write_rows(conn, &mut out, "sqlite_sequence", cancel).await?;
    }
    let others: Vec<(String, String)> = sqlx::query_as(
        "SELECT tbl_name, sql FROM sqlite_master WHERE type IN ('index', 'trigger', 'view') AND sql IS NOT NULL \
         AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY rowid",
    )
    .fetch_all(&mut *conn)
    .await?;
    for (table, sql) in others {
        if !is_shadow(&table) {
            writeln!(out, "{sql};")?;
        }
    }
    writeln!(out, "COMMIT;")?;
    out.into_inner().map_err(|error| error.into_error())?.sync_all()?;
    Ok(())
}

/// An `INSERT` per row of `table`, its values written by SQLite's own `quote`:
/// text quoted, blobs as `X'…'`, reals exactly, NULL as NULL. Generated and
/// hidden columns are left out; SQLite fills them.
async fn write_rows(
    conn: &mut SqliteConnection,
    out: &mut impl Write,
    table: &str,
    cancel: &CancellationToken,
) -> Result<()> {
    let columns: Vec<String> = sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT name FROM pragma_table_info({}) ORDER BY cid",
        literal(table)
    )))
    .fetch_all(&mut *conn)
    .await?;
    if columns.is_empty() {
        return Ok(());
    }
    let names = columns.iter().map(|column| quote_ident(column)).collect::<Vec<_>>().join(",");
    let values =
        columns.iter().map(|column| format!("quote({})", quote_ident(column))).collect::<Vec<_>>().join(" || ',' || ");
    let prefix = format!("INSERT INTO {}({names}) VALUES(", quote_ident(table));
    let query = format!("SELECT {values} FROM {}", quote_ident(table));
    let mut rows = sqlx::query_scalar::<_, String>(AssertSqlSafe(query)).fetch(&mut *conn);
    while let Some(row) = rows.try_next().await? {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        writeln!(out, "{prefix}{row});")?;
    }
    Ok(())
}

/// `name` as a SQLite identifier: in double quotes, inner ones doubled.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `text` as a SQLite string literal.
fn literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Runs the script at `path` into `target`: the connection's own file, or a new one.
pub(super) async fn restore(
    connection: &Connection,
    target: &str,
    path: &Path,
    cancel: &CancellationToken,
    on_line: impl FnMut(&str),
) -> Result<Restored> {
    let file = path.to_path_buf();
    let bytes = blocking(move || std::fs::read(&file)).await?;
    if bytes.starts_with(FILE_HEADER) {
        return Err(AppError::Invalid(format!(
            "{} is a SQLite database file, not a script. Open it as a connection instead.",
            path.display()
        )));
    }
    let script = String::from_utf8(bytes)
        .map_err(|_| AppError::Invalid(format!("{} isn't a SQL script: it isn't UTF-8 text.", path.display())))?;
    let script = without_own_transaction(&script);
    if let Some(statement) = transaction_control(&script) {
        return Err(AppError::Invalid(format!(
            "{} has transaction statements of its own ({statement}), so it can't be restored in one transaction. \
             Remove its BEGIN, COMMIT, END, ROLLBACK, SAVEPOINT and RELEASE statements and try again.",
            path.display()
        )));
    }
    stop_if(cancel)?;
    if target == connection.database {
        let mut conn = open(target, false).await?;
        let outcome = run_script(&mut conn, &script, cancel, on_line).await;
        conn.close().await?;
        return outcome.map(|()| Restored::created(false));
    }
    let target = PathBuf::from(target);
    if target.exists() {
        return Err(AppError::Invalid(format!(
            "There's already a file at {}. Restore into the connection's own file or a new one.",
            target.display()
        )));
    }
    if let Some(dir) = target.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let partial = partial_path(&target);
    remove_database(&partial);
    let outcome = async {
        let mut conn = open(&partial.to_string_lossy(), true).await?;
        let outcome = run_script(&mut conn, &script, cancel, on_line).await;
        conn.close().await?;
        outcome
    }
    .await;
    if let Err(error) = outcome {
        remove_database(&partial);
        return Err(error);
    }
    place(&partial, &target)?;
    Ok(Restored::created(true))
}

/// Moves the restored database at `partial` to `target`, never over a file
/// that appeared there meanwhile: a hard link fails when `target` exists (a
/// copy into a file only created when missing where links aren't supported).
/// `partial` and its journal files are removed either way.
fn place(partial: &Path, target: &Path) -> Result<()> {
    let placed = match std::fs::hard_link(partial, target) {
        Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => copy_new(partial, target),
        linked => linked,
    };
    remove_database(partial);
    match placed {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(AppError::Invalid(format!(
            "There's already a file at {}. Restore into the connection's own file or a new one.",
            target.display()
        ))),
        Err(error) => Err(error.into()),
    }
}

/// Copies `from` to `to`, a file created only when there's none yet.
fn copy_new(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    let copied = std::io::copy(&mut std::fs::File::open(from)?, &mut out).and_then(|_| out.sync_all());
    if copied.is_err() {
        let _ = std::fs::remove_file(to);
    }
    copied
}

/// Removes the database file at `path` and what SQLite keeps beside it.
fn remove_database(path: &Path) {
    for suffix in ["", "-journal", "-wal", "-shm"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(file));
    }
}

/// Runs `script` in one transaction, which a failure or a cancel rolls back.
async fn run_script(
    conn: &mut SqliteConnection,
    script: &str,
    cancel: &CancellationToken,
    mut on_line: impl FnMut(&str),
) -> Result<()> {
    let mut transaction = conn.begin().await?;
    let mut ran = 0_u64;
    {
        let mut results = sqlx::raw_sql(AssertSqlSafe(script.to_owned())).execute_many(&mut *transaction);
        loop {
            let next = tokio::select! {
                next = results.try_next() => next?,
                () = cancel.cancelled() => return Err(cancelled()),
            };
            if next.is_none() {
                break;
            }
            ran += 1;
            on_line(&format!("{ran} statements run"));
        }
    }
    stop_if(cancel)?;
    transaction.commit().await?;
    Ok(())
}

/// `script` without the transaction a `sqlite3 .dump` script (and DBForge's)
/// wraps itself in, since the restore runs it in its own: a `BEGIN
/// TRANSACTION;` line before any statement but `PRAGMA`s, and a `COMMIT;` last line.
fn without_own_transaction(script: &str) -> String {
    let mut lines: Vec<&str> = script.lines().collect();
    let is_statement = |line: &&str| {
        let line = line.trim();
        !line.is_empty() && !line.starts_with("--")
    };
    if let Some(first) = lines
        .iter()
        .position(|line| is_statement(line) && !line.trim_start().to_ascii_uppercase().starts_with("PRAGMA"))
        && matches!(lines[first].trim().to_ascii_uppercase().as_str(), "BEGIN TRANSACTION;" | "BEGIN;")
    {
        lines[first] = "";
    }
    if let Some(last) = lines.iter().rposition(is_statement)
        && matches!(lines[last].trim().to_ascii_uppercase().as_str(), "COMMIT;" | "END TRANSACTION;")
    {
        lines[last] = "";
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// The first statement in `script` that starts, ends or marks a transaction
/// (`BEGIN`, `COMMIT`, `END`, `ROLLBACK`, `SAVEPOINT`, `RELEASE`), by its
/// first word. Strings, quoted names and comments are skipped, and a trigger's
/// body is part of its `CREATE TRIGGER`, which ends at `END;` as SQLite reads it.
fn transaction_control(script: &str) -> Option<String> {
    const CONTROL: [&str; 6] = ["BEGIN", "COMMIT", "END", "ROLLBACK", "SAVEPOINT", "RELEASE"];
    let chars: Vec<char> = script.chars().collect();
    // The statement's first words (a token that isn't a word counts as an empty one).
    let mut words: Vec<String> = Vec::new();
    let mut trigger = false;
    let mut after_end = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '-' && next == Some('-') {
            i = chars[i..].iter().position(|&c| c == '\n').map_or(chars.len(), |at| i + at);
            continue;
        }
        if c == '/' && next == Some('*') {
            i = (i + 2..chars.len().saturating_sub(1))
                .find(|&at| chars[at] == '*' && chars[at + 1] == '/')
                .map_or(chars.len(), |at| at + 2);
            continue;
        }
        if c == ';' {
            if !trigger || after_end {
                words.clear();
                trigger = false;
            }
            after_end = false;
            i += 1;
            continue;
        }
        let word = if c.is_alphanumeric() || c == '_' {
            let end =
                chars[i..].iter().position(|&c| !(c.is_alphanumeric() || c == '_')).map_or(chars.len(), |at| i + at);
            let word: String = chars[i..end].iter().collect::<String>().to_ascii_uppercase();
            i = end;
            word
        } else {
            let close = match c {
                '\'' | '"' | '`' => Some(c),
                '[' => Some(']'),
                _ => None,
            };
            i += 1;
            if let Some(close) = close {
                i = chars[i..].iter().position(|&c| c == close).map_or(chars.len(), |at| i + at + 1);
            }
            String::new()
        };
        if words.is_empty() && CONTROL.contains(&word.as_str()) {
            return Some(word);
        }
        after_end = word == "END";
        if words.len() < 3 {
            words.push(word);
            trigger = trigger
                || matches!(
                    words.iter().map(String::as_str).collect::<Vec<_>>().as_slice(),
                    ["CREATE", "TRIGGER"] | ["CREATE", "TEMP" | "TEMPORARY", "TRIGGER"]
                );
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::model::Engine;
    use crate::dump::{DumpOptions, RestoreOptions, Run, export, import};

    fn sqlite(path: &Path) -> Connection {
        Connection {
            engine: Engine::Sqlite,
            host: String::new(),
            port: 0,
            username: String::new(),
            database: path.display().to_string(),
            ..crate::drivers::postgres::tests::local()
        }
    }

    async fn execute(path: &Path, sql: &str) {
        let options = SqliteConnectOptions::new().filename(path).create_if_missing(true);
        let mut conn = SqliteConnection::connect_with(&options).await.unwrap();
        sqlx::raw_sql(AssertSqlSafe(sql.to_string())).execute(&mut conn).await.unwrap();
        conn.close().await.unwrap();
    }

    async fn rows(path: &Path, sql: &str) -> Vec<String> {
        let options = SqliteConnectOptions::new().filename(path);
        let mut conn = SqliteConnection::connect_with(&options).await.unwrap();
        let rows = sqlx::query_scalar(AssertSqlSafe(sql.to_string())).fetch_all(&mut conn).await.unwrap();
        conn.close().await.unwrap();
        rows
    }

    const SCHEMA: &str = r#"
        CREATE TABLE "odd ""name""" (id INTEGER PRIMARY KEY AUTOINCREMENT, t TEXT, r REAL, b BLOB, n,
                                     doubled INTEGER GENERATED ALWAYS AS (id * 2) VIRTUAL);
        CREATE TABLE child (id INTEGER PRIMARY KEY, parent INTEGER REFERENCES "odd ""name"""(id));
        INSERT INTO "odd ""name""" (t, r, b, n) VALUES
            ('it''s "quoted"' || char(10) || 'COMMIT;', 0.1, X'00FF10', NULL),
            ('ünïcode ☃', -1.5e300, X'', 9223372036854775807),
            (NULL, 1.0, NULL, 'text in an untyped column');
        INSERT INTO child VALUES (1, 1);
        CREATE INDEX odd_t ON "odd ""name"""(t);
        CREATE VIEW named AS SELECT t FROM "odd ""name""";
        CREATE TRIGGER counted AFTER INSERT ON child BEGIN UPDATE child SET parent = parent; END;
    "#;

    const VALUES: &str = r#"SELECT quote(id) || '|' || quote(t) || '|' || quote(r) || '|' || quote(b) || '|' || quote(n) || '|' || doubled FROM "odd ""name""" ORDER BY id"#;

    async fn export_to(source: &Path, file: &Path, format: SqliteFormat) -> Result<()> {
        let cancel = CancellationToken::new();
        let options = DumpOptions::Sqlite(SqliteOptions { format });
        let run = Run { tool: None, cancel: &cancel };
        export(&sqlite(source), None, "", file, &options, run, |_| {}).await.map(|_| ())
    }

    async fn restore_into(
        connection: &Connection,
        target: &Path,
        file: &Path,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        let run = Run { tool: None, cancel };
        let target = target.display().to_string();
        Ok(import(connection, None, &target, file, &RestoreOptions::default(), run, |_| {}).await?.created)
    }

    #[tokio::test]
    async fn a_script_export_restores_every_value_into_a_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.db");
        execute(&source, SCHEMA).await;
        let file = dir.path().join("new folder").join("source.sql");
        export_to(&source, &file, SqliteFormat::Script).await.unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            text.starts_with("PRAGMA foreign_keys=OFF;\nBEGIN TRANSACTION;\n") && text.ends_with("COMMIT;\n"),
            "{text}"
        );
        assert!(
            text.contains(r#"INSERT INTO "odd ""name"""("id","t","r","b","n") VALUES(1,'it''s "quoted""#),
            "{text}"
        );
        assert!(text.contains("X'00FF10'") && text.contains("NULL"), "{text}");
        assert!(
            text.find("CREATE TRIGGER").unwrap() > text.find("INSERT INTO \"child\"").unwrap(),
            "triggers after rows"
        );
        assert_eq!(std::fs::read_dir(file.parent().unwrap()).unwrap().count(), 1, "no partial file is left");

        let copy = dir.path().join("copy.db");
        assert!(restore_into(&sqlite(&source), &copy, &file, &CancellationToken::new()).await.unwrap());
        assert_eq!(rows(&copy, VALUES).await, rows(&source, VALUES).await);
        assert_eq!(rows(&copy, "SELECT quote(r) FROM \"odd \"\"name\"\"\" WHERE id = 1").await, ["0.1"]);
        assert_eq!(
            rows(&copy, "SELECT name FROM sqlite_master ORDER BY name").await,
            rows(&source, "SELECT name FROM sqlite_master ORDER BY name").await
        );
        assert_eq!(rows(&copy, "SELECT CAST(seq AS TEXT) FROM sqlite_sequence").await, ["3"]);
        assert_eq!(rows(&copy, "SELECT count(*) || '' FROM named").await, ["3"]);

        // A file already at the place is never replaced.
        let error = restore_into(&sqlite(&source), &copy, &file, &CancellationToken::new()).await.unwrap_err();
        assert!(error.to_string().starts_with("There's already a file at"), "{error}");
    }

    #[tokio::test]
    async fn a_file_export_is_a_copy_that_opens() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.db");
        execute(&source, SCHEMA).await;
        let file = dir.path().join("copy-2026-11-22-1200.db");
        std::fs::write(&file, "an older copy").unwrap();
        export_to(&source, &file, SqliteFormat::File).await.unwrap();
        assert_eq!(rows(&file, VALUES).await, rows(&source, VALUES).await);
        // A copy isn't a script to restore.
        let error = restore_into(&sqlite(&source), &dir.path().join("x.db"), &file, &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("is a SQLite database file, not a script"), "{error}");
    }

    #[tokio::test]
    async fn a_failed_restore_changes_nothing_and_removes_the_file_it_created() {
        let dir = tempfile::tempdir().unwrap();
        let own = dir.path().join("own.db");
        execute(&own, "CREATE TABLE kept (id INTEGER); INSERT INTO kept VALUES (1);").await;
        let broken = dir.path().join("broken.sql");
        std::fs::write(&broken, "BEGIN TRANSACTION;\nCREATE TABLE ok (id INTEGER);\nINSERT INTO kept VALUES (2);\nINSERT INTO missing VALUES (1);\nCOMMIT;\n").unwrap();

        let error = restore_into(&sqlite(&own), &own, &broken, &CancellationToken::new()).await.unwrap_err();
        assert!(error.to_string().contains("no such table: missing"), "{error}");
        assert_eq!(rows(&own, "SELECT group_concat(name) FROM sqlite_master").await, ["kept"]);
        assert_eq!(rows(&own, "SELECT count(*) || '' FROM kept").await, ["1"]);

        let new = dir.path().join("new.db");
        restore_into(&sqlite(&own), &new, &broken, &CancellationToken::new()).await.unwrap_err();
        let left: Vec<_> =
            std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert!(!left.iter().any(|name| name.starts_with("new.db")), "{left:?}");

        // Into its own file, a script that works.
        let good = dir.path().join("good.sql");
        std::fs::write(&good, "INSERT INTO kept VALUES (2);\n").unwrap();
        assert!(!restore_into(&sqlite(&own), &own, &good, &CancellationToken::new()).await.unwrap());
        assert_eq!(rows(&own, "SELECT count(*) || '' FROM kept").await, ["2"]);

        // Stopped before it started.
        let stopped = CancellationToken::new();
        stopped.cancel();
        let error = restore_into(&sqlite(&own), &own, &good, &stopped).await.unwrap_err();
        assert_eq!(error.to_string(), "Cancelled.");
        assert_eq!(rows(&own, "SELECT count(*) || '' FROM kept").await, ["2"]);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let run = Run { tool: None, cancel: &cancel };
        let options = DumpOptions::Sqlite(SqliteOptions { format: SqliteFormat::Script });
        let error = export(&sqlite(&own), None, "", &dir.path().join("y.sql"), &options, run, |_| {}).await;
        assert_eq!(error.unwrap_err().to_string(), "Cancelled.");
        assert!(!dir.path().join("y.sql").exists() && !dir.path().join("y.sql.dbforge-partial").exists());
    }

    #[test]
    fn the_scripts_own_transaction_gives_way_to_the_restores() {
        let dump = "PRAGMA foreign_keys=OFF;\nBEGIN TRANSACTION;\nCREATE TABLE t(x);\nINSERT INTO t VALUES('COMMIT;');\nCOMMIT;\n";
        assert_eq!(
            without_own_transaction(dump),
            "PRAGMA foreign_keys=OFF;\n\nCREATE TABLE t(x);\nINSERT INTO t VALUES('COMMIT;');\n\n"
        );
        let plain = "CREATE TABLE t(x);\nINSERT INTO t VALUES(1);\n";
        assert_eq!(without_own_transaction(plain), plain);
        // A trigger's END stays.
        let trigger = "CREATE TRIGGER x AFTER INSERT ON t BEGIN\n  SELECT 1;\nEND;\n";
        assert_eq!(without_own_transaction(trigger), trigger);
    }

    #[test]
    fn transaction_statements_are_found_outside_strings_comments_and_triggers() {
        let found = |script: &str| transaction_control(script);
        assert_eq!(found("CREATE TABLE t(x);\nCOMMIT;\nINSERT INTO t VALUES(1);"), Some("COMMIT".into()));
        assert_eq!(found("INSERT INTO t VALUES(1); begin immediate;"), Some("BEGIN".into()));
        for keyword in ["END", "ROLLBACK", "SAVEPOINT", "RELEASE"] {
            assert_eq!(found(&format!("SELECT 1;\n  /* note */ {keyword} x;")), Some(keyword.into()), "{keyword}");
        }
        let fine = "-- COMMIT;\n/* BEGIN; */\nINSERT INTO t VALUES('x; COMMIT;', \"a;\"\"COMMIT\", `b; END`, [c; END]);\n\
             CREATE TRIGGER counted AFTER INSERT ON t BEGIN\n  UPDATE t SET x = 1;\n  SELECT CASE WHEN 1 THEN 2 END AS y;\nEND;\n\
             CREATE TEMP TRIGGER u AFTER DELETE ON t BEGIN SELECT 1; END;\n\
             CREATE TABLE commit_log(begin_at, end_at);\nINSERT INTO t VALUES(1);\n";
        assert_eq!(found(fine), None);
        assert_eq!(found(&format!("{fine}END TRANSACTION;")), Some("END".into()), "after a trigger, counted again");
    }

    #[tokio::test]
    async fn a_script_with_its_own_transactions_is_refused_before_it_runs() {
        let dir = tempfile::tempdir().unwrap();
        let own = dir.path().join("own.db");
        execute(&own, "CREATE TABLE kept (id INTEGER);").await;
        let script = dir.path().join("commits.sql");
        std::fs::write(
            &script,
            "BEGIN TRANSACTION;\nCREATE TABLE early (id INTEGER);\nCOMMIT;\nINSERT INTO missing VALUES (1);\nCOMMIT;\n",
        )
        .unwrap();
        let error = restore_into(&sqlite(&own), &own, &script, &CancellationToken::new()).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("has transaction statements of its own (COMMIT), so it can't be restored in one transaction"),
            "{error}"
        );
        assert_eq!(rows(&own, "SELECT group_concat(name) FROM sqlite_master").await, ["kept"]);
        let new = dir.path().join("new.db");
        restore_into(&sqlite(&own), &new, &script, &CancellationToken::new()).await.unwrap_err();
        assert!(!new.exists());
    }

    #[test]
    fn a_file_that_appeared_meanwhile_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("new.db");
        let partial = partial_path(&target);
        std::fs::write(&partial, "restored").unwrap();
        std::fs::write(PathBuf::from(format!("{}-journal", partial.display())), "journal").unwrap();
        std::fs::write(&target, "someone else's").unwrap();
        let error = place(&partial, &target).unwrap_err();
        assert!(error.to_string().starts_with("There's already a file at"), "{error}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "someone else's");
        let left: Vec<_> =
            std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert_eq!(left, ["new.db"], "the partial file and its journal are removed");

        std::fs::write(&partial, "restored").unwrap();
        let free = dir.path().join("free.db");
        place(&partial, &free).unwrap();
        assert_eq!(std::fs::read_to_string(&free).unwrap(), "restored");
        assert!(!partial.exists());
    }

    #[test]
    fn identifiers_and_literals_are_quoted() {
        assert_eq!(quote_ident(r#"a "b""#), r#""a ""b""""#);
        assert_eq!(literal("it's"), "'it''s'");
    }
}
