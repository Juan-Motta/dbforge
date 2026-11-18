//! The editing rules of a single-line text field, apart from drawing it: the
//! text, the selection, the text an input method is composing, and how keys
//! move and change them. Offsets are UTF-8 byte offsets on grapheme
//! boundaries; the OS's input methods speak UTF-16, converted here.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditState {
    text: String,
    /// What is selected; empty when it is only the caret.
    selection: Range<usize>,
    /// The caret is at the selection's start (it was made leftwards).
    reversed: bool,
    /// The text an input method is composing (a dead key's accent, a CJK word).
    marked: Option<Range<usize>>,
}

impl EditState {
    /// `text` with the caret at its end.
    pub fn new(text: &str) -> Self {
        let text = single_line(text);
        let end = text.len();
        Self { text, selection: end..end, reversed: false, marked: None }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    pub fn is_reversed(&self) -> bool {
        self.reversed
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.clone()]
    }

    /// Where the caret is: the selection's moving end.
    pub fn cursor(&self) -> usize {
        if self.reversed { self.selection.start } else { self.selection.end }
    }

    /// Places the caret at `offset`, dropping the selection.
    pub fn move_to(&mut self, offset: usize) {
        let offset = offset.min(self.text.len());
        self.selection = offset..offset;
        self.reversed = false;
    }

    /// Extends the selection from its anchor to `offset`.
    pub fn select_to(&mut self, offset: usize) {
        let offset = offset.min(self.text.len());
        if self.reversed {
            self.selection.start = offset;
        } else {
            self.selection.end = offset;
        }
        if self.selection.end < self.selection.start {
            self.reversed = !self.reversed;
            self.selection = self.selection.end..self.selection.start;
        }
    }

    pub fn select_all(&mut self) {
        self.selection = 0..self.text.len();
        self.reversed = false;
    }

    /// Left arrow: to the selection's start, or one character back.
    pub fn left(&mut self) {
        if self.selection.is_empty() {
            self.move_to(self.previous_boundary(self.cursor()));
        } else {
            self.move_to(self.selection.start);
        }
    }

    /// Right arrow: to the selection's end, or one character on.
    pub fn right(&mut self) {
        if self.selection.is_empty() {
            self.move_to(self.next_boundary(self.cursor()));
        } else {
            self.move_to(self.selection.end);
        }
    }

    pub fn select_left(&mut self) {
        self.select_to(self.previous_boundary(self.cursor()));
    }

    pub fn select_right(&mut self) {
        self.select_to(self.next_boundary(self.cursor()));
    }

    /// To the start of the word before the caret.
    pub fn word_left(&mut self) {
        self.move_to(self.previous_word(self.cursor()));
    }

    /// To the end of the word after the caret.
    pub fn word_right(&mut self) {
        self.move_to(self.next_word(self.cursor()));
    }

    pub fn select_word_left(&mut self) {
        self.select_to(self.previous_word(self.cursor()));
    }

    pub fn select_word_right(&mut self) {
        self.select_to(self.next_word(self.cursor()));
    }

    pub fn home(&mut self) {
        self.move_to(0);
    }

    pub fn end(&mut self) {
        self.move_to(self.text.len());
    }

    pub fn select_home(&mut self) {
        self.select_to(0);
    }

    pub fn select_end(&mut self) {
        self.select_to(self.text.len());
    }

    /// Deletes the selection, or the character before the caret; whether the text changed.
    pub fn backspace(&mut self) -> bool {
        if self.selection.is_empty() {
            self.select_to(self.previous_boundary(self.cursor()));
        }
        self.replace(None, "")
    }

    /// Deletes the selection, or the character after the caret; whether the text changed.
    pub fn delete(&mut self) -> bool {
        if self.selection.is_empty() {
            self.select_to(self.next_boundary(self.cursor()));
        }
        self.replace(None, "")
    }

    /// Deletes the selection, or the word before the caret; whether the text changed.
    pub fn delete_word_left(&mut self) -> bool {
        if self.selection.is_empty() {
            self.select_to(self.previous_word(self.cursor()));
        }
        self.replace(None, "")
    }

    /// Replaces `range` (by default the composing text, else the selection)
    /// with `new_text` and puts the caret after it; whether the text changed.
    pub fn replace(&mut self, range: Option<Range<usize>>, new_text: &str) -> bool {
        let range = self.target(range);
        let new_text = single_line(new_text);
        let changed = self.text[range.clone()] != new_text;
        self.text.replace_range(range.clone(), &new_text);
        let caret = range.start + new_text.len();
        self.selection = caret..caret;
        self.reversed = false;
        self.marked = None;
        changed
    }

    /// Like [`EditState::replace`], but the new text stays marked as being
    /// composed, with `selected` (relative to it) selected inside.
    pub fn replace_and_mark(&mut self, range: Option<Range<usize>>, new_text: &str, selected: Option<Range<usize>>) {
        let range = self.target(range);
        let new_text = single_line(new_text);
        self.text.replace_range(range.clone(), &new_text);
        self.marked = (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        let end = new_text.len();
        self.selection = match selected {
            Some(selected) => range.start + selected.start.min(end)..range.start + selected.end.min(end),
            None => range.start + end..range.start + end,
        };
        self.reversed = false;
    }

    /// Ends composing: the composed text stays as typed.
    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// The range an edit without one applies to.
    fn target(&self, range: Option<Range<usize>>) -> Range<usize> {
        let range = range.or_else(|| self.marked.clone()).unwrap_or_else(|| self.selection.clone());
        let len = self.text.len();
        range.start.min(len)..range.end.min(len)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.text.grapheme_indices(true).rev().find_map(|(index, _)| (index < offset).then_some(index)).unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .find_map(|(index, _)| (index > offset).then_some(index))
            .unwrap_or(self.text.len())
    }

    /// The start of the word before `offset`, past any spaces or punctuation.
    fn previous_word(&self, offset: usize) -> usize {
        let mut chars = self.text[..offset].char_indices().rev().peekable();
        while chars.next_if(|(_, c)| !is_word(*c)).is_some() {}
        let mut start = chars.peek().map_or(0, |(index, _)| *index);
        while let Some((index, _)) = chars.next_if(|(_, c)| is_word(*c)) {
            start = index;
        }
        start
    }

    /// The end of the word after `offset`, past any spaces or punctuation.
    fn next_word(&self, offset: usize) -> usize {
        let rest = &self.text[offset..];
        let mut chars = rest.char_indices().peekable();
        while chars.next_if(|(_, c)| !is_word(*c)).is_some() {}
        while chars.next_if(|(_, c)| is_word(*c)).is_some() {}
        offset + chars.peek().map_or(rest.len(), |(index, _)| *index)
    }

    /// `offset` in UTF-16 code units, as input methods count.
    pub fn to_utf16(&self, offset: usize) -> usize {
        self.text[..offset.min(self.text.len())].chars().map(char::len_utf16).sum()
    }

    /// The UTF-8 offset of the UTF-16 offset `offset`.
    pub fn from_utf16(&self, offset: usize) -> usize {
        utf8_offset(&self.text, offset)
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.to_utf16(range.start)..self.to_utf16(range.end)
    }

    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.from_utf16(range.start)..self.from_utf16(range.end)
    }
}

/// The UTF-8 offset in `text` of `units` UTF-16 code units, at a character boundary.
pub fn utf8_offset(text: &str, units: usize) -> usize {
    let mut counted = 0;
    for (index, c) in text.char_indices() {
        if counted >= units {
            return index;
        }
        counted += c.len_utf16();
    }
    text.len()
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A line break pasted or typed into a one-line field becomes a space.
fn single_line(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> EditState {
        let mut state = EditState::default();
        state.replace(None, text);
        state
    }

    #[test]
    fn typing_inserts_at_the_caret_and_replaces_a_selection() {
        let mut state = typed("cusomers");
        state.move_to(3);
        assert!(state.replace(None, "t"));
        assert_eq!((state.text(), state.cursor()), ("customers", 4));
        state.move_to(0);
        state.select_to(3);
        state.replace(None, "CUS");
        assert_eq!((state.text(), state.selection()), ("CUStomers", 3..3));
        assert!(!state.replace(None, ""), "deleting nothing changes nothing");
    }

    #[test]
    fn arrows_step_over_whole_characters() {
        let mut state = typed("añé👍");
        state.left();
        assert_eq!(state.cursor(), "añé".len(), "an emoji is one step");
        state.left();
        assert_eq!(state.cursor(), "añ".len());
        state.home();
        state.right();
        state.right();
        assert_eq!(state.cursor(), "añ".len());
    }

    #[test]
    fn an_arrow_collapses_a_selection_to_its_side() {
        let mut state = typed("orders");
        state.move_to(2);
        state.select_to(5);
        state.left();
        assert_eq!(state.selection(), 2..2);
        state.select_to(5);
        state.right();
        assert_eq!(state.selection(), 5..5);
    }

    #[test]
    fn shift_arrows_select_and_can_cross_the_anchor() {
        let mut state = typed("abc");
        state.move_to(1);
        state.select_right();
        assert_eq!((state.selection(), state.is_reversed()), (1..2, false));
        state.select_left();
        state.select_left();
        assert_eq!((state.selection(), state.is_reversed(), state.cursor()), (0..1, true, 0));
        assert_eq!(state.selected_text(), "a");
        state.select_end();
        assert_eq!(state.selection(), 1..3, "the anchor stays put");
    }

    #[test]
    fn word_moves_skip_spaces_and_punctuation() {
        let mut state = typed("select id, total_due from orders");
        state.word_left();
        assert_eq!(state.cursor(), "select id, total_due from ".len());
        state.word_left();
        state.word_left();
        assert_eq!(state.cursor(), "select id, ".len(), "an underscore is part of a word");
        state.word_right();
        assert_eq!(state.cursor(), "select id, total_due".len());
        state.home();
        state.select_word_right();
        assert_eq!(state.selected_text(), "select");
        state.end();
        assert!(state.delete_word_left());
        assert_eq!(state.text(), "select id, total_due from ");
    }

    #[test]
    fn backspace_and_delete_remove_one_character_or_the_selection() {
        let mut state = typed("niño");
        state.backspace();
        assert_eq!(state.text(), "niñ");
        state.backspace();
        assert_eq!(state.text(), "ni");
        state.home();
        state.delete();
        assert_eq!(state.text(), "i");
        state.select_all();
        state.backspace();
        assert_eq!(state.text(), "");
        assert!(!state.backspace(), "nothing left to delete");
        assert!(!state.delete());
    }

    #[test]
    fn a_dead_key_composes_then_commits_an_accent() {
        // macOS: ⌥e shows a marked "´", then "e" replaces it with "é".
        let mut state = typed("caf");
        state.replace_and_mark(None, "´", Some("´".len().."´".len()));
        assert_eq!((state.text(), state.marked()), ("caf´", Some(3..5)));
        assert_eq!(state.selection(), 5..5);
        state.replace(None, "é");
        assert_eq!((state.text(), state.marked(), state.cursor()), ("café", None, 5));
    }

    #[test]
    fn composing_text_can_select_inside_itself() {
        let mut state = typed("ab");
        state.move_to(1);
        state.replace_and_mark(None, "xyz", Some(1..2));
        assert_eq!((state.text(), state.marked(), state.selection()), ("axyzb", Some(1..4), 2..3));
        state.unmark();
        assert_eq!(state.marked(), None);
    }

    #[test]
    fn line_breaks_become_spaces() {
        let state = typed("a\nb\r\nc");
        assert_eq!(state.text(), "a b c");
        assert_eq!(EditState::new("x\ny").text(), "x y");
    }

    #[test]
    fn converts_offsets_to_and_from_utf16() {
        let state = typed("é👍x");
        assert_eq!(state.to_utf16("é".len()), 1);
        assert_eq!(state.to_utf16("é👍".len()), 3, "an emoji is two UTF-16 units");
        assert_eq!(state.from_utf16(3), "é👍".len());
        assert_eq!(state.from_utf16(99), state.text().len());
        assert_eq!(state.range_from_utf16(&(1..3)), "é".len().."é👍".len());
        assert_eq!(state.range_to_utf16(&(0..state.text().len())), 0..4);
    }

    #[test]
    fn an_edit_out_of_range_is_clamped() {
        let mut state = typed("abc");
        state.replace(Some(2..99), "Z");
        assert_eq!(state.text(), "abZ");
        state.move_to(99);
        assert_eq!(state.cursor(), 3);
    }
}
