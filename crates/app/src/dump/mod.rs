//! A whole PostgreSQL database to a .sql file with pg_dump and back with
//! psql, as the former web UI's `features/dump`: the two dialogs a database's menu
//! opens, the folder exports go to and the client tools that run, both also
//! in Settings › Export and import. Restoring into a connection whose tag
//! asks first (PROD) warns on the first press and runs on the second.

mod dialogs;
mod folder;
mod job;
mod settings;
mod tool_row;

use chrono::{Datelike, NaiveDateTime, Timelike};
use dbforge_core::connections::model::Engine;
pub use dbforge_core::dump::Content;

pub use dialogs::{DumpEvent, ExportDatabaseDialog, ImportDatabaseDialog, key_bindings};
pub use folder::ExportFolder;
pub use settings::ExportSettings;
pub use tool_row::ToolRow;

use crate::i18n::DumpMessages;
use crate::ui::callout::Tone;

/// Which of a database's dialogs: writing it to a file, or restoring one into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpMode {
    Export,
    Import,
}

/// Whether `engine`'s databases are dumped and restored with PostgreSQL's tools.
pub fn dumps(engine: Engine) -> bool {
    engine == Engine::Postgres
}

/// What a dump can hold, as the export dialog offers it, with its icon.
pub fn contents(m: &DumpMessages) -> [(Content, &'static str, &'static str, &'static str); 3] {
    [
        (Content::All, m.all_label, m.all_description, "database"),
        (Content::Structure, m.structure_label, m.structure_description, "schema"),
        (Content::Data, m.data_label, m.data_description, "table"),
    ]
}

/// What restoring into `name` will do, given the server's databases (`None` until they are read).
pub fn target_note(name: &str, databases: Option<&[String]>, m: &DumpMessages) -> &'static str {
    let name = name.trim();
    match databases {
        _ if name.is_empty() => m.note_name_target,
        None => m.note_restore_or_create,
        Some(databases) if databases.iter().any(|database| database == name) => m.note_restore_existing,
        Some(_) => m.note_create_then_restore,
    }
}

/// `name` inside `folder`, with the folder's own separator (`\` on Windows).
pub fn join_path(folder: &str, name: &str) -> String {
    let separator = if folder.contains('\\') && !folder.contains('/') { '\\' } else { '/' };
    format!("{}{separator}{name}", folder.trim_end_matches(['\\', '/']))
}

/// The folder used when none was chosen: `DBForge` in the user's Documents.
pub fn default_export_folder(documents: &str) -> String {
    join_path(documents, "DBForge")
}

/// A file name for a database's export, unique to the minute: `shop-2026-11-10-1432.sql`.
pub fn export_file_name(database: &str, at: NaiveDateTime) -> String {
    // Characters a file name can't hold on any system become dashes.
    let safe: String = database
        .chars()
        .map(|c| if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c })
        .collect();
    format!("{safe}-{}-{:02}-{:02}-{:02}{:02}.sql", at.year(), at.month(), at.day(), at.hour(), at.minute())
}

/// The last part of a path, for a result line.
pub fn file_name_of(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// What the box under a dump form says once a run is over, or before one starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub tone: Tone,
    pub title: String,
    pub detail: Option<String>,
    pub mono: bool,
}

/// The core's word for a run the Stop button ended (`jobs::cancelled`).
const CANCELLED: &str = "Cancelled.";

/// How a run that failed ended: stopped by the user, or failed with the tool's message.
pub fn ended(error: &str, stopped: &str, failed: &str) -> Outcome {
    if error == CANCELLED {
        Outcome { tone: Tone::Stopped, title: stopped.into(), detail: None, mono: false }
    } else {
        Outcome { tone: Tone::Failed, title: failed.into(), detail: Some(error.into()), mono: true }
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::i18n::Language;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day).unwrap().and_hms_opt(hour, minute, 0).unwrap()
    }

    #[test]
    fn says_whether_the_database_is_restored_into_or_created_first() {
        let m = &Language::En.messages().dump;
        let databases = ["postgres".to_owned(), "shop".to_owned()];
        assert_eq!(target_note("shop", Some(&databases), m), "Restores into this existing database.");
        assert_eq!(target_note(" shop_copy ", Some(&databases), m), "Creates this database, then restores into it.");
        assert_eq!(target_note("  ", Some(&databases[1..]), m), "Name the database to restore into.");
        assert_eq!(target_note("shop", None, m), "Restores into this database, creating it if it doesn’t exist.");
        let es = &Language::Es.messages().dump;
        assert_eq!(target_note("shop", Some(&databases), es), "Restaura en esta base de datos existente.");
    }

    #[test]
    fn joins_a_file_to_a_folder_with_its_own_separator() {
        assert_eq!(join_path("/Users/ann/Exports/", "shop.sql"), "/Users/ann/Exports/shop.sql");
        assert_eq!(join_path("C:\\Users\\ann\\Exports", "shop.sql"), "C:\\Users\\ann\\Exports\\shop.sql");
        assert_eq!(default_export_folder("/Users/ann/Documents"), "/Users/ann/Documents/DBForge");
    }

    #[test]
    fn names_an_export_after_its_database_and_the_minute_without_characters_files_can_t_hold() {
        assert_eq!(export_file_name("shop", at(2026, 11, 10, 14, 32)), "shop-2026-11-10-1432.sql");
        assert_eq!(export_file_name("a/b:c", at(2026, 1, 2, 3, 4)), "a-b-c-2026-01-02-0304.sql");
    }

    #[test]
    fn tells_a_stopped_run_from_a_failed_one() {
        assert_eq!(
            ended("Cancelled.", "Stopped", "Failed"),
            Outcome { tone: Tone::Stopped, title: "Stopped".into(), detail: None, mono: false }
        );
        let failed = ended("pg_dump: error: connection refused", "Stopped", "Failed");
        assert_eq!(failed.tone, Tone::Failed);
        assert_eq!(failed.detail.as_deref(), Some("pg_dump: error: connection refused"));
        assert!(failed.mono);
    }

    #[test]
    fn dumps_only_postgresql_and_names_the_last_part_of_a_path() {
        assert!(dumps(Engine::Postgres));
        assert!(!dumps(Engine::Mysql));
        assert!(!dumps(Engine::Sqlite));
        assert_eq!(file_name_of("/tmp/out/shop.sql"), "shop.sql");
        assert_eq!(file_name_of("C:\\out\\shop.sql"), "shop.sql");
    }
}
