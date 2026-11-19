//! The texts of the command palette (⌘K) and of the commands it runs.

/// The palette's texts in one language.
pub struct PaletteMessages {
    pub select_connection: &'static str,
    pub loading_tables: &'static str,
    pub load_tables_failed: fn(&str) -> String,
    pub no_tables: &'static str,
    pub search_placeholder: fn(&str) -> String,
    pub type_command: &'static str,
    pub tables_and_views: &'static str,
    pub commands: &'static str,
    pub no_commands: &'static str,
    pub navigate: &'static str,
    pub open: &'static str,
    pub close: &'static str,
    pub type_before: &'static str,
    pub type_after: &'static str,
    // Commands
    pub new_query: &'static str,
    pub show_diagram: &'static str,
    pub new_connection: &'static str,
    pub theme_dark: &'static str,
    pub theme_light: &'static str,
    pub theme_system: &'static str,
    pub open_settings: &'static str,
}
