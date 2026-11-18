//! A multi-line text being edited, apart from drawing it: the field's
//! [`EditState`] (shared with the one-line text field) plus where each line
//! starts, moves between lines, indentation, and undo and redo.

use std::ops::Range;

use crate::ui::editing::EditState;

/// What a line is indented by, with Tab.
pub const INDENT: &str = "  ";

/// How an edit is grouped for undo: a run of typing (or of deleting) is
/// undone at once; anything else on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Deleting,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    text: String,
    selection: Range<usize>,
    reversed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Document {
    state: EditState,
    /// Where each line starts; the first is 0.
    starts: Vec<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The kind of the last edit, while further ones of it join its undo step.
    grouping: Option<EditKind>,
    /// The column (in characters) Up and Down keep to across shorter lines.
    goal: Option<usize>,
}

/// The most undo steps kept.
const UNDO_LIMIT: usize = 500;

impl Document {
    /// `text` with the caret at its start.
    pub fn new(text: &str) -> Self {
        let mut document = Self { state: EditState::multiline(text), ..Self::default() };
        document.index();
        document
    }

    pub fn state(&self) -> &EditState {
        &self.state
    }

    pub fn text(&self) -> &str {
        self.state.text()
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// Line `row`'s byte range, without its line break.
    pub fn line_range(&self, row: usize) -> Range<usize> {
        let start = self.starts[row.min(self.starts.len() - 1)];
        let end = self.starts.get(row + 1).map_or(self.text().len(), |next| next - 1);
        start..end
    }

    pub fn line(&self, row: usize) -> &str {
        &self.text()[self.line_range(row)]
    }

    /// The line `offset` is on.
    pub fn row_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|start| *start <= offset).saturating_sub(1)
    }

    /// The caret's line, and its byte offset in that line.
    pub fn caret_position(&self) -> (usize, usize) {
        let caret = self.state.cursor();
        let row = self.row_of(caret);
        (row, caret - self.starts[row])
    }

    /// The offset of byte `column` of line `row`, clamped to the line.
    pub fn offset(&self, row: usize, column: usize) -> usize {
        let range = self.line_range(row);
        (range.start + column).min(range.end)
    }

    /// Recomputes where lines start, after the text changed.
    fn index(&mut self) {
        self.starts = std::iter::once(0).chain(self.text().match_indices('\n').map(|(at, _)| at + 1)).collect();
    }

    /// Runs `edit`, keeping what to undo; whether the text changed.
    pub fn edit(&mut self, kind: EditKind, edit: impl FnOnce(&mut EditState) -> bool) -> bool {
        let before = self.snapshot();
        let changed = edit(&mut self.state);
        self.goal = None;
        if !changed {
            return false;
        }
        if kind == EditKind::Other || self.grouping != Some(kind) {
            self.undo.push(before);
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.grouping = (kind != EditKind::Other).then_some(kind);
        self.redo.clear();
        self.index();
        true
    }

    /// Moves the caret or the selection; a following edit starts a new undo step.
    pub fn caret(&mut self, change: impl FnOnce(&mut EditState)) {
        change(&mut self.state);
        self.grouping = None;
        self.goal = None;
    }

    /// Replaces the whole text, as an edit that can be undone, the caret at its end.
    pub fn replace_all(&mut self, text: &str) -> bool {
        self.edit(EditKind::Other, |state| {
            state.select_all();
            let changed = state.replace(None, text);
            state.end();
            changed
        })
    }

    pub fn undo(&mut self) -> bool {
        self.step(true)
    }

    pub fn redo(&mut self) -> bool {
        self.step(false)
    }

    fn step(&mut self, back: bool) -> bool {
        let (from, to) = if back { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        let Some(snapshot) = from.pop() else { return false };
        to.push(Snapshot {
            text: self.state.text().to_owned(),
            selection: self.state.selection(),
            reversed: self.state.is_reversed(),
        });
        self.restore(snapshot);
        self.grouping = None;
        self.goal = None;
        self.index();
        true
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.text().to_owned(), selection: self.state.selection(), reversed: self.state.is_reversed() }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.state = EditState::multiline(&snapshot.text);
        self.state.set_selection(snapshot.selection, snapshot.reversed);
    }

    /// Up (`rows` < 0) or Down, keeping to the column the move started in.
    pub fn move_vertically(&mut self, rows: isize, select: bool) {
        let (row, column) = self.caret_position();
        let goal = self.goal.unwrap_or_else(|| self.line(row)[..column].chars().count());
        let target = row as isize + rows;
        let offset = if target < 0 {
            0
        } else if target as usize >= self.line_count() {
            self.text().len()
        } else {
            let line = self.line(target as usize);
            let byte = line.char_indices().nth(goal).map_or(line.len(), |(at, _)| at);
            self.starts[target as usize] + byte
        };
        self.place(offset, select);
        self.goal = Some(goal);
    }

    /// Home: to the line's first non-blank character, or to its very start
    /// when the caret is already there.
    pub fn line_home(&mut self, select: bool) {
        let (row, column) = self.caret_position();
        let line = self.line(row);
        let indent = line.len() - line.trim_start().len();
        let target = if column == indent { 0 } else { indent };
        self.place(self.starts[row] + target, select);
    }

    pub fn line_end(&mut self, select: bool) {
        let (row, _) = self.caret_position();
        self.place(self.line_range(row).end, select);
    }

    /// Moves the caret to `offset`, or selects to it.
    pub fn place(&mut self, offset: usize, select: bool) {
        self.caret(|state| if select { state.select_to(offset) } else { state.move_to(offset) });
    }

    /// Selects the word at `offset` (a double-click).
    pub fn select_word(&mut self, offset: usize) {
        let text = self.text();
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let start = text[..offset].rfind(|c: char| !is_word(c)).map_or(0, |at| at + 1);
        let end = text[offset..].find(|c: char| !is_word(c)).map_or(text.len(), |at| offset + at);
        self.caret(|state| state.set_selection(start..end, false));
    }

    /// Selects line `row` with its line break (a triple-click).
    pub fn select_line(&mut self, row: usize) {
        let range = self.line_range(row);
        let end = (range.end + 1).min(self.text().len());
        self.caret(|state| state.set_selection(range.start..end, false));
    }

    /// Enter: a new line indented as the caret's line.
    pub fn newline(&mut self) -> bool {
        let (row, _) = self.caret_position();
        let line = self.line(row);
        let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        self.edit(EditKind::Other, |state| state.replace(None, &format!("\n{indent}")))
    }

    /// The lines the selection touches.
    fn selected_rows(&self) -> Range<usize> {
        let selection = self.state.selection();
        let first = self.row_of(selection.start);
        let mut last = self.row_of(selection.end);
        // A selection ending at a line's very start leaves that line out.
        if last > first && self.starts[last] == selection.end {
            last -= 1;
        }
        first..last + 1
    }

    /// Tab: indents the selected lines, or inserts an indent at the caret.
    pub fn indent(&mut self) -> bool {
        let rows = self.selected_rows();
        if self.state.selection().is_empty() || rows.len() == 1 && !self.spans_whole_line() {
            return self.edit(EditKind::Other, |state| state.replace(None, INDENT));
        }
        self.reindent(rows, |line| format!("{INDENT}{line}"))
    }

    /// Shift+Tab: takes one indent off the selected lines.
    pub fn outdent(&mut self) -> bool {
        let rows = self.selected_rows();
        self.reindent(rows, |line| {
            let blanks = line.len() - line.trim_start_matches(' ').len();
            line[blanks.min(INDENT.len())..].to_owned()
        })
    }

    fn spans_whole_line(&self) -> bool {
        let selection = self.state.selection();
        let row = self.row_of(selection.start);
        self.line_range(row) == selection
    }

    /// Rewrites `rows` line by line, then selects them.
    fn reindent(&mut self, rows: Range<usize>, rewrite: impl Fn(&str) -> String) -> bool {
        let range = self.starts[rows.start]..self.line_range(rows.end - 1).end;
        let text: Vec<String> = self.text()[range.clone()].split('\n').map(&rewrite).collect();
        let text = text.join("\n");
        self.edit(EditKind::Other, |state| {
            state.set_selection(range.clone(), false);
            let changed = state.replace(None, &text);
            state.set_selection(range.start..range.start + text.len(), false);
            changed
        })
    }

    /// The widest line, in characters, for the horizontal scroll.
    pub fn widest_line(&self) -> usize {
        (0..self.line_count()).map(|row| self.line(row).chars().count()).max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document with the caret (or the selection, between two `|`) where marked.
    fn doc(marked: &str) -> Document {
        let mut carets = marked.match_indices('|').map(|(at, _)| at);
        let first = carets.next().expect("a caret");
        let second = carets.next().map(|at| at - 1);
        let mut document = Document::new(&marked.replace('|', ""));
        document.caret(|state| state.set_selection(first..second.unwrap_or(first), false));
        document
    }

    /// The text with the caret or selection marked again.
    fn shown(document: &Document) -> String {
        let mut text = document.text().to_owned();
        let selection = document.state().selection();
        if selection.is_empty() {
            text.insert(selection.start, '|');
        } else {
            text.insert(selection.end, '|');
            text.insert(selection.start, '|');
        }
        text
    }

    #[test]
    fn indexes_lines_and_finds_the_caret_s() {
        let document = doc("SELECT\n  id|\nFROM t");
        assert_eq!(document.line_count(), 3);
        assert_eq!((document.line(1), document.line(2)), ("  id", "FROM t"));
        assert_eq!(document.caret_position(), (1, 4));
        assert_eq!(document.row_of(0), 0);
        assert_eq!(document.row_of(document.text().len()), 2);
        assert_eq!(Document::new("a\n").line_count(), 2, "a trailing line break starts an empty line");
    }

    #[test]
    fn up_and_down_keep_the_column_across_short_lines() {
        let mut document = doc("SELECT id,|\nx\nFROM orders");
        document.move_vertically(1, false);
        assert_eq!(shown(&document), "SELECT id,\nx|\nFROM orders");
        document.move_vertically(1, false);
        assert_eq!(shown(&document), "SELECT id,\nx\nFROM order|s", "back to the tenth column");
        document.move_vertically(1, false);
        assert_eq!(document.state().cursor(), document.text().len(), "down on the last line goes to its end");
        document.move_vertically(-9, true);
        assert_eq!(document.state().selection(), 0..document.text().len(), "up past the top selects to the start");
    }

    #[test]
    fn home_goes_to_the_indent_then_the_line_start() {
        let mut document = doc("a\n    sel|ect");
        document.line_home(false);
        assert_eq!(shown(&document), "a\n    |select");
        document.line_home(false);
        assert_eq!(shown(&document), "a\n|    select");
        document.line_end(true);
        assert_eq!(shown(&document), "a\n|    select|");
    }

    #[test]
    fn enter_keeps_the_indent_and_tab_indents() {
        let mut document = doc("  SELECT|");
        document.newline();
        assert_eq!(shown(&document), "  SELECT\n  |");
        document.indent();
        assert_eq!(shown(&document), "  SELECT\n    |");
        let mut document = doc("|a\nb\n|c");
        document.indent();
        assert_eq!(shown(&document), "|  a\n  b|\nc", "a selection ending at a line's start leaves it out");
        document.outdent();
        document.outdent();
        assert_eq!(shown(&document), "|a\nb|\nc");
    }

    #[test]
    fn undo_takes_back_a_run_of_typing_at_once() {
        let mut document = doc("|");
        for c in ["S", "E", "L"] {
            document.edit(EditKind::Typing, |state| state.replace(None, c));
        }
        document.newline();
        document.edit(EditKind::Typing, |state| state.replace(None, "x"));
        assert!(document.undo());
        assert_eq!(shown(&document), "SEL\n|");
        assert!(document.undo());
        assert_eq!(shown(&document), "SEL|");
        assert!(document.undo());
        assert_eq!(shown(&document), "|");
        assert!(!document.undo(), "nothing more to undo");
        assert!(document.redo());
        assert_eq!(shown(&document), "SEL|");
        document.edit(EditKind::Typing, |state| state.replace(None, "!"));
        assert!(!document.redo(), "an edit drops what could be redone");
    }

    #[test]
    fn a_caret_move_ends_the_undo_step() {
        let mut document = doc("|");
        document.edit(EditKind::Typing, |state| state.replace(None, "a"));
        document.place(0, false);
        document.edit(EditKind::Typing, |state| state.replace(None, "b"));
        document.undo();
        assert_eq!(document.text(), "a");
        assert_eq!(document.line_count(), 1);
    }

    #[test]
    fn selects_a_word_or_a_line_and_replaces_everything() {
        let mut document = doc("SELECT total_due FROM t|");
        document.select_word(10);
        assert_eq!(shown(&document), "SELECT |total_due| FROM t");
        let mut document = doc("a\nbc|\nd");
        document.select_line(1);
        assert_eq!(shown(&document), "a\n|bc\n|d");
        document.replace_all("SELECT 1");
        assert_eq!(shown(&document), "SELECT 1|");
        document.undo();
        assert_eq!(document.text(), "a\nbc\nd");
    }
}
