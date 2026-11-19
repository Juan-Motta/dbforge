//! Settings › Backup, as the web UI's `features/backup`: DBForge's
//! configuration written to a file (passwords and the MCP token only when
//! asked, in plain text, as the core writes them), or a file read back,
//! showing what it holds and applying the parts ticked, merging: nothing
//! here is deleted. The core does the reading and writing; the window only
//! applies the file's preferences (theme, language, export folder).

mod section;

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};

pub use section::{BackupEvent, BackupSection};

use crate::i18n::Language;
use crate::theme::ThemePreference;

/// The keys a backup keeps the window's preferences under, as the web UI wrote them.
pub mod keys {
    pub const THEME: &str = "theme";
    pub const LANGUAGE: &str = "language";
    pub const EXPORT_FOLDER: &str = "exportFolder";
}

/// The preferences this window keeps, for a backup: the theme and language
/// in use, and the export folder when one was chosen.
pub fn current_preferences(
    theme: ThemePreference,
    language: Language,
    export_folder: Option<&str>,
) -> BTreeMap<String, String> {
    let mut preferences = BTreeMap::from([
        (keys::THEME.to_owned(), theme.as_str().to_owned()),
        (keys::LANGUAGE.to_owned(), language.as_str().to_owned()),
    ]);
    if let Some(folder) = export_folder.filter(|folder| !folder.is_empty()) {
        preferences.insert(keys::EXPORT_FOLDER.to_owned(), folder.to_owned());
    }
    preferences
}

/// A backup's preferences, keeping only values this version understands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KnownPreferences {
    pub theme: Option<ThemePreference>,
    pub language: Option<Language>,
    pub export_folder: Option<String>,
}

pub fn known_preferences(preferences: &BTreeMap<String, String>) -> KnownPreferences {
    let get = |key: &str| preferences.get(key).map(String::as_str);
    let theme = match get(keys::THEME) {
        Some("dark") => Some(ThemePreference::Dark),
        Some("light") => Some(ThemePreference::Light),
        Some("system") => Some(ThemePreference::System),
        _ => None,
    };
    let export_folder = get(keys::EXPORT_FOLDER).map(str::trim).filter(|folder| !folder.is_empty()).map(str::to_owned);
    KnownPreferences { theme, language: get(keys::LANGUAGE).and_then(Language::parse), export_folder }
}

/// `dbforge-backup-2026-11-10.json`.
pub fn backup_file_name(day: NaiveDate) -> String {
    format!("dbforge-backup-{}-{:02}-{:02}.json", day.year(), day.month(), day.day())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect()
    }

    #[test]
    fn keeps_only_values_this_version_understands() {
        assert_eq!(
            known_preferences(&map(&[("theme", "light"), ("language", "es"), ("exportFolder", " /backups ")])),
            KnownPreferences {
                theme: Some(ThemePreference::Light),
                language: Some(Language::Es),
                export_folder: Some("/backups".into())
            }
        );
        assert_eq!(
            known_preferences(&map(&[("theme", "sepia"), ("language", "fr"), ("exportFolder", "  ")])),
            KnownPreferences::default()
        );
    }

    #[test]
    fn names_the_file_after_the_day() {
        assert_eq!(backup_file_name(NaiveDate::from_ymd_opt(2026, 11, 9).unwrap()), "dbforge-backup-2026-11-09.json");
    }

    #[test]
    fn backs_up_the_theme_and_language_in_use_and_a_chosen_export_folder() {
        assert_eq!(
            current_preferences(ThemePreference::Dark, Language::En, None),
            map(&[("theme", "dark"), ("language", "en")])
        );
        assert_eq!(
            current_preferences(ThemePreference::System, Language::Es, Some("/x")),
            map(&[("theme", "system"), ("language", "es"), ("exportFolder", "/x")])
        );
    }
}
