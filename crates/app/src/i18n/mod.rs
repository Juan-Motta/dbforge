//! English and Spanish. Each language is one [`Messages`] value, so a text
//! missing from one fails to compile rather than showing a key at runtime.
//! The language is the user's choice, or the OS's when DBForge speaks it.

mod connection;
mod en;
mod es;
mod query;
mod table;

use gpui::{App, Global};

pub use self::connection::ConnectionMessages;
pub use self::query::{HistoryMessages, QueryMessages, thousands};
pub use self::table::TableMessages;
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
    /// A table's or view's menu: open its rows.
    pub open_data: &'static str,
    pub refresh_connection: &'static str,
    pub copy_connection_url: &'static str,
    pub duplicate_connection: &'static str,
    pub edit_connection: &'static str,
    pub delete_connection: &'static str,
    /// The name a duplicated connection gets.
    pub duplicate_name: fn(&str) -> String,
    pub resize_explorer: &'static str,
    // Toolbar
    pub no_connection: &'static str,
    pub search_everywhere: &'static str,
    pub settings: &'static str,
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
