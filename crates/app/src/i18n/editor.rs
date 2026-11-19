//! The texts of the code editor: its find and replace bar, and the cell
//! inspector's soft wrap toggle that the editor carries out.

/// The code editor's texts in one language.
pub struct EditorMessages {
    /// The find field's placeholder.
    pub find: &'static str,
    /// The replace field's placeholder, and its button.
    pub replace: &'static str,
    pub replace_all: &'static str,
    pub next: &'static str,
    pub previous: &'static str,
    /// The button selecting every match, and its tooltip.
    pub all: &'static str,
    pub all_hint: &'static str,
    pub match_case: &'static str,
    pub regex: &'static str,
    pub by_word: &'static str,
    pub close: &'static str,
    /// The cell inspector's soft wrap toggle.
    pub wrap_lines: &'static str,
}
