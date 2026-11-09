//! PostgreSQL's own client tools, as installed on the computer, and running
//! them. A Mac app opened from the Finder doesn't get the terminal's PATH, so
//! the places Homebrew, Postgres.app and Linux packages install them are
//! searched too, and the newest copy wins: pg_dump can only dump servers no
//! newer than itself.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, Result};

/// Where client tools live besides PATH. `*` stands for any version.
const PLACES: [&str; 10] = [
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
}

/// The newest `name` (`pg_dump`, `psql`) installed, or how to install it.
pub async fn find(name: &str) -> Result<Tool> {
    let mut newest: Option<Tool> = None;
    for path in candidates(name) {
        let Ok(output) = Command::new(&path).arg("--version").output().await else { continue };
        let Some(version) = version_of(&String::from_utf8_lossy(&output.stdout)) else { continue };
        if newest.as_ref().is_none_or(|tool| version > tool.version) {
            newest = Some(Tool { path, version });
        }
    }
    newest.ok_or_else(|| {
        AppError::Invalid(format!(
            "{name} isn't installed. Install PostgreSQL's client tools: `brew install libpq` on a Mac, \
             or the postgresql-client package on Linux."
        ))
    })
}

/// Every existing `name` in PATH and the usual places, without repeats.
fn candidates(name: &str) -> Vec<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut directories: Vec<PathBuf> = std::env::split_paths(&path).collect();
    for place in PLACES {
        directories.extend(expand(Path::new(place)));
    }
    let mut found = Vec::new();
    // A relative entry (`.`, or an empty one) would run whatever sits in the current directory.
    for directory in directories.into_iter().filter(|directory| directory.is_absolute()) {
        let tool = directory.join(name);
        if tool.is_file() && !found.contains(&tool) {
            found.push(tool);
        }
    }
    found
}

/// The directories a place stands for: itself, or one per match of its `*`.
fn expand(place: &Path) -> Vec<PathBuf> {
    let text = place.to_string_lossy();
    let Some((before, after)) = text.split_once('*') else { return vec![place.to_path_buf()] };
    // The `*` is inside one path component: `postgresql@*` or a whole `*`.
    let (parent, prefix) = before.rsplit_once('/').unwrap_or(("", before));
    let (suffix, rest) = after.split_once('/').map_or((after, ""), |(suffix, rest)| (suffix, rest));
    let Ok(entries) = std::fs::read_dir(if parent.is_empty() { "/" } else { parent }) else { return Vec::new() };
    let mut matches: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(prefix) && name.ends_with(suffix) && name.len() >= prefix.len() + suffix.len())
        .map(|name| Path::new(parent).join(name).join(rest))
        .collect();
    matches.sort();
    matches
}

/// The version in `pg_dump (PostgreSQL) 18.6 (Homebrew)`, as major and minor:
/// `(18, 6)`; a development build such as `19beta1` or `18devel` has minor 0.
fn version_of(text: &str) -> Option<(u32, u32)> {
    let word = text.split_whitespace().find(|word| word.starts_with(|c: char| c.is_ascii_digit()))?;
    let number = |part: &str| part.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok();
    let mut parts = word.splitn(2, '.');
    let major = number(parts.next()?)?;
    Some((major, parts.next().and_then(number).unwrap_or(0)))
}

pub fn cancelled() -> AppError {
    AppError::Invalid("Cancelled.".into())
}

/// Runs `command`, handing every line it writes (output and errors alike) to
/// `on_line`. `cancel` stops it, even if it came before the tool started. A
/// failure is the tool's own last error lines.
pub async fn run(mut command: Command, cancel: &CancellationToken, mut on_line: impl FnMut(&str)) -> Result<()> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
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

/// The error lines worth reading: the tool's `error:` lines (its progress
/// lines say nothing about the failure), else its last lines.
fn failure(lines: &[String]) -> String {
    let errors: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains("error:") || line.contains("ERROR:") || line.contains("FATAL:"))
        .collect();
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
    fn expands_versioned_places() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["postgresql@16", "postgresql@18", "libpq"] {
            std::fs::create_dir_all(dir.path().join(name).join("bin")).unwrap();
        }
        let place = dir.path().join("postgresql@*/bin");
        let found = expand(&place);
        assert_eq!(found, [dir.path().join("postgresql@16/bin"), dir.path().join("postgresql@18/bin")]);
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
}
