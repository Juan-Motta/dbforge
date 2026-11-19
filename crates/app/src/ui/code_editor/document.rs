//! A multi-line text being edited, apart from drawing it: the text (in the
//! field's [`EditState`], shared with the one-line text field, which keeps
//! the primary selection and the text an input method composes), every
//! selection, where each line starts, folded ranges, brackets typed for the
//! user, and undo and redo. Each edit and move applies to every selection.

use std::ops::Range;

use super::display_map::DisplayMap;
use super::selection::{Change, Selection, Selections, apply, map};
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

/// Where the selections go after an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Then {
    /// Carried over the changes; with `after`, past text inserted where they are.
    Map { after: bool },
    /// These, in the new text.
    Select(Selections),
}

/// What an edit does at one selection: replaces `range` with `insert`, then
/// selects from `anchor` to `head`, both counted from the range's start in
/// the new text; `closes` is where it put a closing bracket for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub insert: String,
    pub anchor: usize,
    pub head: usize,
    pub closes: Option<usize>,
}

impl Edit {
    /// `range` replaced by `insert`, the caret after it.
    pub fn replace(range: Range<usize>, insert: impl Into<String>) -> Self {
        let insert = insert.into();
        let end = insert.len();
        Self { range, insert, anchor: end, head: end, closes: None }
    }

    pub fn delete(range: Range<usize>) -> Self {
        Self::replace(range, "")
    }
}

/// How typing treats brackets and quotes: which close, and whether an offset is inside a string.
pub struct BracketRules<'a> {
    pub brackets: &'a [char],
    pub in_string: &'a dyn Fn(usize) -> bool,
}

/// What a typed bracket's closing one is; a quote closes with itself.
pub fn closing(open: char) -> char {
    match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        other => other,
    }
}

/// What a closing bracket typed for the user may stand before.
const CLOSE_BEFORE: &str = ")]}:;>";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    text: String,
    selections: Selections,
    folds: Vec<Range<usize>>,
}

#[derive(Debug, Clone, Default)]
pub struct Document {
    state: EditState,
    selections: Selections,
    /// Where each line starts; the first is 0.
    starts: Vec<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The kind of the last edit, while further ones of it join its undo step.
    grouping: Option<EditKind>,
    /// Closing brackets typed for the user on the primary caret's line,
    /// which typing the same bracket steps over (CodeMirror's rule).
    closed: Vec<usize>,
    /// Folded ranges, in text order, apart.
    folds: Vec<Range<usize>>,
    /// Counts changes to the text, so what is derived from it knows when to follow.
    version: u64,
}

/// The most undo steps kept.
const UNDO_LIMIT: usize = 500;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Line breaks as `\n`, as the editor keeps them.
fn line_breaks(text: &str) -> String {
    if text.contains('\r') { text.replace("\r\n", "\n").replace('\r', "\n") } else { text.to_owned() }
}

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

    pub fn selections(&self) -> &Selections {
        &self.selections
    }

    pub fn primary(&self) -> Selection {
        self.selections.primary()
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn folds(&self) -> &[Range<usize>] {
        &self.folds
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

    /// The lines' byte ranges, in order.
    pub fn line_ranges(&self) -> impl ExactSizeIterator<Item = Range<usize>> + '_ {
        (0..self.line_count()).map(|row| self.line_range(row))
    }

    /// The line `offset` is on.
    pub fn row_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|start| *start <= offset).saturating_sub(1)
    }

    /// The primary caret's line, and its byte offset in that line.
    pub fn caret_position(&self) -> (usize, usize) {
        let caret = self.primary().head;
        let row = self.row_of(caret);
        (row, caret - self.starts[row])
    }

    /// The offset of byte `column` of line `row`, clamped to the line.
    pub fn offset(&self, row: usize, column: usize) -> usize {
        let range = self.line_range(row);
        (range.start + column).min(range.end)
    }

    /// The rows of the text without soft wrap: a line each, folds joined.
    pub fn lines_map(&self) -> DisplayMap {
        DisplayMap::build(self.text(), self.line_ranges(), &self.folds, None)
    }

    /// Recomputes where lines start, after the text changed.
    fn index(&mut self) {
        self.starts = std::iter::once(0).chain(self.text().match_indices('\n').map(|(at, _)| at + 1)).collect();
    }

    // Selections

    /// Selects `selections` (clamped to the text); a following edit starts a new undo step.
    pub fn set_selections(&mut self, selections: Selections) {
        self.selections = selections.clamped(self.text().len());
        self.grouping = None;
        self.sync();
    }

    /// One selection, from `anchor` to `head`.
    pub fn select(&mut self, anchor: usize, head: usize) {
        self.set_selections(Selections::single(Selection::new(anchor, head)));
    }

    /// Moves the primary caret to `offset` (the only selection now), or extends the primary selection to it.
    pub fn place(&mut self, offset: usize, select: bool) {
        let anchor = if select { self.primary().anchor } else { offset };
        self.select(anchor, offset);
    }

    /// Adds a selection, which becomes the primary one.
    pub fn add_selection(&mut self, selection: Selection) {
        let mut all = self.selections.all().to_vec();
        all.push(selection);
        let primary = all.len() - 1;
        self.set_selections(Selections::new(all, primary));
    }

    /// Takes the caret at `offset` away, when there are others; whether there was one.
    pub fn remove_caret(&mut self, offset: usize) -> bool {
        let all = self.selections.all();
        if all.len() < 2 {
            return false;
        }
        let Some(index) = all.iter().position(|selection| selection.is_empty() && selection.head == offset) else {
            return false;
        };
        let mut all = all.to_vec();
        all.remove(index);
        let primary = all.len() - 1;
        self.set_selections(Selections::new(all, primary));
        true
    }

    /// Keeps only the primary selection; whether there were others.
    pub fn single_selection(&mut self) -> bool {
        if self.selections.count() < 2 {
            return false;
        }
        let primary = self.primary();
        self.set_selections(Selections::single(primary));
        true
    }

    /// Replaces each selection with what `change` makes of it.
    pub fn move_each(&mut self, change: impl Fn(&Self, Selection) -> Selection) {
        let all = self.selections.all().iter().map(|selection| change(self, *selection)).collect();
        self.set_selections(Selections::new(all, self.selections.primary_index()));
    }

    /// Moves each caret to where `to` says; with `select` each selection's anchor stays.
    pub fn move_heads(&mut self, select: bool, to: impl Fn(&Self, Selection) -> usize) {
        self.move_each(|document, selection| {
            let head = to(document, selection);
            Selection::new(if select { selection.anchor } else { head }, head)
        });
    }

    /// Left: each selection to its start, or each caret a character back.
    pub fn left(&mut self, select: bool) {
        if select {
            return self.move_heads(true, |document, selection| document.state.previous_boundary(selection.head));
        }
        self.move_each(|document, selection| match selection.is_empty() {
            true => Selection::caret(document.state.previous_boundary(selection.head)),
            false => Selection::caret(selection.range().start),
        });
    }

    /// Right: each selection to its end, or each caret a character on.
    pub fn right(&mut self, select: bool) {
        if select {
            return self.move_heads(true, |document, selection| document.state.next_boundary(selection.head));
        }
        self.move_each(|document, selection| match selection.is_empty() {
            true => Selection::caret(document.state.next_boundary(selection.head)),
            false => Selection::caret(selection.range().end),
        });
    }

    pub fn word_left(&mut self, select: bool) {
        self.move_heads(select, |document, selection| document.state.previous_word(selection.head));
    }

    pub fn word_right(&mut self, select: bool) {
        self.move_heads(select, |document, selection| document.state.next_word(selection.head));
    }

    /// Up (`rows` < 0) or Down by rows on screen, each caret keeping to the
    /// column its move started in.
    pub fn move_vertically(&mut self, rows: isize, select: bool, display: &DisplayMap) {
        self.move_each(|document, selection| {
            let (row, column) = display.column(document.text(), selection.head);
            let goal = selection.goal.unwrap_or(column);
            let target = row as isize + rows;
            let head = if target < 0 {
                0
            } else if target as usize >= display.row_count() {
                document.text().len()
            } else {
                display.offset_at_column(document.text(), target as usize, goal)
            };
            Selection { anchor: if select { selection.anchor } else { head }, head, goal: Some(goal) }
        });
    }

    /// A caret on the line above (or below) each caret, at its column, as
    /// CodeMirror's `addCursorAbove`; the last one added is primary. Whether any was added.
    pub fn add_caret_vertically(&mut self, below: bool, display: &DisplayMap) -> bool {
        let text = self.text();
        let mut all = self.selections.all().to_vec();
        let before = all.len();
        for selection in self.selections.all() {
            let line = self.row_of(selection.head);
            let (mut row, column) = display.column(text, selection.head);
            let goal = selection.goal.unwrap_or(column);
            loop {
                let next = if below { row + 1 } else { row.wrapping_sub(1) };
                if next >= display.row_count() {
                    break;
                }
                row = next;
                if self.row_of(display.row(row).start) != line {
                    let head = display.offset_at_column(text, row, goal);
                    if !all.iter().any(|other| other.head == head) {
                        all.push(Selection { anchor: head, head, goal: Some(goal) });
                    }
                    break;
                }
            }
        }
        if all.len() == before {
            return false;
        }
        let primary = all.len() - 1;
        self.set_selections(Selections::new(all, primary));
        true
    }

    /// Home: each caret to its line's first non-blank character, or to its
    /// very start when it is already there.
    pub fn line_home(&mut self, select: bool) {
        self.move_heads(select, |document, selection| {
            let row = document.row_of(selection.head);
            let line = document.line(row);
            let indent = line.len() - line.trim_start().len();
            let column = selection.head - document.starts[row];
            document.starts[row] + if column == indent { 0 } else { indent }
        });
    }

    pub fn line_end(&mut self, select: bool) {
        self.move_heads(select, |document, selection| document.line_range(document.row_of(selection.head)).end);
    }

    /// The word around `offset` (touching it on either side), if any.
    pub fn word_at(&self, offset: usize) -> Option<Range<usize>> {
        let text = self.text();
        let start =
            text[..offset].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(offset, |(at, _)| at);
        let end = text[offset..].char_indices().find(|(_, c)| !is_word(*c)).map_or(text.len(), |(at, _)| offset + at);
        (start < end).then_some(start..end)
    }

    /// Selects the word at `offset` (a double-click).
    pub fn select_word(&mut self, offset: usize) {
        let range = self.word_at(offset).unwrap_or(offset..offset);
        self.select(range.start, range.end);
    }

    /// Selects line `row` with its line break (a triple-click).
    pub fn select_line(&mut self, row: usize) {
        let range = self.line_range(row);
        let end = (range.end + 1).min(self.text().len());
        self.select(range.start, end);
    }

    pub fn select_all(&mut self) {
        self.select(0, self.text().len());
    }

    /// ⌘D: selects the word at each caret; with words (or any equal text)
    /// selected, adds the next place the text is, wrapping round past the
    /// end, as CodeMirror's `selectNextOccurrence`. Whether the selection changed.
    pub fn select_next_occurrence(&mut self) -> bool {
        let all = self.selections.all();
        if all.iter().any(Selection::is_empty) {
            let before = self.selections.clone();
            self.move_each(|document, selection| match document.word_at(selection.head) {
                Some(word) => Selection::new(word.start, word.end),
                None => selection,
            });
            return self.selections != before;
        }
        let text = self.text();
        let searched = &text[all[0].range()];
        if all.iter().any(|selection| &text[selection.range()] != searched) {
            return false;
        }
        let primary = self.primary();
        let whole_word = self.word_at(primary.head) == Some(primary.range());
        let last = all[all.len() - 1].range();
        let taken = |start: usize| all.iter().any(|selection| selection.range().start == start);
        let fits = |start: usize| !whole_word || self.word_at(start) == Some(start..start + searched.len());
        let after = text[last.end..].match_indices(searched).map(|(at, _)| last.end + at);
        let before = text[..last.start].match_indices(searched).map(|(at, _)| at);
        let Some(found) = after.chain(before).find(|start| !taken(*start) && fits(*start)) else { return false };
        let mut all = all.to_vec();
        all.push(Selection::new(found, found + searched.len()));
        let primary = self.selections.primary_index();
        self.set_selections(Selections::new(all, primary));
        true
    }

    /// The selected texts, joined by line breaks (what Copy takes).
    pub fn selected_text(&self) -> Option<String> {
        let parts: Vec<&str> =
            self.selections.all().iter().filter(|s| !s.is_empty()).map(|s| &self.text()[s.range()]).collect();
        (!parts.is_empty()).then(|| parts.join("\n"))
    }

    /// Keeps the primary selection in the field's state, forgets brackets
    /// typed off the primary caret's line, and opens folds a caret went into.
    fn sync(&mut self) {
        let primary = self.primary();
        self.state.set_selection(primary.range(), primary.is_reversed());
        let line = self.line_range(self.row_of(primary.head));
        self.closed.retain(|at| line.start <= *at && *at <= line.end);
        let heads: Vec<usize> = self.selections.all().iter().map(|selection| selection.head).collect();
        self.folds.retain(|fold| !heads.iter().any(|head| fold.start < *head && *head < fold.end));
    }

    // Edits

    /// Runs `edit` on the primary selection in the field's state (an input
    /// method's composing, a completion), keeping what to undo; the other
    /// selections follow the text. Whether the text changed.
    pub fn edit(&mut self, kind: EditKind, edit: impl FnOnce(&mut EditState) -> bool) -> bool {
        let primary = self.primary();
        self.state.set_selection(primary.range(), primary.is_reversed());
        let before = self.snapshot();
        let changed = edit(&mut self.state);
        let selection = self.state.selection();
        let primary = match self.state.is_reversed() {
            true => Selection::new(selection.end, selection.start),
            false => Selection::new(selection.start, selection.end),
        };
        if !changed {
            self.replace_primary(primary, &[]);
            self.grouping = None;
            return false;
        }
        self.record(kind, before.clone());
        let change = difference(&before.text, self.text());
        self.carry(std::slice::from_ref(&change));
        self.replace_primary(primary, std::slice::from_ref(&change));
        true
    }

    /// The primary selection replaced by `primary`; the others carried over `changes`.
    fn replace_primary(&mut self, primary: Selection, changes: &[Change]) {
        let index = self.selections.primary_index();
        let mut all: Vec<Selection> = self
            .selections
            .all()
            .iter()
            .map(|selection| Selection::new(map(selection.anchor, changes, false), map(selection.head, changes, false)))
            .collect();
        all[index] = primary;
        self.selections = Selections::new(all, index).clamped(self.text().len());
        self.sync();
    }

    /// Makes `changes` (offsets in the current text, in order, apart),
    /// keeping what to undo, then selects as `then` says. Whether the text changed.
    pub fn change(&mut self, kind: EditKind, changes: Vec<Change>, then: Then) -> bool {
        let mut changes: Vec<Change> =
            changes.into_iter().map(|change| Change::new(change.range, line_breaks(&change.insert))).collect();
        changes.sort_by_key(|change| change.range.start);
        let changed = changes.iter().any(|change| self.text()[change.range.clone()] != change.insert);
        if !changed {
            // Nothing to write (stepping over a closing bracket); the selections still move.
            let selections = match then {
                Then::Map { after } => self.selections.map(&changes, after),
                Then::Select(selections) => selections,
            };
            self.set_selections(selections);
            return false;
        }
        let before = self.snapshot();
        self.record(kind, before);
        let text = apply(self.text(), &changes);
        let selections = match then {
            Then::Map { after } => self.selections.map(&changes, after),
            Then::Select(selections) => selections,
        };
        self.state = EditState::multiline(&text);
        self.carry(&changes);
        let selections = Selections::new(
            selections.all().iter().map(|selection| Selection::new(selection.anchor, selection.head)).collect(),
            selections.primary_index(),
        );
        self.selections = selections.clamped(self.text().len());
        self.sync();
        true
    }

    /// Runs `edit` at each selection; nothing when it declines any (returns
    /// `None`). Whether the text changed.
    pub fn replace_each(&mut self, kind: EditKind, edit: impl FnMut(&Self, Selection) -> Option<Edit>) -> bool {
        self.each(edit).is_some_and(|edits| self.apply_edits(kind, edits))
    }

    /// What `edit` makes of each selection, or `None` when it declines any.
    fn each(&self, mut edit: impl FnMut(&Self, Selection) -> Option<Edit>) -> Option<Vec<Edit>> {
        self.selections.all().iter().map(|selection| edit(self, *selection)).collect()
    }

    fn apply_edits(&mut self, kind: EditKind, edits: Vec<Edit>) -> bool {
        let mut changes = Vec::with_capacity(edits.len());
        let mut selections = Vec::with_capacity(edits.len());
        let mut closes = Vec::new();
        let mut shift: isize = 0;
        let mut last_end = 0;
        for edit in edits {
            // Two selections' edits may reach into each other (deleting words): the first wins.
            let start = edit.range.start.max(last_end);
            let end = edit.range.end.max(start);
            let new_start = (start as isize + shift) as usize;
            selections.push(Selection::new(new_start + edit.anchor, new_start + edit.head));
            closes.extend(edit.closes.map(|at| new_start + at));
            shift += edit.insert.len() as isize - (end - start) as isize;
            last_end = end;
            changes.push(Change::new(start..end, edit.insert));
        }
        let selections = Selections::new(selections, self.selections.primary_index());
        let changed = self.change(kind, changes, Then::Select(selections));
        self.closed.extend(closes);
        self.closed.sort_unstable();
        self.closed.dedup();
        changed
    }

    /// Carries the folds and the typed closing brackets over `changes`.
    fn carry(&mut self, changes: &[Change]) {
        self.version += 1;
        self.index();
        self.redo.clear();
        let text = self.state.text();
        self.closed = self
            .closed
            .iter()
            .map(|at| map(*at, changes, true))
            .filter(|at| text.get(*at..).is_some_and(|rest| rest.starts_with([')', ']', '}', '\'', '"', '`'])))
            .collect();
        self.folds = self
            .folds
            .iter()
            .filter(|fold| !changes.iter().any(|change| change.range.start < fold.end && change.range.end > fold.start))
            .map(|fold| map(fold.start, changes, true)..map(fold.end, changes, false))
            .filter(|fold| fold.start < fold.end)
            .collect();
    }

    /// Keeps `before` to undo to, unless the edit joins the last one's step.
    fn record(&mut self, kind: EditKind, before: Snapshot) {
        if kind == EditKind::Other || self.grouping != Some(kind) {
            self.undo.push(before);
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.grouping = (kind != EditKind::Other).then_some(kind);
    }

    /// Replaces the whole text, as an edit that can be undone, the caret at its end.
    pub fn replace_all(&mut self, text: &str) -> bool {
        let text = line_breaks(text);
        let end = text.len();
        let all = Change::new(0..self.text().len(), text);
        self.change(EditKind::Other, vec![all], Then::Select(Selections::single(Selection::caret(end))))
    }

    pub fn undo(&mut self) -> bool {
        self.step(true)
    }

    pub fn redo(&mut self) -> bool {
        self.step(false)
    }

    fn step(&mut self, back: bool) -> bool {
        let current = self.snapshot();
        let (from, to) = if back { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        let Some(snapshot) = from.pop() else { return false };
        to.push(current);
        self.state = EditState::multiline(&snapshot.text);
        self.selections = snapshot.selections;
        self.folds = snapshot.folds;
        self.closed.clear();
        self.grouping = None;
        self.version += 1;
        self.index();
        self.sync();
        true
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.text().to_owned(), selections: self.selections.clone(), folds: self.folds.clone() }
    }

    /// Types `text` at every selection. A bracket or quote closes as
    /// CodeMirror's `closeBrackets` closes it: an opening one gets its closing
    /// one before space or `)]}:;>`, wraps a selection, and a closing one
    /// typed for the user is stepped over; a quote pairs where no word or
    /// string is next to it. When any caret can't, all type it plainly.
    pub fn type_text(&mut self, text: &str, rules: &BracketRules) -> bool {
        let mut chars = text.chars();
        if let (Some(typed), None) = (chars.next(), chars.next())
            && let Some(edits) = self.each(|document, selection| document.bracket(typed, selection, rules))
        {
            return self.apply_edits(EditKind::Typing, edits);
        }
        self.replace_each(EditKind::Typing, |_, selection| Some(Edit::replace(selection.range(), text)))
    }

    /// What typing `typed` at `selection` does with brackets, if anything.
    fn bracket(&self, typed: char, selection: Selection, rules: &BracketRules) -> Option<Edit> {
        let text = self.text();
        let range = selection.range();
        let next = text[range.end..].chars().next();
        let previous = text[..range.start].chars().next_back();
        for open in rules.brackets.iter().copied() {
            let close = closing(open);
            if typed == open {
                if !selection.is_empty() {
                    // Wraps the selection, which stays selected inside.
                    let inner = &text[range.clone()];
                    let insert = format!("{open}{inner}{close}");
                    let shift = |at: usize| at - range.start + open.len_utf8();
                    return Some(Edit {
                        closes: Some(open.len_utf8() + inner.len()),
                        anchor: shift(selection.anchor),
                        head: shift(selection.head),
                        range,
                        insert,
                    });
                }
                let pair = Edit {
                    range: range.clone(),
                    insert: format!("{open}{close}"),
                    anchor: open.len_utf8(),
                    head: open.len_utf8(),
                    closes: Some(open.len_utf8()),
                };
                if close != open {
                    let free = next.is_none_or(|next| next.is_whitespace() || CLOSE_BEFORE.contains(next));
                    return free.then_some(pair);
                }
                if next == Some(close) {
                    return self.closed.contains(&range.end).then(|| self.step_over(range.end, close));
                }
                let quotable = !next.is_some_and(is_word) && !previous.is_some_and(is_word);
                return (quotable && !(rules.in_string)(range.end)).then_some(pair);
            }
            if typed == close && self.closed.contains(&self.primary().range().start) {
                return (selection.is_empty() && next == Some(close)).then(|| self.step_over(range.end, close));
            }
        }
        None
    }

    /// Steps over the closing bracket at `at` (writing it over itself).
    fn step_over(&self, at: usize, close: char) -> Edit {
        Edit::replace(at..at + close.len_utf8(), close.to_string())
    }

    /// Backspace: deletes each selection, or the character before each caret;
    /// a bracket typed with its pair right after the caret goes with it.
    pub fn backspace(&mut self, brackets: &[char]) -> bool {
        let pairs = self.each(|document, selection| {
            let text = document.text();
            let head = selection.head;
            let before = text[..head].chars().next_back()?;
            let after = text[head..].chars().next()?;
            (selection.is_empty() && brackets.contains(&before) && after == closing(before))
                .then(|| Edit::delete(head - before.len_utf8()..head + after.len_utf8()))
        });
        if let Some(pairs) = pairs {
            return self.apply_edits(EditKind::Deleting, pairs);
        }
        self.replace_each(EditKind::Deleting, |document, selection| {
            Some(Edit::delete(match selection.is_empty() {
                true => document.state.previous_boundary(selection.head)..selection.head,
                false => selection.range(),
            }))
        })
    }

    /// Delete: each selection, or the character after each caret.
    pub fn delete(&mut self) -> bool {
        self.replace_each(EditKind::Deleting, |document, selection| {
            Some(Edit::delete(match selection.is_empty() {
                true => selection.head..document.state.next_boundary(selection.head),
                false => selection.range(),
            }))
        })
    }

    /// Each selection, or the word before each caret.
    pub fn delete_word_left(&mut self) -> bool {
        self.replace_each(EditKind::Other, |document, selection| {
            Some(Edit::delete(match selection.is_empty() {
                true => document.state.previous_word(selection.head)..selection.head,
                false => selection.range(),
            }))
        })
    }

    /// Each selection, or from each caret back to its line's start.
    pub fn delete_to_line_start(&mut self) -> bool {
        self.replace_each(EditKind::Other, |document, selection| {
            Some(Edit::delete(match selection.is_empty() {
                true => document.line_range(document.row_of(selection.head)).start..selection.head,
                false => selection.range(),
            }))
        })
    }

    /// Deletes every selection (Cut).
    pub fn delete_selections(&mut self) -> bool {
        self.replace_each(EditKind::Other, |_, selection| Some(Edit::delete(selection.range())))
    }

    /// Pastes `text` at every selection; when it has as many lines as there
    /// are selections, a line at each, as CodeMirror pastes.
    pub fn paste(&mut self, text: &str) -> bool {
        let text = line_breaks(text);
        let lines: Vec<&str> = text.split('\n').collect();
        let spread = self.selections.count() > 1 && lines.len() == self.selections.count();
        let mut index = 0;
        self.replace_each(EditKind::Other, |_, selection| {
            let insert = if spread { lines[index] } else { text.as_str() };
            index += 1;
            Some(Edit::replace(selection.range(), insert))
        })
    }

    /// Enter: a new line at each selection, indented as the line it is on.
    pub fn newline(&mut self) -> bool {
        self.replace_each(EditKind::Other, |document, selection| {
            let line = document.line(document.row_of(selection.range().start));
            let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            Some(Edit::replace(selection.range(), format!("\n{indent}")))
        })
    }

    /// The lines `selection` touches.
    fn selected_rows(&self, selection: Selection) -> Range<usize> {
        let range = selection.range();
        let first = self.row_of(range.start);
        let mut last = self.row_of(range.end);
        // A selection ending at a line's very start leaves that line out.
        if last > first && self.starts[last] == range.end {
            last -= 1;
        }
        first..last + 1
    }

    /// Whether Tab at `selection` inserts an indent rather than indenting lines:
    /// a caret, or part of one line.
    fn indents_in_place(&self, selection: Selection) -> bool {
        let rows = self.selected_rows(selection);
        selection.is_empty() || rows.len() == 1 && self.line_range(rows.start) != selection.range()
    }

    /// Tab: indents the selected lines, or inserts an indent at each caret.
    pub fn indent(&mut self) -> bool {
        if self.selections.all().iter().all(|selection| self.indents_in_place(*selection)) {
            return self.replace_each(EditKind::Other, |_, selection| Some(Edit::replace(selection.range(), INDENT)));
        }
        self.reindent(|line| format!("{INDENT}{line}"))
    }

    /// Shift+Tab: takes one indent off the selected lines.
    pub fn outdent(&mut self) -> bool {
        self.reindent(|line| {
            let blanks = line.len() - line.trim_start_matches(' ').len();
            line[blanks.min(INDENT.len())..].to_owned()
        })
    }

    /// Rewrites the lines the selections touch, then selects each selection's lines.
    fn reindent(&mut self, rewrite: impl Fn(&str) -> String) -> bool {
        let spans: Vec<Range<usize>> =
            self.selections.all().iter().map(|selection| self.selected_rows(*selection)).collect();
        let mut rows: Vec<usize> = spans.iter().flat_map(Clone::clone).collect();
        rows.dedup();
        let changes: Vec<Change> =
            rows.iter().map(|row| Change::new(self.line_range(*row), rewrite(self.line(*row)))).collect();
        let selections = spans
            .iter()
            .map(|rows| {
                let start = map(self.line_range(rows.start).start, &changes, false);
                let end = map(self.line_range(rows.end - 1).end, &changes, true);
                Selection::new(start, end)
            })
            .collect();
        let selections = Selections::new(selections, self.selections.primary_index());
        self.change(EditKind::Other, changes, Then::Select(selections))
    }

    /// ⌘/: comments the selected lines out with `token`, or back in when
    /// each already is, as CodeMirror's `toggleLineComment`: the token goes
    /// at the lines' shallowest indent, followed by a space, blank lines
    /// left alone unless only one line is selected.
    pub fn toggle_comment(&mut self, token: &str) -> bool {
        struct Line {
            start: usize,
            comment: Option<usize>,
            indent: usize,
            empty: bool,
            single: bool,
            len: usize,
        }
        let mut lines: Vec<Line> = Vec::new();
        let mut previous: Option<usize> = None;
        for selection in self.selections.all() {
            let Range { start: from, end: to } = selection.range();
            let first = lines.len();
            let mut shallowest = usize::MAX;
            let mut row = self.row_of(from);
            while row < self.line_count() && self.starts[row] <= to {
                let range = self.line_range(row);
                if previous.is_none_or(|previous| row > previous) && (from == to || to > range.start) {
                    previous = Some(row);
                    let line = self.line(row);
                    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
                    if indent < line.len() {
                        shallowest = shallowest.min(indent);
                    }
                    let comment = line[indent..].starts_with(token).then_some(indent);
                    lines.push(Line {
                        start: range.start,
                        comment,
                        indent,
                        empty: indent == line.len(),
                        single: false,
                        len: line.len(),
                    });
                }
                row += 1;
            }
            if shallowest < usize::MAX {
                for line in lines[first..].iter_mut().filter(|line| line.indent < line.len) {
                    line.indent = shallowest;
                }
            }
            if lines.len() == first + 1 {
                lines[first].single = true;
            }
        }
        let text = self.text();
        if lines.iter().any(|line| line.comment.is_none() && (!line.empty || line.single)) {
            let changes = lines
                .iter()
                .filter(|line| line.single || !line.empty)
                .map(|line| Change::new(line.start + line.indent..line.start + line.indent, format!("{token} ")))
                .collect();
            return self.change(EditKind::Other, changes, Then::Map { after: true });
        }
        let changes: Vec<Change> = lines
            .iter()
            .filter_map(|line| {
                let from = line.start + line.comment?;
                let mut to = from + token.len();
                if text[to..].starts_with(' ') {
                    to += 1;
                }
                Some(Change::new(from..to, ""))
            })
            .collect();
        !changes.is_empty() && self.change(EditKind::Other, changes, Then::Map { after: false })
    }

    // Folds

    /// Hides `range` behind a placeholder; carets inside go to its start.
    pub fn fold(&mut self, range: Range<usize>) -> bool {
        if range.is_empty() || self.folds.iter().any(|fold| fold.start <= range.start && range.end <= fold.end) {
            return false;
        }
        self.folds.retain(|fold| !(range.start <= fold.start && fold.end <= range.end));
        let at = self.folds.partition_point(|fold| fold.start < range.start);
        self.folds.insert(at, range.clone());
        let inside = |offset: usize| if range.start < offset && offset < range.end { range.start } else { offset };
        let all = self.selections.all().iter().map(|s| Selection::new(inside(s.anchor), inside(s.head))).collect();
        self.selections = Selections::new(all, self.selections.primary_index());
        self.sync();
        true
    }

    /// Shows again the folds `which` picks; whether any was.
    pub fn unfold(&mut self, which: impl Fn(&Range<usize>) -> bool) -> bool {
        let before = self.folds.len();
        self.folds.retain(|fold| !which(fold));
        self.folds.len() != before
    }

    /// The widest line, in characters, for the horizontal scroll.
    pub fn widest_line(&self) -> usize {
        (0..self.line_count()).map(|row| self.line(row).chars().count()).max().unwrap_or(0)
    }
}

/// The one change that turns `before` into `after`: what lies between their
/// common start and common end.
fn difference(before: &str, after: &str) -> Change {
    let prefix = before.bytes().zip(after.bytes()).take_while(|(a, b)| a == b).count();
    let longest = before.len().min(after.len()) - prefix;
    let suffix = before.bytes().rev().zip(after.bytes().rev()).take(longest).take_while(|(a, b)| a == b).count();
    let mut start = prefix;
    while !before.is_char_boundary(start) || !after.is_char_boundary(start) {
        start -= 1;
    }
    let (mut old_end, mut new_end) = (before.len() - suffix, after.len() - suffix);
    while !before.is_char_boundary(old_end) || !after.is_char_boundary(new_end) {
        old_end += 1;
        new_end += 1;
    }
    Change::new(start..old_end, &after[start..new_end])
}

#[cfg(test)]
// One fold or one selection is a one-range list here, not a range to collect.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    /// A document with a caret at each `|`, or a selection between two `|` in a `[]`.
    fn doc(marked: &str) -> Document {
        let mut selections = Vec::new();
        let mut text = String::new();
        let mut open: Option<usize> = None;
        for c in marked.chars() {
            match c {
                '|' if open.is_none() => selections.push(Selection::caret(text.len())),
                '«' => open = Some(text.len()),
                '»' => selections.push(Selection::new(open.take().expect("an opening «"), text.len())),
                _ => text.push(c),
            }
        }
        let mut document = Document::new(&text);
        document.set_selections(Selections::new(selections, 0));
        document
    }

    /// The text with each caret (`|`) or selection (`«…»`) marked again.
    fn shown(document: &Document) -> String {
        let mut text = document.text().to_owned();
        for selection in document.selections().all().iter().rev() {
            let range = selection.range();
            if range.is_empty() {
                text.insert(range.start, '|');
            } else {
                text.insert(range.end, '»');
                text.insert(range.start, '«');
            }
        }
        text
    }

    const SQL: BracketRules = BracketRules { brackets: &['(', '[', '{', '\'', '"', '`'], in_string: &|_| false };

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
        let map = document.lines_map();
        document.move_vertically(1, false, &map);
        assert_eq!(shown(&document), "SELECT id,\nx|\nFROM orders");
        document.move_vertically(1, false, &map);
        assert_eq!(shown(&document), "SELECT id,\nx\nFROM order|s", "back to the tenth column");
        document.move_vertically(1, false, &map);
        assert_eq!(document.primary().head, document.text().len(), "down on the last line goes to its end");
        document.move_vertically(-9, true, &map);
        assert_eq!(document.primary().range(), 0..document.text().len(), "up past the top selects to the start");
    }

    #[test]
    fn home_goes_to_the_indent_then_the_line_start() {
        let mut document = doc("a\n    sel|ect");
        document.line_home(false);
        assert_eq!(shown(&document), "a\n    |select");
        document.line_home(false);
        assert_eq!(shown(&document), "a\n|    select");
        document.line_end(true);
        assert_eq!(shown(&document), "a\n«    select»");
    }

    #[test]
    fn enter_keeps_the_indent_and_tab_indents() {
        let mut document = doc("  SELECT|");
        document.newline();
        assert_eq!(shown(&document), "  SELECT\n  |");
        document.indent();
        assert_eq!(shown(&document), "  SELECT\n    |");
        let mut document = doc("«a\nb\n»c");
        document.indent();
        assert_eq!(shown(&document), "«  a\n  b»\nc", "a selection ending at a line's start leaves it out");
        document.outdent();
        document.outdent();
        assert_eq!(shown(&document), "«a\nb»\nc");
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
        assert_eq!(shown(&document), "SELECT «total_due» FROM t");
        let mut document = doc("a\nbc|\nd");
        document.select_line(1);
        assert_eq!(shown(&document), "a\n«bc\n»d");
        document.replace_all("SELECT 1");
        assert_eq!(shown(&document), "SELECT 1|");
        document.undo();
        assert_eq!(document.text(), "a\nbc\nd");
    }

    // Multiple selections

    #[test]
    fn typing_and_deleting_apply_at_every_caret() {
        let mut document = doc("a|\nb|\nc|");
        document.type_text("x", &SQL);
        assert_eq!(shown(&document), "ax|\nbx|\ncx|");
        document.backspace(&[]);
        document.backspace(&[]);
        assert_eq!(shown(&document), "|\n|\n|");
        document.undo();
        assert_eq!(shown(&document), "ax|\nbx|\ncx|", "a run of deleting is undone at once");
        document.word_left(true);
        assert_eq!(shown(&document), "«ax»\n«bx»\n«cx»");
        document.delete();
        assert_eq!(shown(&document), "|\n|\n|");
        let mut document = doc("ab|cd|");
        document.left(false);
        document.delete();
        assert_eq!(shown(&document), "a|c|", "Delete takes the character after each caret");
        let mut document = doc("x|\ny|");
        document.newline();
        assert_eq!(shown(&document), "x\n|\ny\n|");
    }

    #[test]
    fn carets_that_meet_join() {
        let mut document = doc("ab|c|");
        document.backspace(&[]);
        document.backspace(&[]);
        assert_eq!(shown(&document), "|", "two carets deleting into each other become one");
        assert_eq!(document.selections().count(), 1);
    }

    #[test]
    fn one_caret_deletes_and_moves_as_before() {
        let mut document = doc("SELECT total_due|");
        document.delete_word_left();
        assert_eq!(shown(&document), "SELECT |");
        document.type_text("x", &SQL);
        document.delete_to_line_start();
        assert_eq!(shown(&document), "|");
        let mut document = doc("a«bc»d");
        document.backspace(SQL.brackets);
        assert_eq!(shown(&document), "a|d", "a selection goes whole");
        document.delete();
        assert_eq!(shown(&document), "a|");
        document.left(false);
        document.right(true);
        assert_eq!(shown(&document), "«a»");
        document.right(false);
        assert_eq!(shown(&document), "a|", "Right ends a selection at its end");
        document.paste("1\n2");
        assert_eq!(shown(&document), "a1\n2|", "one caret pastes every line");
        assert!(document.selected_text().is_none(), "nothing selected, nothing to copy");
        document.word_left(true);
        assert_eq!(document.selected_text().as_deref(), Some("2"));
        assert!(document.delete_selections());
        assert_eq!(shown(&document), "a1\n|", "Cut takes the selection");
    }

    #[test]
    fn select_next_occurrence_adds_the_next_match() {
        let mut document = doc("id, ids, id|, x.id");
        assert!(document.select_next_occurrence());
        assert_eq!(shown(&document), "id, ids, «id», x.id", "first the word at the caret");
        assert!(document.select_next_occurrence());
        assert_eq!(shown(&document), "id, ids, «id», x.«id»", "then its next whole-word match");
        assert!(document.select_next_occurrence());
        assert_eq!(shown(&document), "«id», ids, «id», x.«id»", "wrapping round, skipping ids");
        assert!(!document.select_next_occurrence(), "nothing left");
        assert_eq!(document.primary().range(), 9..11, "the first selection stays primary");
        document.type_text("key", &SQL);
        assert_eq!(document.text(), "key, ids, key, x.key");
    }

    #[test]
    fn adds_carets_above_and_below_at_the_column() {
        let mut document = doc("abc\nab|c\na\nabcd");
        let map = document.lines_map();
        assert!(document.add_caret_vertically(true, &map));
        let map = document.lines_map();
        assert!(document.add_caret_vertically(true, &map));
        assert_eq!(shown(&document), "abc\nab|c\na|\nab|cd", "the column kept past a short line");
        assert_eq!(document.primary().head, 12, "the last caret added is primary");
        let map = document.lines_map();
        assert!(document.add_caret_vertically(false, &map));
        assert!(shown(&document).starts_with("ab|c\n"));
        assert!(document.single_selection());
        assert_eq!(document.selections().count(), 1);
    }

    #[test]
    fn pastes_a_line_at_each_caret_when_the_counts_match() {
        let mut document = doc("|\n|");
        document.paste("a\nb");
        assert_eq!(shown(&document), "a|\nb|");
        document.paste("xyz");
        assert_eq!(shown(&document), "axyz|\nbxyz|");
        document.word_left(true);
        assert_eq!(document.selected_text().as_deref(), Some("axyz\nbxyz"), "copy joins the selections");
    }

    // Brackets

    #[test]
    fn closes_brackets_and_quotes_where_codemirror_does() {
        let mut document = doc("|");
        document.type_text("(", &SQL);
        assert_eq!(shown(&document), "(|)");
        document.type_text("'", &SQL);
        assert_eq!(shown(&document), "('|')");
        document.type_text("a", &SQL);
        document.type_text("'", &SQL);
        assert_eq!(shown(&document), "('a'|)", "a closing quote typed for the user is stepped over");
        document.type_text(")", &SQL);
        assert_eq!(shown(&document), "('a')|", "and so is a closing bracket");
        let mut document = doc("|x");
        document.type_text("(", &SQL);
        assert_eq!(shown(&document), "(|x", "no closing bracket before a word");
        let mut document = doc("a|");
        document.type_text("'", &SQL);
        assert_eq!(shown(&document), "a'|", "no closing quote after a word");
        let mut document = doc("|;");
        document.type_text("[", &SQL);
        assert_eq!(shown(&document), "[|];", "before ; a bracket closes");
        let in_string = BracketRules { brackets: SQL.brackets, in_string: &|_| true };
        let mut document = doc("|");
        document.type_text("\"", &in_string);
        assert_eq!(shown(&document), "\"|", "inside a string a quote doesn't pair");
        let mut document = doc("a|)");
        document.type_text(")", &SQL);
        assert_eq!(shown(&document), "a)|)", "a closing bracket the user typed isn't stepped over");
    }

    #[test]
    fn wraps_a_selection_and_deletes_a_pair_with_backspace() {
        let mut document = doc("«id» = 1");
        document.type_text("(", &SQL);
        assert_eq!(shown(&document), "(«id») = 1", "the selection stays selected inside");
        let mut document = doc("f(|)");
        document.backspace(SQL.brackets);
        assert_eq!(shown(&document), "f|", "an empty pair goes at once");
        let mut document = doc("f(|x)");
        document.backspace(SQL.brackets);
        assert_eq!(shown(&document), "f|x)");
        let mut document = doc("|\n|");
        document.type_text("{", &SQL);
        assert_eq!(shown(&document), "{|}\n{|}", "at every caret");
        let json = BracketRules { brackets: &['[', '{', '"'], in_string: &|_| false };
        let mut document = doc("|");
        document.type_text("(", &json);
        assert_eq!(shown(&document), "(|", "JSON doesn't close (");
    }

    // Comments

    #[test]
    fn toggles_line_comments_at_the_shallowest_indent() {
        let mut document = doc("«SELECT 1\n  FROM t\n\nWHERE x»");
        assert!(document.toggle_comment("--"));
        assert_eq!(document.text(), "-- SELECT 1\n--   FROM t\n\n-- WHERE x", "blank lines left alone");
        assert!(document.toggle_comment("--"));
        assert_eq!(document.text(), "SELECT 1\n  FROM t\n\nWHERE x", "back as it was");
        let mut document = doc("  a|\n  -- b|");
        document.toggle_comment("//");
        assert_eq!(shown(&document), "  // a|\n  // -- b|", "one line uncommented comments every line");
        let mut document = doc("  -- a|");
        document.toggle_comment("--");
        assert_eq!(shown(&document), "  a|");
        let mut document = doc("|");
        document.toggle_comment("--");
        assert_eq!(shown(&document), "-- |", "a single blank line is commented too");
    }

    // Folds

    #[test]
    fn folds_hide_text_until_a_caret_goes_in() {
        let mut document = doc("{\n  a\n}|");
        assert!(document.fold(1..6));
        assert_eq!(document.lines_map().row_count(), 1);
        document.type_text("x", &SQL);
        assert_eq!(document.folds(), [1..6], "an edit elsewhere keeps the fold");
        document.place(3, false);
        assert!(document.folds().is_empty(), "a caret inside opens it");
        assert!(document.fold(1..6));
        assert_eq!(document.primary().head, 1, "folding a caret's text puts it before the fold");
        document.type_text("y", &SQL);
        assert_eq!(document.folds(), [2..7], "typing before a fold moves it");
        document.undo();
        assert_eq!((document.text(), document.folds()), ("{\n  a\n}x", &[1..6][..]), "undo brings back its folds");
    }
}
