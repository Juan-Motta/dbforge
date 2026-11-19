//! English and Spanish. Each language is one [`Messages`] value, so a text
//! missing from one fails to compile rather than showing a key at runtime.
//! The language is the user's choice, or the OS's when DBForge speaks it.

mod admin;
mod connection;
mod diagram;
mod editor;
mod en;
mod es;
mod keyspace;
mod mcp;
mod palette;
mod query;
mod settings;
mod statements;
mod structure;
mod table;
mod transfer;
mod updates;

use gpui::{App, BorrowAppContext, Global};

pub use self::admin::AdminMessages;
pub use self::connection::ConnectionMessages;
pub use self::diagram::DiagramMessages;
pub use self::editor::EditorMessages;
pub use self::keyspace::KeyspaceMessages;
pub use self::mcp::McpMessages;
pub use self::palette::PaletteMessages;
pub use self::query::{HistoryMessages, PlanMessages, QueryMessages, thousands};
pub use self::settings::{SettingsMessages, TagsMessages};
pub use self::statements::StatementMessages;
pub use self::structure::StructureMessages;
pub use self::table::TableMessages;
pub use self::transfer::{BackupMessages, DumpMessages, ExportMessages, ImportMessages};
pub use self::updates::UpdateMessages;
use crate::preferences::{Preferences, keys};

/// The languages DBForge speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    En,
    Es,
}

impl Language {
    /// Each language named in itself, as the settings list them.
    pub const ALL: [(Language, &'static str); 2] = [(Language::En, "English"), (Language::Es, "Español")];

    /// A saved choice, if it is one DBForge speaks.
    pub fn parse(saved: &str) -> Option<Self> {
        match saved {
            "en" => Some(Self::En),
            "es" => Some(Self::Es),
            _ => None,
        }
    }

    /// As it is saved under [`keys::LANGUAGE`].
    pub fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Es => "es",
        }
    }

    /// The OS's first preferred language when DBForge speaks it, otherwise English.
    pub fn system(locales: &[String]) -> Self {
        match locales.first() {
            Some(locale) if locale.to_lowercase().starts_with("es") => Self::Es,
            _ => Self::En,
        }
    }

    /// The saved choice; the OS's language when nothing (or something unknown) was saved.
    pub fn choose(saved: Option<&str>, locales: &[String]) -> Self {
        saved.and_then(Self::parse).unwrap_or_else(|| Self::system(locales))
    }

    /// Saves it as the user's choice; the window, observing the
    /// preferences, speaks it at once.
    pub fn save(self, cx: &mut App) {
        cx.update_global::<Preferences, _>(|preferences, _| {
            preferences.set(keys::LANGUAGE, Some(self.as_str().into()))
        });
    }

    pub fn messages(self) -> &'static Messages {
        match self {
            Self::En => &en::MESSAGES,
            Self::Es => &es::MESSAGES,
        }
    }
}

/// The window's texts in one language.
pub struct Messages {
    // Explorer
    pub connections: &'static str,
    pub new_connection: &'static str,
    pub refresh: &'static str,
    pub filter_placeholder: &'static str,
    pub no_connections: &'static str,
    pub collapse_all: &'static str,
    /// Under a node whose listing came back empty.
    pub no_objects: &'static str,
    /// When the filter hides every node.
    pub no_matches: fn(&str) -> String,
    /// The folders tables and views are grouped in.
    pub tables: &'static str,
    pub views: &'static str,
    pub copy_name: &'static str,
    /// A MongoDB collection's menu: copy the mongosh that reads its first documents.
    pub copy_find: &'static str,
    /// A MongoDB collection's menu: copy the mongosh that counts its documents.
    pub copy_count: &'static str,
    /// A table's or view's menu: open its rows.
    pub open_data: &'static str,
    /// A table's or view's menu: open its structure.
    pub view_structure: &'static str,
    /// A schema's (or a database that is its own schema) menu: open its ER diagram.
    pub show_diagram: &'static str,
    pub refresh_connection: &'static str,
    pub copy_connection_url: &'static str,
    pub duplicate_connection: &'static str,
    pub edit_connection: &'static str,
    pub delete_connection: &'static str,
    /// The name a duplicated connection gets.
    pub duplicate_name: fn(&str) -> String,
    pub resize_explorer: &'static str,
    /// A table's or view's menu: export its rows.
    pub export_menu: &'static str,
    /// A table's menu: import a file's rows into it.
    pub import_menu: &'static str,
    /// A database's menu, where the engine has pg_dump and psql: dump it to a file, and restore one.
    pub export_database: &'static str,
    pub import_sql_file: &'static str,
    // Toolbar
    pub no_connection: &'static str,
    pub search_everywhere: &'static str,
    pub settings: &'static str,
    /// The toolbar's toggles, each with the shortcut that does the same.
    pub hide_explorer: fn(&str) -> String,
    pub show_explorer: fn(&str) -> String,
    pub show_statements: fn(&str) -> String,
    pub hide_statements: fn(&str) -> String,
    // Main area with no tab open: before, the shortcut's name, after.
    pub new_query: &'static str,
    pub empty_before: &'static str,
    pub empty_after: &'static str,
    // Settings files that couldn't be read at start
    pub set_aside_title: &'static str,
    pub set_aside_note: &'static str,
    pub set_aside_ok: &'static str,
    /// The connection dialog and deleting a connection.
    pub connection: ConnectionMessages,
    /// The query console, its results and the history.
    pub query: QueryMessages,
    /// A table's tab: its rows, their pending changes, the cell inspector.
    pub table: TableMessages,
    /// A table's structure: its columns, indexes, keys, triggers and DDL.
    pub structure: StructureMessages,
    /// An ER diagram.
    pub diagram: DiagramMessages,
    /// The Redis key browser.
    pub keyspace: KeyspaceMessages,
    /// The command palette and its commands.
    pub palette: PaletteMessages,
    /// The statement console.
    pub statements: StatementMessages,
    /// The settings dialog.
    pub settings_dialog: SettingsMessages,
    /// The settings' list of tags.
    pub tags: TagsMessages,
    /// Creating databases, roles and users, and dropping databases.
    pub admin: AdminMessages,
    /// Exporting rows to a file or the clipboard.
    pub export: ExportMessages,
    /// Importing a file's rows into a table.
    pub import: ImportMessages,
    /// A whole database to a .sql file with pg_dump and back with psql, and where exports go.
    pub dump: DumpMessages,
    /// The settings' backup of DBForge's configuration.
    pub backup: BackupMessages,
    /// Settings › Agents (MCP) and the dialog approving an agent's change.
    pub mcp: McpMessages,
    /// The update dialog.
    pub updates: UpdateMessages,
    /// The code editor: its find and replace bar.
    pub editor: EditorMessages,
}

/// The language in use, a GPUI global ([`Translate`]).
pub struct Locale(pub Language);

impl Global for Locale {}

impl Locale {
    /// Sets the language the preferences choose; whether it changed.
    pub fn sync(cx: &mut App) -> bool {
        let locales: Vec<String> = sys_locale::get_locales().collect();
        let language = Language::choose(Preferences::global(cx).get(keys::LANGUAGE), &locales);
        if cx.try_global::<Locale>().is_some_and(|locale| locale.0 == language) {
            return false;
        }
        cx.set_global(Locale(language));
        true
    }
}

/// The texts in the language in use, from any context.
pub trait Translate {
    fn messages(&self) -> &'static Messages;
}

impl Translate for App {
    fn messages(&self) -> &'static Messages {
        self.global::<Locale>().0.messages()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locales(list: &[&str]) -> Vec<String> {
        list.iter().map(|locale| locale.to_string()).collect()
    }

    #[test]
    fn speaks_the_os_s_first_language_when_it_can() {
        assert_eq!(Language::system(&locales(&["es-CO", "en-US"])), Language::Es);
        assert_eq!(Language::system(&locales(&["ES"])), Language::Es);
        assert_eq!(Language::system(&locales(&["en-US", "es-ES"])), Language::En, "only the first counts");
        assert_eq!(Language::system(&locales(&["fr-FR"])), Language::En);
        assert_eq!(Language::system(&[]), Language::En);
    }

    #[test]
    fn a_saved_choice_wins_and_an_unknown_one_falls_back_to_the_os() {
        assert_eq!(Language::choose(Some("en"), &locales(&["es-MX"])), Language::En);
        assert_eq!(Language::choose(Some("es"), &locales(&["en-US"])), Language::Es);
        assert_eq!(Language::choose(Some("de"), &locales(&["es-MX"])), Language::Es);
        assert_eq!(Language::choose(None, &locales(&["en-GB"])), Language::En);
        for (language, _) in Language::ALL {
            assert_eq!(Language::parse(language.as_str()), Some(language));
        }
    }

    #[test]
    fn each_language_has_its_own_texts() {
        assert_eq!(Language::En.messages().new_query, "New query");
        assert_eq!(Language::Es.messages().new_query, "Nueva consulta");
    }
}
