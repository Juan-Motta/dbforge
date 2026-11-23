//! The client tools DBForge runs for dumps and restores, as installed on the
//! computer, and running them: PostgreSQL's pg_dump and psql, MySQL's (or
//! MariaDB's) mysqldump and mysql, and MongoDB's mongodump and mongorestore.
//!
//! A Mac app opened from the Finder doesn't get the terminal's PATH, so the
//! places Homebrew, the vendors' installers and Linux packages put them are
//! searched too, and the newest copy wins: pg_dump, for one, can only dump
//! servers no newer than itself. MySQL and MariaDB ship tools of the same
//! names whose version numbers don't compare (MySQL 8.4, MariaDB 11.4), so for
//! those the copy built for the connection's server comes first.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, Result};
use crate::jobs::cancelled;

/// Lines of a failed run's error output shown in its message.
const ERROR_LINES: usize = 6;
/// The latest error-output lines kept while a tool runs, to find those in.
const KEPT_LINES: usize = 200;

/// A client tool found on the computer.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub path: PathBuf,
    /// Major and minor version.
    pub version: (u32, u32),
    /// Chosen by the user in place of the one DBForge finds.
    pub custom: bool,
    /// Built by MariaDB (its version line says so), whose options differ from MySQL's.
    pub mariadb: bool,
}

/// Which build of a tool a connection wants, when two vendors ship one of the same name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    /// The tool's own vendor's (MySQL's for mysqldump and mysql).
    Standard,
    /// MariaDB's, for a MariaDB server.
    Mariadb,
}

/// One tool DBForge runs: the name the user knows it by (and its choice is
/// saved under), the programs that do its job, and where to look for them.
struct Spec {
    name: &'static str,
    /// The program names that do this job, e.g. `mariadb-dump` as well as `mysqldump`.
    programs: &'static [&'static str],
    places: &'static [&'static str],
    install: &'static str,
}

const POSTGRES_INSTALL: &str = "Install PostgreSQL's client tools: `brew install libpq` on a Mac, \
     the postgresql-client package on Linux, or PostgreSQL's installer on Windows.";
const MYSQL_INSTALL: &str = "Install MySQL's client tools: `brew install mysql-client` on a Mac \
     (`brew install mariadb` for MariaDB's), the mysql-client or mariadb-client package on Linux, \
     or MySQL's or MariaDB's installer on Windows.";
const MONGODB_INSTALL: &str = "Install the MongoDB Database Tools: \
     `brew install mongodb/brew/mongodb-database-tools` on a Mac, the mongodb-database-tools package \
     on Linux, or MongoDB's Database Tools installer on Windows.";

/// Where client tools live besides PATH. `*` stands for any version.
#[cfg(not(windows))]
const POSTGRES_PLACES: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/homebrew/opt/libpq/bin",
    "/usr/local/opt/libpq/bin",
    "/opt/homebrew/opt/postgresql@*/bin",
    "/usr/local/opt/postgresql@*/bin",
    "/Applications/Postgres.app/Contents/Versions/*/bin",
    "/Library/PostgreSQL/*/bin",
    "/usr/lib/postgresql/*/bin",
    "/usr/bin",
];

/// Where the PostgreSQL installers for Windows put the client tools.
#[cfg(windows)]
const POSTGRES_PLACES: &[&str] = &["C:/Program Files/PostgreSQL/*/bin", "C:/Program Files (x86)/PostgreSQL/*/bin"];

/// Homebrew's MySQL and MariaDB formulas (keg-only `mysql-client` among them),
/// MySQL's own installer for the Mac, and Linux packages.
#[cfg(not(windows))]
const MYSQL_PLACES: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/homebrew/opt/mysql-client/bin",
    "/opt/homebrew/opt/mysql-client@*/bin",
    "/opt/homebrew/opt/mysql/bin",
    "/opt/homebrew/opt/mysql@*/bin",
    "/opt/homebrew/opt/mariadb/bin",
    "/opt/homebrew/opt/mariadb@*/bin",
    "/usr/local/opt/mysql-client/bin",
    "/usr/local/opt/mysql-client@*/bin",
    "/usr/local/opt/mysql/bin",
    "/usr/local/opt/mysql@*/bin",
    "/usr/local/opt/mariadb/bin",
    "/usr/local/opt/mariadb@*/bin",
    "/usr/local/mysql/bin",
    "/usr/bin",
];

#[cfg(windows)]
const MYSQL_PLACES: &[&str] = &[
    "C:/Program Files/MySQL/MySQL Server */bin",
    "C:/Program Files (x86)/MySQL/MySQL Server */bin",
    "C:/Program Files/MariaDB */bin",
    "C:/Program Files (x86)/MariaDB */bin",
];

#[cfg(not(windows))]
const MONGODB_PLACES: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/homebrew/opt/mongodb-database-tools/bin",
    "/usr/local/opt/mongodb-database-tools/bin",
    "/usr/bin",
];

#[cfg(windows)]
const MONGODB_PLACES: &[&str] = &["C:/Program Files/MongoDB/Tools/*/bin"];

const SPECS: [Spec; 6] = [
    Spec { name: "pg_dump", programs: &["pg_dump"], places: POSTGRES_PLACES, install: POSTGRES_INSTALL },
    Spec { name: "psql", programs: &["psql"], places: POSTGRES_PLACES, install: POSTGRES_INSTALL },
    Spec { name: "mysqldump", programs: &["mysqldump", "mariadb-dump"], places: MYSQL_PLACES, install: MYSQL_INSTALL },
    Spec { name: "mysql", programs: &["mysql", "mariadb"], places: MYSQL_PLACES, install: MYSQL_INSTALL },
    Spec { name: "mongodump", programs: &["mongodump"], places: MONGODB_PLACES, install: MONGODB_INSTALL },
    Spec { name: "mongorestore", programs: &["mongorestore"], places: MONGODB_PLACES, install: MONGODB_INSTALL },
];

fn spec(name: &str) -> Result<&'static Spec> {
    SPECS
        .iter()
        .find(|spec| spec.name == name)
        .ok_or_else(|| AppError::Invalid(format!("{name} isn't a tool DBForge runs.")))
}

/// `name`, when it is one of the tools DBForge runs.
pub fn check_name(name: &str) -> Result<&'static str> {
    spec(name).map(|spec| spec.name)
}

/// What `path` says with `--version`: its version, and whether MariaDB built
/// it; `None` when it doesn't run or isn't a tool.
async fn probe(path: &Path) -> Option<((u32, u32), bool)> {
    let output = Command::new(path).arg("--version").output().await.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Some((version_of(&text)?, text.contains("MariaDB")))
}

/// The tool to run for `name`: the one the user chose (`custom`), else the best found for `flavour`.
pub async fn resolve(name: &str, custom: Option<&Path>, flavour: Flavour) -> Result<Tool> {
    let spec = spec(name)?;
    let Some(path) = custom else { return find(spec.name, flavour).await };
    match probe(path).await {
        Some((version, mariadb)) => Ok(Tool { path: path.to_path_buf(), version, custom: true, mariadb }),
        None => Err(AppError::Invalid(format!(
            "The {name} chosen in DBForge can't be run: {}. Choose another one, or use the one DBForge finds.",
            path.display()
        ))),
    }
}

/// The newest `name` installed, a build of `flavour` before any other, or how to install it.
pub async fn find(name: &str, flavour: Flavour) -> Result<Tool> {
    let spec = spec(name)?;
    let mut best: Option<Tool> = None;
    for path in candidates(spec.programs, spec.places) {
        let Some((version, mariadb)) = probe(&path).await else { continue };
        let tool = Tool { path, version, custom: false, mariadb };
        if best.as_ref().is_none_or(|best| rank(&tool, flavour) > rank(best, flavour)) {
            best = Some(tool);
        }
    }
    best.ok_or_else(|| AppError::Invalid(format!("{name} isn't installed. {}", spec.install)))
}

/// How well `tool` fits: the wanted build first, then the newest.
fn rank(tool: &Tool, flavour: Flavour) -> (bool, (u32, u32)) {
    (tool.mariadb == (flavour == Flavour::Mariadb), tool.version)
}

/// Every existing one of `programs` in PATH and `places`, without repeats.
fn candidates(programs: &[&str], places: &[&str]) -> Vec<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut directories: Vec<PathBuf> = std::env::split_paths(&path).collect();
    for place in places {
        directories.extend(expand(Path::new(place)));
    }
    let mut found = Vec::new();
    // A relative entry (`.`, or an empty one) would run whatever sits in the current directory.
    for directory in directories.into_iter().filter(|directory| directory.is_absolute()) {
        for program in programs {
            // `pg_dump.exe` on Windows.
            let tool = directory.join(format!("{program}{}", std::env::consts::EXE_SUFFIX));
            if tool.is_file() && !found.contains(&tool) {
                found.push(tool);
            }
        }
    }
    found
}

/// The directories a place stands for: itself, or one per match of its `*`.
fn expand(place: &Path) -> Vec<PathBuf> {
    // The `*` is inside one path component: `postgresql@*`, `MySQL Server *` or a whole `*`.
    // Components, not `/`, so Windows paths (`C:\Program Files\PostgreSQL\*\bin`) split the same way.
    let components: Vec<_> = place.components().collect();
    let Some(at) = components.iter().position(|part| part.as_os_str().to_string_lossy().contains('*')) else {
        return vec![place.to_path_buf()];
    };
    let parent: PathBuf = components[..at].iter().collect();
    let rest: PathBuf = components[at + 1..].iter().collect();
    let pattern = components[at].as_os_str().to_string_lossy();
    let (prefix, suffix) = pattern.split_once('*').expect("checked");
    let Ok(entries) = std::fs::read_dir(&parent) else { return Vec::new() };
    let mut matches: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(prefix) && name.ends_with(suffix) && name.len() >= prefix.len() + suffix.len())
        .map(|name| parent.join(name).join(&rest))
        .collect();
    matches.sort();
    matches
}

/// The version a tool's `--version` reports, as major and minor:
/// `pg_dump (PostgreSQL) 18.6 (Homebrew)` is `(18, 6)`, and a development
/// build such as `19beta1` or `18devel` has minor 0; `mysqldump  Ver 8.4.3 for
/// macos15` is `(8, 4)`; `mariadb-dump from 11.4.2-MariaDB, client 10.19` is
/// `(11, 4)`; `mongodump version: 100.10.0` is `(100, 10)`. Older MySQL and
/// MariaDB tools give their own version first and the server's after
/// `Distrib` (`mysqldump  Ver 10.19 Distrib 10.6.16-MariaDB`): that one counts.
fn version_of(text: &str) -> Option<(u32, u32)> {
    let mut words = text.split_whitespace();
    let word = match text.split_whitespace().position(|word| word == "Distrib") {
        Some(at) => words.nth(at + 1)?,
        None => words.find(|word| word.starts_with(|c: char| c.is_ascii_digit()))?,
    };
    let number = |part: &str| part.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok();
    let mut parts = word.splitn(2, '.');
    let major = number(parts.next()?)?;
    Some((major, parts.next().and_then(number).unwrap_or(0)))
}

/// Runs `command`, handing every line it writes (output and errors alike) to
/// `on_line`. `cancel` stops it, even if it came before the tool started. A
/// failure is the tool's own last error lines.
pub async fn run(command: Command, cancel: &CancellationToken, on_line: impl FnMut(&str)) -> Result<()> {
    run_with_input(command, Stdio::null(), cancel, on_line).await
}

/// [`run`], with `input` (a file, say) as the tool's standard input.
pub async fn run_with_input(
    mut command: Command,
    input: Stdio,
    cancel: &CancellationToken,
    mut on_line: impl FnMut(&str),
) -> Result<()> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    command.stdin(input).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| AppError::Invalid(format!("Could not start: {error}")))?;
    let mut stdout = BufReader::new(child.stdout.take().expect("piped")).lines();
    let mut stderr = BufReader::new(child.stderr.take().expect("piped")).lines();
    let mut errors: Vec<String> = Vec::new();
    let (mut out_done, mut err_done) = (false, false);
    while !(out_done && err_done) {
        tokio::select! {
            line = stdout.next_line(), if !out_done => match line {
                Ok(Some(line)) => on_line(&line),
                _ => out_done = true,
            },
            line = stderr.next_line(), if !err_done => match line {
                Ok(Some(line)) => {
                    on_line(&line);
                    errors.push(line);
                    if errors.len() > KEPT_LINES {
                        errors.remove(0);
                    }
                }
                _ => err_done = true,
            },
            () = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err(cancelled());
            }
        }
    }
    let status = child.wait().await.map_err(|error| AppError::Invalid(error.to_string()))?;
    if status.success() {
        return Ok(());
    }
    Err(AppError::Invalid(failure(&errors)))
}

/// The error lines worth reading: the tool's error lines (its progress lines
/// say nothing about the failure), else its last lines. pg_dump and psql write
/// `error:` and `ERROR:`, mysqldump `Got error:`, mysql `ERROR 1146 (42S02)`,
/// and the MongoDB tools `Failed:`.
fn failure(lines: &[String]) -> String {
    let is_error = |line: &str| {
        line.contains("error:")
            || line.contains("ERROR:")
            || line.contains("FATAL:")
            || line.contains("Failed:")
            || line.starts_with("ERROR ")
    };
    let errors: Vec<&String> = lines.iter().filter(|line| is_error(line)).collect();
    let chosen: Vec<&String> = if errors.is_empty() {
        lines.iter().rev().take(ERROR_LINES).rev().collect()
    } else {
        errors.into_iter().rev().take(ERROR_LINES).rev().collect()
    };
    let text = chosen.iter().map(|line| line.trim()).collect::<Vec<_>>().join("\n");
    if text.is_empty() { "The tool stopped with an error and said nothing more.".into() } else { text }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_versions() {
        assert_eq!(version_of("pg_dump (PostgreSQL) 18.6 (Homebrew)"), Some((18, 6)));
        assert_eq!(version_of("psql (PostgreSQL) 9.6.24"), Some((9, 6)));
        assert_eq!(version_of("pg_dump (PostgreSQL) 19beta1"), Some((19, 0)));
        assert_eq!(version_of("pg_dump (PostgreSQL) 18devel"), Some((18, 0)));
        assert_eq!(version_of("nothing here"), None);
        assert!(version_of("pg_dump (PostgreSQL) 18.10").unwrap() > version_of("pg_dump (PostgreSQL) 18.6").unwrap());
    }

    #[test]
    fn reads_the_versions_of_the_mysql_mariadb_and_mongodb_tools() {
        assert_eq!(version_of("mysqldump  Ver 8.4.3 for macos15.1 on arm64 (Homebrew)"), Some((8, 4)));
        assert_eq!(version_of("mysql  Ver 9.1.0 for Linux on x86_64 (MySQL Community Server - GPL)"), Some((9, 1)));
        assert_eq!(version_of("mariadb-dump from 11.4.2-MariaDB, client 10.19 for osx10.19 (arm64)"), Some((11, 4)));
        assert_eq!(version_of("mariadb from 11.4.2-MariaDB, client 15.2 for Linux (x86_64)"), Some((11, 4)));
        assert_eq!(
            version_of("mysqldump  Ver 10.19 Distrib 10.6.16-MariaDB, for debian-linux-gnu (x86_64)"),
            Some((10, 6))
        );
        assert_eq!(version_of("mysqldump  Ver 10.13 Distrib 5.7.44, for Linux (x86_64)"), Some((5, 7)));
        let mongodump = "mongodump version: 100.10.0\ngit version: 6d4f001be3fcf673a7a1c3b3d2dbd3b2d4b4b4b4\n\
                         Go version: go1.21.12\n   os: darwin\n   arch: arm64\n   compiler: gc\n";
        assert_eq!(version_of(mongodump), Some((100, 10)));
        assert_eq!(version_of("mongorestore version: 100.9.4"), Some((100, 9)));
    }

    /// A program that prints `version` for `--version`.
    #[cfg(unix)]
    fn fake(dir: &Path, name: &str, version: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\necho '{version}'\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_chosen_mariadb_tool_is_known_as_one() {
        let dir = tempfile::tempdir().unwrap();
        let mariadb = fake(dir.path(), "mariadb-dump", "mariadb-dump from 11.4.2-MariaDB, client 10.19 for osx10.19");
        let tool = resolve("mysqldump", Some(&mariadb), Flavour::Standard).await.unwrap();
        assert_eq!((tool.version, tool.custom, tool.mariadb), ((11, 4), true, true));
        let mysql = fake(dir.path(), "mysqldump", "mysqldump  Ver 8.4.3 for macos15.1 on arm64 (Homebrew)");
        let tool = resolve("mysqldump", Some(&mysql), Flavour::Mariadb).await.unwrap();
        assert_eq!((tool.version, tool.mariadb), ((8, 4), false));
    }

    #[test]
    fn the_build_for_the_server_ranks_before_a_newer_one() {
        let tool = |version, mariadb| Tool { path: PathBuf::new(), version, custom: false, mariadb };
        let (mysql, mariadb) = (tool((8, 4), false), tool((11, 4), true));
        assert!(rank(&mysql, Flavour::Standard) > rank(&mariadb, Flavour::Standard));
        assert!(rank(&mariadb, Flavour::Mariadb) > rank(&mysql, Flavour::Mariadb));
        assert!(rank(&tool((9, 1), false), Flavour::Standard) > rank(&mysql, Flavour::Standard));
    }

    #[test]
    fn every_tool_has_its_programs_and_places() {
        for name in ["pg_dump", "psql", "mysqldump", "mysql", "mongodump", "mongorestore"] {
            let spec = spec(name).unwrap();
            assert!(!spec.programs.is_empty() && !spec.places.is_empty(), "{name}");
        }
        assert_eq!(spec("mysqldump").unwrap().programs, ["mysqldump", "mariadb-dump"]);
        assert_eq!(spec("mysql").unwrap().programs, ["mysql", "mariadb"]);
        #[cfg(not(windows))]
        {
            assert!(MYSQL_PLACES.contains(&"/opt/homebrew/opt/mysql-client/bin"));
            assert!(MYSQL_PLACES.contains(&"/opt/homebrew/opt/mysql-client@*/bin"));
            assert!(MYSQL_PLACES.contains(&"/opt/homebrew/opt/mariadb/bin"));
            assert!(MONGODB_PLACES.contains(&"/opt/homebrew/opt/mongodb-database-tools/bin"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn candidates_include_every_program_of_a_tool() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("mariadb").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let mariadb = fake(&bin, "mariadb-dump", "x");
        let mysql = fake(&bin, "mysqldump", "x");
        let place = dir.path().join("maria*").join("bin").display().to_string();
        let found = candidates(spec("mysqldump").unwrap().programs, &[place.as_str()]);
        assert!(found.contains(&mysql) && found.contains(&mariadb), "{found:?}");
    }

    #[test]
    fn a_missing_tool_says_what_to_install() {
        assert!(spec("mongodump").unwrap().install.contains("mongodb-database-tools"));
        assert!(spec("mysqldump").unwrap().install.contains("mysql-client"));
        assert!(spec("psql").unwrap().install.contains("libpq"));
        assert!(check_name("rm").is_err());
        assert_eq!(check_name("mongorestore").unwrap(), "mongorestore");
    }

    #[test]
    fn expands_versioned_places() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["postgresql@16", "postgresql@18", "libpq", "MySQL Server 8.4"] {
            std::fs::create_dir_all(dir.path().join(name).join("bin")).unwrap();
        }
        let place = dir.path().join("postgresql@*/bin");
        let found = expand(&place);
        assert_eq!(found, [dir.path().join("postgresql@16/bin"), dir.path().join("postgresql@18/bin")]);
        assert_eq!(expand(&dir.path().join("MySQL Server */bin")), [dir.path().join("MySQL Server 8.4/bin")]);
        assert_eq!(expand(Path::new("/no/such/place@*/bin")), Vec::<PathBuf>::new());
        assert_eq!(expand(Path::new("/usr/bin")), [PathBuf::from("/usr/bin")]);
    }

    #[test]
    fn failures_keep_the_error_lines() {
        let lines: Vec<String> = [
            "pg_dump: dumping contents of table \"public.a\"",
            "pg_dump: error: query failed: ERROR:  permission denied for table b",
            "pg_dump: detail: Query was: LOCK TABLE b",
        ]
        .map(String::from)
        .into();
        assert_eq!(failure(&lines), "pg_dump: error: query failed: ERROR:  permission denied for table b");
        assert_eq!(failure(&["just", "talk"].map(String::from)), "just\ntalk");
        let mysql =
            ["mysql: [Warning] Using a password", "ERROR 1146 (42S02) at line 2: Table 'x.missing' doesn't exist"];
        assert_eq!(failure(&mysql.map(String::from)), mysql[1]);
        let mongo =
            ["2026-11-22T10:00:00.000+0000\twriting a.b", "2026-11-22T10:00:00.000+0000\tFailed: can't connect"];
        assert_eq!(failure(&mongo.map(String::from)), "2026-11-22T10:00:00.000+0000\tFailed: can't connect");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runs_streams_lines_and_cancels() {
        let mut seen = Vec::new();
        let mut command = Command::new("sh");
        command.args(["-c", "echo one; echo two >&2; exit 0"]);
        run(command, &CancellationToken::new(), |line| seen.push(line.to_string())).await.unwrap();
        seen.sort();
        assert_eq!(seen, ["one", "two"]);

        let mut command = Command::new("sh");
        command.args(["-c", "echo 'psql: error: no such file' >&2; exit 2"]);
        let error = run(command, &CancellationToken::new(), |_| {}).await.unwrap_err();
        assert_eq!(error.to_string(), "psql: error: no such file");

        // A cancel that came before the tool started still stops it.
        let early = CancellationToken::new();
        early.cancel();
        let mut command = Command::new("sh");
        command.args(["-c", "echo ran"]);
        let mut ran = false;
        assert_eq!(run(command, &early, |_| ran = true).await.unwrap_err().to_string(), "Cancelled.");
        assert!(!ran);

        let cancel = CancellationToken::new();
        let waker = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            waker.cancel();
        });
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5"]);
        let started = std::time::Instant::now();
        let error = run(command, &cancel, |_| {}).await.unwrap_err();
        assert_eq!(error.to_string(), "Cancelled.");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_file_can_be_the_tools_input() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("in.sql");
        std::fs::write(&file, "SELECT 1;\n").unwrap();
        let mut seen = Vec::new();
        let mut command = Command::new("sh");
        command.args(["-c", "cat"]);
        let input = Stdio::from(std::fs::File::open(&file).unwrap());
        run_with_input(command, input, &CancellationToken::new(), |line| seen.push(line.to_string())).await.unwrap();
        assert_eq!(seen, ["SELECT 1;"]);
    }

    #[tokio::test]
    async fn a_chosen_tool_is_used_or_refused_with_its_path() {
        let found = find("pg_dump", Flavour::Standard).await.unwrap();
        let chosen = resolve("pg_dump", Some(&found.path), Flavour::Standard).await.unwrap();
        assert_eq!((chosen.path, chosen.version, chosen.custom), (found.path.clone(), found.version, true));
        let error =
            resolve("pg_dump", Some(Path::new("/nowhere/pg_dump")), Flavour::Standard).await.unwrap_err().to_string();
        assert!(error.contains("/nowhere/pg_dump") && error.contains("can't be run"), "{error}");
        assert!(!resolve("pg_dump", None, Flavour::Standard).await.unwrap().custom);
        assert!(check_name("rm").is_err());
    }
}
