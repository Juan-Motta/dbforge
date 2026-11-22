//! The texts of saved queries: the Library panel beside a console, saving a
//! console, running one with parameters, and their place in the palette.

/// Saved queries' texts in one language.
pub struct SavedMessages {
    // The Library panel
    pub library: &'static str,
    pub close_library: &'static str,
    pub saved: &'static str,
    pub search_placeholder: &'static str,
    pub this_connection: &'static str,
    pub all: &'static str,
    /// A query's scope: any connection of an engine (`PostgreSQL`).
    pub any_engine_connection: fn(&str) -> String,
    /// A query saved for a connection that no longer exists.
    pub deleted_connection: &'static str,
    /// With nothing saved yet, given the shortcut that saves.
    pub empty: fn(&str) -> String,
    pub none_for_connection: &'static str,
    pub no_match: fn(&str) -> String,
    /// Given how a click that opens a new tab is written (`⌘-click`) and the save shortcut.
    pub hint: fn(&str, &str) -> String,
    pub run: &'static str,
    pub explain: &'static str,
    pub open_in_tab: &'static str,
    pub more: &'static str,
    pub rename: &'static str,
    pub duplicate: &'static str,
    pub delete: &'static str,
    pub never_run: &'static str,
    pub just_now: &'static str,
    pub minutes_ago: fn(i64) -> String,
    pub hours_ago: fn(i64) -> String,
    pub yesterday: &'static str,
    pub days_ago: fn(i64) -> String,
    /// Longer ago: the day, written `2026-01-31`.
    pub on_day: fn(&str) -> String,
    // Deleting and renaming
    pub delete_title: fn(&str) -> String,
    pub delete_note: &'static str,
    pub confirm_delete: &'static str,
    pub deleting: &'static str,
    pub rename_title: &'static str,
    pub cancel: &'static str,
    pub save: &'static str,
    pub saving: &'static str,
    // Saving a console
    pub save_title: &'static str,
    /// The console toolbar's Save tooltip, given its shortcut.
    pub save_tooltip: fn(&str) -> String,
    /// The same once the console is a saved query: its name and the shortcut.
    pub save_changes_tooltip: fn(&str, &str) -> String,
    pub name: &'static str,
    pub name_placeholder: &'static str,
    pub folder: &'static str,
    pub no_folder: &'static str,
    pub new_folder: &'static str,
    pub new_folder_placeholder: &'static str,
    pub choose_folder: &'static str,
    pub available_on: &'static str,
    /// Given the connection's name.
    pub only_here_note: fn(&str) -> String,
    /// Given the engine's name.
    pub any_engine_note: fn(&str) -> String,
    pub description: &'static str,
    pub optional: &'static str,
    pub description_placeholder: &'static str,
    pub parameters_found: &'static str,
    pub asked_when_it_runs: &'static str,
    pub no_parameters: &'static str,
    // Problems
    pub not_found: &'static str,
    /// Given the engine's name.
    pub no_connection_to_run: fn(&str) -> String,
    pub connection_gone: &'static str,
    // Running with parameters
    pub run_title: fn(&str) -> String,
    pub parameters: fn(usize) -> String,
    pub parameter: &'static str,
    pub kind: &'static str,
    pub value: &'static str,
    pub kind_text: &'static str,
    pub kind_number: &'static str,
    pub kind_date: &'static str,
    pub kind_date_time: &'static str,
    pub kind_boolean: &'static str,
    pub kind_null: &'static str,
    pub date_placeholder: &'static str,
    pub date_time_placeholder: &'static str,
    pub boolean_placeholder: &'static str,
    /// The value last used and when.
    pub last_used: fn(&str, &str) -> String,
    pub preview: &'static str,
    pub preview_note: &'static str,
    pub remember_values: &'static str,
    /// The button listing a parameter's last values.
    pub recent_values: &'static str,
    /// How an empty remembered value is listed.
    pub empty_value: &'static str,
    // Why a value can't be written
    pub missing_value: fn(&str) -> String,
    pub not_a_number: fn(&str) -> String,
    pub not_a_date: fn(&str) -> String,
    pub not_a_date_time: fn(&str) -> String,
    pub not_a_boolean: fn(&str) -> String,
    /// A parameter's name and why its value can't be written.
    pub invalid_value: fn(&str, &str) -> String,
    // The palette
    pub saved_queries: &'static str,
    /// A saved query in the palette: its folder (if any) and scope.
    pub palette_detail: fn(Option<&str>, &str) -> String,
}
