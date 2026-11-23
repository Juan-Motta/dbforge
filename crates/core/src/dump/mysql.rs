//! MySQL and MariaDB, with mysqldump and mysql (or MariaDB's mariadb-dump and
//! mariadb). The tools read the connection from an option file only their
//! owner can read, deleted after the run, and from nothing else: not the
//! user's own option files, login paths or `MYSQL_…` settings, which could
//! send them elsewhere.
//!
//! An export holds one database's objects without naming the database, so it
//! restores into a database of any name; a file that names its database
//! (`USE`, `CREATE DATABASE`) is refused unless it names the target. A restore stops at the first error;
//! MySQL can't undo the tables it already created then (its DDL commits on
//! its own), but a database created for the restore is dropped again.

mod script;

use std::ffi::OsString;
use std::io::BufReader;
use std::path::Path;
use std::process::Stdio;

use serde::Deserialize;
use tokio::process::Command;

use super::secret_file::SecretFile;
use super::tools::{self, Tool};
use super::{Content, Restored, Run, blocking, stop_if};
use crate::connections::model::{Connection, Engine};
use crate::connections::transport::{Ssl, SslMode};
use crate::drivers::{NewObject, mysql};
use crate::error::{AppError, Result};
use crate::tunnel::Route;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysqlOptions {
    pub content: Content,
    /// `DROP TABLE IF EXISTS` before each table, so the file restores over an existing database.
    pub add_drop_table: bool,
    /// Reads InnoDB tables in one consistent snapshot, without locking them.
    pub single_transaction: bool,
    /// Stored procedures and functions, triggers and events.
    pub routines: bool,
}

/// Writes `database` to `file` with mysqldump.
pub(super) async fn dump(
    connection: &Connection,
    password: Option<&str>,
    database: &str,
    file: &Path,
    options: &MysqlOptions,
    run: Run<'_>,
    on_line: impl FnMut(&str),
) -> Result<()> {
    let tool = run.tool()?;
    // mysqldump takes the database as an argument: one starting with `-` would read as an option.
    if database.is_empty() || database.starts_with('-') {
        return Err(AppError::Invalid(format!("Can't export a database named “{database}”.")));
    }
    let route = crate::tunnel::route(connection).await?;
    let defaults = SecretFile::new(&option_file(connection, &route, password, tool.mariadb), ".cnf")?;
    let mut command = command(tool, &defaults);
    command.args(dump_args(options, tool, connection.engine, file)).arg(database);
    tools::run(command, run.cancel, on_line).await
}

/// Runs the SQL file at `path` into `database` with mysql, creating it first
/// when it doesn't exist. A file that would switch to another database (as a
/// `mysqldump --databases` export does) is refused before anything runs.
pub(super) async fn restore(
    connection: &Connection,
    password: Option<&str>,
    database: &str,
    path: &Path,
    run: Run<'_>,
    on_line: impl FnMut(&str),
) -> Result<Restored> {
    let tool = run.tool()?;
    let file = path.to_path_buf();
    let target = database.to_string();
    let switch = blocking(move || script::leaves(BufReader::new(std::fs::File::open(&file)?), &target)).await?;
    if let Some(switch) = switch {
        return Err(switch.refusal(database));
    }
    stop_if(run.cancel)?;
    let created = !mysql::database_exists(connection, password, database).await?;
    stop_if(run.cancel)?;
    if created {
        let new = NewObject::Database { name: database.to_string(), collection: None };
        mysql::create_object(connection, password, &new).await?;
    }
    let outcome = async {
        let route = crate::tunnel::route(connection).await?;
        let defaults = SecretFile::new(&option_file(connection, &route, password, tool.mariadb), ".cnf")?;
        let mut command = command(tool, &defaults);
        command.args(restore_args(database));
        let input = Stdio::from(std::fs::File::open(path)?);
        tools::run_with_input(command, input, run.cancel, on_line).await
    }
    .await;
    let Err(error) = outcome else { return Ok(Restored::created(created)) };
    if created && let Err(problem) = mysql::drop_database(connection, password, database).await {
        return Err(AppError::Invalid(format!(
            "{error}\nThe database {database} created for the import couldn't be removed: {problem}"
        )));
    }
    Err(error)
}

/// `tool`, reading the connection from `defaults` and nothing else.
fn command(tool: &Tool, defaults: &SecretFile) -> Command {
    let mut command = Command::new(&tool.path);
    // Read first, or the tools refuse it.
    let mut option = OsString::from("--defaults-file=");
    option.push(defaults.path());
    command.arg(option);
    for (name, _) in std::env::vars_os() {
        let name_text = name.to_string_lossy();
        if name_text.starts_with("MYSQL") || name_text.starts_with("MARIADB") {
            command.env_remove(name);
        }
    }
    // MySQL's tools read the login paths of `mysql_config_editor` even with
    // `--defaults-file`; this file doesn't exist, so there are none.
    let mut none = defaults.path().as_os_str().to_owned();
    none.push(".no-login-paths");
    command.env("MYSQL_TEST_LOGIN_FILE", none);
    command
}

/// The option file the tools sign in with: the server (or its tunnel), the
/// user and password, and SSL as the connection says. MariaDB's tools name
/// SSL settings their own way.
fn option_file(connection: &Connection, route: &Route, password: Option<&str>, mariadb: bool) -> String {
    let mut lines = vec!["[client]".to_string()];
    lines.push(format!("host={}", quoted(&route.host)));
    lines.push(format!("port={}", route.port));
    // TCP even to `localhost`, which the tools would otherwise reach through a socket file.
    lines.push("protocol=TCP".into());
    if !connection.username.is_empty() {
        lines.push(format!("user={}", quoted(&connection.username)));
    }
    if let Some(password) = password {
        lines.push(format!("password={}", quoted(password)));
    }
    lines.extend(ssl_options(&connection.ssl, mariadb));
    lines.push(String::new());
    lines.join("\n")
}

/// The SSL settings for `ssl`, as `drivers::mysql` connects. MariaDB's tools
/// have no `ssl-mode`: they encrypt with `ssl` and check the server with
/// `ssl-verify-server-cert`, which checks its name too.
fn ssl_options(ssl: &Ssl, mariadb: bool) -> Vec<String> {
    let mut lines: Vec<String> = if mariadb {
        match ssl.mode {
            SslMode::Disable => vec!["skip-ssl".into()],
            SslMode::Prefer => vec!["skip-ssl-verify-server-cert".into()],
            SslMode::Require => vec!["ssl".into(), "skip-ssl-verify-server-cert".into()],
            SslMode::VerifyCa | SslMode::VerifyFull => vec!["ssl".into(), "ssl-verify-server-cert".into()],
        }
    } else {
        let mode = match ssl.mode {
            SslMode::Disable => "DISABLED",
            SslMode::Prefer => "PREFERRED",
            SslMode::Require => "REQUIRED",
            SslMode::VerifyCa => "VERIFY_CA",
            SslMode::VerifyFull => "VERIFY_IDENTITY",
        };
        vec![format!("ssl-mode={mode}")]
    };
    if ssl.mode.encrypts() {
        if !ssl.ca_file.is_empty() {
            lines.push(format!("ssl-ca={}", quoted(&ssl.ca_file)));
        }
        if !ssl.cert_file.is_empty() {
            lines.push(format!("ssl-cert={}", quoted(&ssl.cert_file)));
            lines.push(format!("ssl-key={}", quoted(&ssl.key_file)));
        }
    }
    lines
}

/// `value` in double quotes, as option files read it: a `#` inside is not a
/// comment, and backslashes, quotes and line breaks are escaped.
fn quoted(value: &str) -> String {
    let mut text = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => text.push_str("\\\\"),
            '"' => text.push_str("\\\""),
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            '\t' => text.push_str("\\t"),
            c => text.push(c),
        }
    }
    text.push('"');
    text
}

/// mysqldump's options for `options`, writing to `file`; the database goes after them.
fn dump_args(options: &MysqlOptions, tool: &Tool, server: Engine, file: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> =
        ["--verbose", "--default-character-set=utf8mb4", "--hex-blob", "--no-tablespaces"].map(OsString::from).into();
    let mut result = OsString::from("--result-file=");
    result.push(file);
    args.push(result);
    let mut add = |arg: &str| args.push(arg.into());
    if options.single_transaction {
        add("--single-transaction");
    }
    match options.content {
        Content::All => {}
        Content::Structure => add("--no-data"),
        Content::Data => add("--no-create-info"),
    }
    if options.content == Content::Data {
        // Triggers, routines and events are structure.
        add("--skip-triggers");
    } else {
        if options.routines {
            add("--routines");
            add("--triggers");
            add("--events");
        } else {
            add("--skip-triggers");
        }
        add(if options.add_drop_table { "--add-drop-table" } else { "--skip-add-drop-table" });
    }
    if !tool.mariadb {
        // GTID statements in the file would fail a restore into another server.
        add("--set-gtid-purged=OFF");
        // MySQL 8's mysqldump asks for column statistics MariaDB doesn't keep.
        if server == Engine::Mariadb && tool.version >= (8, 0) {
            add("--column-statistics=0");
        }
    }
    args
}

/// mysql's options to run a script from its input into `database`.
fn restore_args(database: &str) -> Vec<String> {
    vec!["--default-character-set=utf8mb4".into(), format!("--database={database}")]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::drivers::mysql::tests::{PASSWORD, local, mariadb};

    const ALL: MysqlOptions =
        MysqlOptions { content: Content::All, add_drop_table: true, single_transaction: true, routines: true };

    fn tool(mariadb: bool) -> Tool {
        Tool { path: PathBuf::from("/usr/bin/mysqldump"), version: (8, 4), custom: false, mariadb }
    }

    fn route() -> Route {
        Route { host: "127.0.0.1".into(), port: 41234 }
    }

    fn strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter().map(|arg| arg.into_string().unwrap()).collect()
    }

    #[test]
    fn the_option_file_holds_the_route_the_user_and_the_password() {
        let connection = Connection { host: "db.internal".into(), port: 3306, ..local() };
        let text = option_file(&connection, &route(), Some("it's \"a\" #\\ secret"), false);
        assert_eq!(
            text,
            "[client]\nhost=\"127.0.0.1\"\nport=41234\nprotocol=TCP\nuser=\"root\"\n\
             password=\"it's \\\"a\\\" #\\\\ secret\"\nssl-mode=PREFERRED\n"
        );
        let without = option_file(&Connection { username: String::new(), ..connection }, &route(), None, false);
        assert!(!without.contains("user=") && !without.contains("password="), "{without}");
    }

    #[test]
    fn ssl_settings_map_for_mysql_and_mariadb_tools() {
        let ssl = |mode| Ssl { mode, ca_file: "/ca.pem".into(), cert_file: "/c.pem".into(), key_file: "/k.pem".into() };
        assert_eq!(
            ssl_options(&ssl(SslMode::VerifyFull), false),
            ["ssl-mode=VERIFY_IDENTITY", "ssl-ca=\"/ca.pem\"", "ssl-cert=\"/c.pem\"", "ssl-key=\"/k.pem\""]
        );
        assert_eq!(ssl_options(&ssl(SslMode::VerifyCa), false)[0], "ssl-mode=VERIFY_CA");
        assert_eq!(ssl_options(&ssl(SslMode::Require), false)[0], "ssl-mode=REQUIRED");
        assert_eq!(ssl_options(&ssl(SslMode::Disable), false), ["ssl-mode=DISABLED"], "no certificates unencrypted");
        assert_eq!(ssl_options(&ssl(SslMode::Disable), true), ["skip-ssl"]);
        assert_eq!(
            ssl_options(&Ssl { mode: SslMode::Prefer, ..Ssl::default() }, true),
            ["skip-ssl-verify-server-cert"]
        );
        assert_eq!(
            ssl_options(&ssl(SslMode::Require), true),
            ["ssl", "skip-ssl-verify-server-cert", "ssl-ca=\"/ca.pem\"", "ssl-cert=\"/c.pem\"", "ssl-key=\"/k.pem\""]
        );
        assert_eq!(ssl_options(&ssl(SslMode::VerifyFull), true)[..2], ["ssl", "ssl-verify-server-cert"]);
    }

    #[test]
    fn the_command_reads_only_the_option_file_and_no_password() {
        let defaults = SecretFile::new(&option_file(&local(), &route(), Some("s3cret"), false), ".cnf").unwrap();
        let mut command = command(&tool(false), &defaults);
        command.args(dump_args(&ALL, &tool(false), Engine::Mysql, Path::new("/out/shop.sql.dbforge-partial")));
        command.arg("shop");
        let std = command.as_std();
        let args: Vec<String> = std.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert_eq!(args[0], format!("--defaults-file={}", defaults.path().display()), "first, as the tools require");
        assert!(!args.iter().any(|arg| arg.contains("s3cret")), "{args:?}");
        assert!(!std.get_envs().any(|(_, value)| value.is_some_and(|v| v.to_string_lossy().contains("s3cret"))));
        let login = std.get_envs().find(|(name, _)| *name == "MYSQL_TEST_LOGIN_FILE").unwrap().1.unwrap();
        assert!(!Path::new(login).exists(), "no login paths are read");
        assert_eq!(args.last().unwrap(), "shop");
    }

    #[test]
    fn dump_options_become_mysqldump_arguments() {
        let file = Path::new("/out/shop.sql.dbforge-partial");
        let args = strings(dump_args(&ALL, &tool(false), Engine::Mysql, file));
        for expected in [
            "--result-file=/out/shop.sql.dbforge-partial",
            "--single-transaction",
            "--routines",
            "--triggers",
            "--events",
            "--add-drop-table",
            "--hex-blob",
            "--no-tablespaces",
            "--set-gtid-purged=OFF",
        ] {
            assert!(args.contains(&expected.to_string()), "{expected} in {args:?}");
        }
        assert!(!args.iter().any(|arg| arg == "--databases" || arg == "--no-data" || arg == "--no-create-info"));

        let structure = MysqlOptions { content: Content::Structure, add_drop_table: false, routines: false, ..ALL };
        let args = strings(dump_args(&structure, &tool(false), Engine::Mysql, file));
        for expected in ["--no-data", "--skip-add-drop-table", "--skip-triggers"] {
            assert!(args.contains(&expected.to_string()), "{expected} in {args:?}");
        }
        assert!(!args.contains(&"--routines".to_string()));

        let data = MysqlOptions { content: Content::Data, single_transaction: false, ..ALL };
        let args = strings(dump_args(&data, &tool(false), Engine::Mysql, file));
        assert!(args.contains(&"--no-create-info".into()) && args.contains(&"--skip-triggers".into()), "{args:?}");
        assert!(!args.iter().any(|arg| arg == "--routines" || arg.contains("drop") || arg == "--single-transaction"));
    }

    #[test]
    fn each_tool_gets_only_the_options_it_knows() {
        let file = Path::new("/out/x.sql");
        let mariadb_tool = strings(dump_args(&ALL, &tool(true), Engine::Mariadb, file));
        assert!(
            !mariadb_tool.iter().any(|arg| arg.starts_with("--set-gtid") || arg.starts_with("--column-statistics"))
        );
        let mysql_tool_on_mariadb = strings(dump_args(&ALL, &tool(false), Engine::Mariadb, file));
        assert!(mysql_tool_on_mariadb.contains(&"--column-statistics=0".into()));
        let old = Tool { version: (5, 7), ..tool(false) };
        assert!(!strings(dump_args(&ALL, &old, Engine::Mariadb, file)).iter().any(|a| a.starts_with("--column")));
        assert_eq!(restore_args("my db"), ["--default-character-set=utf8mb4", "--database=my db"]);
    }

    #[tokio::test]
    async fn a_database_named_like_an_option_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let run = Run { tool: Some(&tool(false)), cancel: &cancel };
        let error =
            dump(&local(), None, "--result-file=/etc/x", &dir.path().join("x"), &ALL, run, |_| {}).await.unwrap_err();
        assert!(error.to_string().starts_with("Can't export a database named"), "{error}");
    }

    // Live: MySQL on 53306 and MariaDB on 53307 (see `drivers::mysql::tests`),
    // and their client tools installed. `cargo test -- --ignored`.

    use crate::dump::tools::Flavour;
    use crate::dump::{DumpOptions, RestoreOptions, export, import};
    use sqlx::{AssertSqlSafe, Connection as _, MySqlConnection};
    use tokio_util::sync::CancellationToken;

    async fn tool_for(connection: &Connection, name: &str) -> Tool {
        let flavour = if connection.engine == Engine::Mariadb { Flavour::Mariadb } else { Flavour::Standard };
        tools::find(name, flavour).await.unwrap()
    }

    async fn sql(connection: &Connection, statements: &str) {
        let url = format!("mysql://root:dbforge@127.0.0.1:{}/", connection.port);
        let mut conn = MySqlConnection::connect(&url).await.unwrap();
        sqlx::raw_sql(AssertSqlSafe(statements.to_string())).execute(&mut conn).await.unwrap();
        conn.close().await.unwrap();
    }

    async fn count(connection: &Connection, sql: &str) -> i64 {
        let url = format!("mysql://root:dbforge@127.0.0.1:{}/", connection.port);
        let mut conn = MySqlConnection::connect(&url).await.unwrap();
        let n = sqlx::query_scalar(AssertSqlSafe(sql.to_string())).fetch_one(&mut conn).await.unwrap();
        conn.close().await.unwrap();
        n
    }

    async fn exists(connection: &Connection, name: &str) -> bool {
        mysql::database_exists(connection, PASSWORD, name).await.unwrap()
    }

    async fn export_all(connection: &Connection, password: Option<&str>, database: &str, file: &Path) -> Result<()> {
        let tool = tool_for(connection, "mysqldump").await;
        let run = Run { tool: Some(&tool), cancel: &CancellationToken::new() };
        export(connection, password, database, file, &DumpOptions::Mysql(ALL), run, |_| {}).await.map(|_| ())
    }

    async fn restore_into(
        connection: &Connection,
        database: &str,
        file: &Path,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        let tool = tool_for(connection, "mysql").await;
        let run = Run { tool: Some(&tool), cancel };
        let imported = import(connection, PASSWORD, database, file, &RestoreOptions::default(), run, |_| {}).await?;
        Ok(imported.created)
    }

    async fn round_trip(server: Connection) {
        sql(
            &server,
            "DROP DATABASE IF EXISTS dbforge_dump_source; CREATE DATABASE dbforge_dump_source;
             CREATE TABLE dbforge_dump_source.items (id INT PRIMARY KEY, name TEXT, data BLOB);
             INSERT INTO dbforge_dump_source.items VALUES (1, 'it''s', X'00FF'), (2, NULL, NULL);
             CREATE PROCEDURE dbforge_dump_source.two() SELECT 2;
             DROP DATABASE IF EXISTS dbforge_dump_copy;",
        )
        .await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new folder").join("source.sql");
        export_all(&server, PASSWORD, "dbforge_dump_source", &file).await.unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("CREATE TABLE `items`") && !text.contains("CREATE DATABASE"), "{text}");
        assert!(text.contains("PROCEDURE `two`"), "routines are kept");

        assert!(restore_into(&server, "dbforge_dump_copy", &file, &CancellationToken::new()).await.unwrap());
        assert_eq!(count(&server, "SELECT COUNT(*) FROM dbforge_dump_copy.items").await, 2);
        let blob = "SELECT COUNT(*) FROM dbforge_dump_copy.items WHERE data = X'00FF' AND name = 'it''s'";
        assert_eq!(count(&server, blob).await, 1);
        // Again over it: the file drops what it creates.
        assert!(!restore_into(&server, "dbforge_dump_copy", &file, &CancellationToken::new()).await.unwrap());
        sql(&server, "DROP DATABASE dbforge_dump_source; DROP DATABASE dbforge_dump_copy;").await;
    }

    async fn failures(server: Connection) {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("broken.sql");
        std::fs::write(
            &broken,
            "CREATE TABLE ok (id INT);\nINSERT INTO missing VALUES (1);\nCREATE TABLE never (id INT);\n",
        )
        .unwrap();
        sql(&server, "DROP DATABASE IF EXISTS dbforge_dump_broken").await;
        let error = restore_into(&server, "dbforge_dump_broken", &broken, &CancellationToken::new()).await.unwrap_err();
        assert!(error.to_string().contains("missing"), "{error}");
        assert!(!exists(&server, "dbforge_dump_broken").await, "the database it created is dropped");

        // A wrong password leaves no file, and a file already there as it was.
        let file = dir.path().join("kept.sql");
        std::fs::write(&file, "-- an older dump").unwrap();
        let error = export_all(&server, Some("wrong"), "shop", &file).await.unwrap_err();
        assert!(error.to_string().contains("Access denied"), "{error}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "-- an older dump");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2, "no partial file is left");

        // Stopped before mysql started: nothing is created.
        sql(&server, "DROP DATABASE IF EXISTS dbforge_dump_never").await;
        let stopped = CancellationToken::new();
        stopped.cancel();
        let error = restore_into(&server, "dbforge_dump_never", &broken, &stopped).await.unwrap_err();
        assert_eq!(error.to_string(), "Cancelled.");
        assert!(!exists(&server, "dbforge_dump_never").await);

        // Stopped while it runs: the database it created is dropped.
        let slow = dir.path().join("slow.sql");
        std::fs::write(&slow, "CREATE TABLE slow (id INT);\nSELECT SLEEP(30);\n").unwrap();
        sql(&server, "DROP DATABASE IF EXISTS dbforge_dump_stopped").await;
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            stop.cancel();
        });
        let started = std::time::Instant::now();
        let error = restore_into(&server, "dbforge_dump_stopped", &slow, &cancel).await.unwrap_err();
        assert_eq!(error.to_string(), "Cancelled.");
        assert!(started.elapsed() < std::time::Duration::from_secs(20));
        assert!(!exists(&server, "dbforge_dump_stopped").await);
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306 and its client tools"]
    async fn mysql_a_file_for_another_database_is_refused_before_anything_runs() {
        let server = local();
        sql(
            &server,
            "DROP DATABASE IF EXISTS dbforge_dump_named; CREATE DATABASE dbforge_dump_named;
             CREATE TABLE dbforge_dump_named.items (id INT PRIMARY KEY);
             INSERT INTO dbforge_dump_named.items VALUES (1), (2);
             DROP DATABASE IF EXISTS dbforge_dump_elsewhere;",
        )
        .await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("named.sql");
        let tool = tool_for(&server, "mysqldump").await;
        let defaults = SecretFile::new(
            &option_file(&server, &crate::tunnel::route(&server).await.unwrap(), PASSWORD, false),
            ".cnf",
        )
        .unwrap();
        let mut command = command(&tool, &defaults);
        command.args(dump_args(&ALL, &tool, Engine::Mysql, &file)).args(["--databases", "dbforge_dump_named"]);
        tools::run(command, &CancellationToken::new(), |_| {}).await.unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("USE `dbforge_dump_named`"), "{text}");

        let error =
            restore_into(&server, "dbforge_dump_elsewhere", &file, &CancellationToken::new()).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "This file creates another database (CREATE DATABASE `dbforge_dump_named`), so it would not restore \
             into dbforge_dump_elsewhere. Export it again without --databases, or restore it into `dbforge_dump_named`."
        );
        assert!(!exists(&server, "dbforge_dump_elsewhere").await, "nothing is created");
        assert_eq!(count(&server, "SELECT COUNT(*) FROM dbforge_dump_named.items").await, 2, "the source is untouched");
        sql(&server, "DROP DATABASE dbforge_dump_named").await;
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306 and its client tools"]
    async fn mysql_exports_a_database_and_restores_it_into_a_new_one() {
        round_trip(local()).await;
    }

    #[tokio::test]
    #[ignore = "needs a local MariaDB on port 53307 and its client tools"]
    async fn mariadb_exports_a_database_and_restores_it_into_a_new_one() {
        round_trip(mariadb()).await;
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306 and its client tools"]
    async fn mysql_failed_or_stopped_runs_leave_nothing_behind() {
        failures(local()).await;
    }

    #[tokio::test]
    #[ignore = "needs a local MariaDB on port 53307 and its client tools"]
    async fn mariadb_failed_or_stopped_runs_leave_nothing_behind() {
        failures(mariadb()).await;
    }
}
