//! The texts of an ER diagram: a schema's, or a table's own.

/// An ER diagram's texts in one language.
pub struct DiagramMessages {
    /// A schema diagram tab's title.
    pub tab: fn(&str) -> String,
    pub loading: fn(&str) -> String,
    pub no_tables: fn(&str) -> String,
    pub is_view: fn(&str) -> String,
    pub direct: &'static str,
    pub two_steps: &'static str,
    pub whole_schema: &'static str,
    pub tables: &'static str,
    pub shown_of: fn(usize, usize) -> String,
    pub filter: &'static str,
    pub filter_placeholder: &'static str,
    pub show_all: &'static str,
    pub hide_all: &'static str,
    pub relations_followed: &'static str,
    pub columns_shown: &'static str,
    pub columns: &'static str,
    pub keys_only: &'static str,
    pub auto_layout: &'static str,
    pub reload: &'static str,
    pub open_schema: &'static str,
    pub zoom_out: &'static str,
    pub zoom_in: &'static str,
    pub fit: &'static str,
}
