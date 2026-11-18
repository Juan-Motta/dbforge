//! Extensions a plain pg_dump script creates, and a copy of the script without
//! the ones a server doesn't have. A dump of a managed server (Azure, AWS, …)
//! creates the provider's own extensions, which no other server can install,
//! and psql would stop the whole restore at the first of them.
//!
//! pg_dump heads every object with a `-- Name: …; Type: …;` comment, and
//! extensions come before any table's data, so only the part of the script up
//! to its first `COPY … FROM stdin;` is read; the rest is copied as it is.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use tempfile::NamedTempFile;

/// What an object's header says it is.
enum Entry<'a> {
    /// `CREATE EXTENSION`.
    Extension(&'a str),
    /// `COMMENT ON EXTENSION`.
    ExtensionComment(&'a str),
    Other,
}

/// Reads a `-- Name: …; Type: …;` header line; `None` for any other line.
fn entry(line: &str) -> Option<Entry<'_>> {
    let (name, rest) = line.strip_prefix("-- Name: ")?.split_once("; Type: ")?;
    let kind = rest.split(';').next().unwrap_or_default();
    Some(match (kind, name.strip_prefix("EXTENSION ")) {
        ("EXTENSION", _) => Entry::Extension(name),
        ("COMMENT", Some(extension)) => Entry::ExtensionComment(extension),
        _ => Entry::Other,
    })
}

/// Where a table's data starts: everything that creates extensions comes before.
fn starts_data(line: &str) -> bool {
    line.starts_with("COPY ") && line.ends_with(" FROM stdin;")
}

/// A line without its line ending.
fn text(line: &[u8]) -> std::borrow::Cow<'_, str> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    String::from_utf8_lossy(line.strip_suffix(b"\r").unwrap_or(line))
}

/// The extensions the script at `path` creates, in name order.
pub fn requested(path: &Path) -> io::Result<BTreeSet<String>> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut found = BTreeSet::new();
    let mut line = Vec::new();
    while reader.read_until(b'\n', &mut line)? > 0 {
        let current = text(&line);
        if starts_data(&current) {
            break;
        }
        if let Some(Entry::Extension(name)) = entry(&current) {
            found.insert(name.to_string());
        }
        line.clear();
    }
    Ok(found)
}

/// A copy of the script at `path` in a private temporary file, with the
/// statements that create or comment on the `left_out` extensions blanked.
/// Blank lines take their place, so an error's line number still points at
/// the same line of the original file. The copy is deleted when dropped.
pub fn without(path: &Path, left_out: &BTreeSet<String>) -> io::Result<NamedTempFile> {
    let mut copy = NamedTempFile::with_prefix("dbforge-import-")?;
    let mut reader = BufReader::new(File::open(path)?);
    let mut writer = BufWriter::new(copy.as_file_mut());
    let mut skipping = false;
    let mut line = Vec::new();
    while reader.read_until(b'\n', &mut line)? > 0 {
        let current = text(&line);
        if starts_data(&current) {
            writer.write_all(&line)?;
            io::copy(&mut reader, &mut writer)?;
            break;
        }
        if let Some(found) = entry(&current) {
            skipping = match found {
                Entry::Extension(name) | Entry::ExtensionComment(name) => left_out.contains(name),
                Entry::Other => false,
            };
        }
        // Comment lines stay: a header, or one inside a blanked statement, which reads as a comment too.
        if skipping && !current.starts_with("--") {
            writer.write_all(if line.ends_with(b"\n") { b"\n" } else { b"" })?;
        } else {
            writer.write_all(&line)?;
        }
        line.clear();
    }
    writer.flush()?;
    drop(writer);
    Ok(copy)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "\
--
-- PostgreSQL database dump
--

SET client_encoding = 'UTF8';

--
-- Name: azure_cdc; Type: EXTENSION; Schema: -; Owner: -
--

CREATE EXTENSION IF NOT EXISTS azure_cdc WITH SCHEMA pg_catalog;


--
-- Name: EXTENSION azure_cdc; Type: COMMENT; Schema: -; Owner: -
--

COMMENT ON EXTENSION azure_cdc IS 'Azure CDC
-- integration';


--
-- Name: pgcrypto; Type: EXTENSION; Schema: -; Owner: -
--

CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;


--
-- Name: users; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.users (id integer);


--
-- Data for Name: users; Type: TABLE DATA; Schema: public; Owner: -
--

COPY public.users (id) FROM stdin;
-- Name: late; Type: EXTENSION; Schema: -; Owner: -
\\.
";

    fn script() -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(SCRIPT.as_bytes()).unwrap();
        file
    }

    #[test]
    fn finds_the_extensions_created_before_the_data() {
        let names: Vec<String> = requested(script().path()).unwrap().into_iter().collect();
        assert_eq!(names, ["azure_cdc", "pgcrypto"]);
    }

    #[test]
    fn blanks_only_what_creates_or_comments_on_the_left_out_extensions() {
        let file = script();
        let copy = without(file.path(), &BTreeSet::from(["azure_cdc".to_string()])).unwrap();
        let written = std::fs::read_to_string(copy.path()).unwrap();
        let expected = SCRIPT
            .replace("CREATE EXTENSION IF NOT EXISTS azure_cdc WITH SCHEMA pg_catalog;", "")
            .replace("COMMENT ON EXTENSION azure_cdc IS 'Azure CDC\n", "\n");
        assert_eq!(written, expected);
        assert_eq!(written.lines().count(), SCRIPT.lines().count());
    }

    #[test]
    fn copies_the_script_unchanged_when_nothing_is_left_out() {
        let file = script();
        let copy = without(file.path(), &BTreeSet::new()).unwrap();
        assert_eq!(std::fs::read_to_string(copy.path()).unwrap(), SCRIPT);
    }

    #[test]
    fn deletes_the_copy_once_dropped() {
        let file = script();
        let copy = without(file.path(), &BTreeSet::new()).unwrap();
        let path = copy.path().to_path_buf();
        drop(copy);
        assert!(!path.exists());
    }
}
