//! The editor's selections: one or more, each a caret or a stretch of text,
//! in text order and never overlapping, one of them primary (the one input
//! methods, completion and Run read). Every edit and move applies to each,
//! as CodeMirror's `EditorSelection` does; and the changes an edit makes,
//! which carry offsets (selections, folds) over to the new text.

use std::ops::Range;

/// A caret (`anchor == head`) or a selected stretch, by byte offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Where the selection was started; it stays as the head moves.
    pub anchor: usize,
    /// Where the caret is.
    pub head: usize,
    /// The column Up and Down keep to across shorter lines, once one moved.
    pub goal: Option<usize>,
}

impl Selection {
    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head, goal: None }
    }

    pub fn caret(at: usize) -> Self {
        Self::new(at, at)
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// The caret is at the start (the selection was made leftwards).
    pub fn is_reversed(&self) -> bool {
        self.head < self.anchor
    }

    /// The selection with its ends where `map` takes them.
    fn mapped(self, mut map: impl FnMut(usize) -> usize) -> Self {
        Self { anchor: map(self.anchor), head: map(self.head), goal: self.goal }
    }
}

/// Every selection, in text order, and which is primary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selections {
    all: Vec<Selection>,
    primary: usize,
}

impl Default for Selections {
    fn default() -> Self {
        Self::single(Selection::caret(0))
    }
}

impl Selections {
    pub fn single(selection: Selection) -> Self {
        Self { all: vec![selection], primary: 0 }
    }

    /// `all` sorted, with those that overlap joined (as CodeMirror joins
    /// them: overlapping ones, or a caret touching another selection).
    pub fn new(all: Vec<Selection>, primary: usize) -> Self {
        assert!(!all.is_empty(), "there is always a selection");
        let primary = primary.min(all.len() - 1);
        let mut indexed: Vec<(usize, Selection)> = all.into_iter().enumerate().collect();
        indexed.sort_by_key(|(_, selection)| (selection.range().start, selection.range().end));
        let mut joined: Vec<(bool, Selection)> = Vec::with_capacity(indexed.len());
        for (index, selection) in indexed {
            let is_primary = index == primary;
            if let Some((was_primary, last)) = joined.last_mut() {
                let (previous, range) = (last.range(), selection.range());
                let overlaps = if range.is_empty() || previous.is_empty() {
                    range.start <= previous.end
                } else {
                    range.start < previous.end
                };
                if overlaps {
                    let (from, to) = (previous.start.min(range.start), previous.end.max(range.end));
                    *last = if selection.is_reversed() { Selection::new(to, from) } else { Selection::new(from, to) };
                    *was_primary |= is_primary;
                    continue;
                }
            }
            joined.push((is_primary, selection));
        }
        let primary = joined.iter().position(|(is_primary, _)| *is_primary).unwrap_or(0);
        Self { all: joined.into_iter().map(|(_, selection)| selection).collect(), primary }
    }

    pub fn all(&self) -> &[Selection] {
        &self.all
    }

    pub fn count(&self) -> usize {
        self.all.len()
    }

    pub fn primary(&self) -> Selection {
        self.all[self.primary]
    }

    pub fn primary_index(&self) -> usize {
        self.primary
    }

    /// The selections with their ends carried over `changes`.
    pub fn map(&self, changes: &[Change], after: bool) -> Self {
        let all = self.all.iter().map(|selection| selection.mapped(|offset| map(offset, changes, after))).collect();
        Self::new(all, self.primary)
    }

    /// The selections with their ends clamped to a text `len` bytes long.
    pub fn clamped(&self, len: usize) -> Self {
        Self::new(self.all.iter().map(|selection| selection.mapped(|offset| offset.min(len))).collect(), self.primary)
    }
}

/// One replacement in an edit: `range` of the old text becomes `insert`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub range: Range<usize>,
    pub insert: String,
}

impl Change {
    pub fn new(range: Range<usize>, insert: impl Into<String>) -> Self {
        Self { range, insert: insert.into() }
    }
}

/// `offset` in the text after `changes` (in text order, apart). An offset
/// inside a replaced range goes to the replacement's end when `after`, else
/// to its start; one where text is inserted stays before it unless `after`.
pub fn map(offset: usize, changes: &[Change], after: bool) -> usize {
    let mut shift: isize = 0;
    for change in changes {
        let range = &change.range;
        if offset < range.start {
            break;
        }
        let start = (range.start as isize + shift) as usize;
        let end = start + change.insert.len();
        if range.is_empty() && offset == range.start {
            return if after { end } else { start };
        }
        if offset == range.start {
            return start;
        }
        if offset < range.end {
            return if after { end } else { start };
        }
        shift += change.insert.len() as isize - range.len() as isize;
    }
    (offset as isize + shift) as usize
}

/// `text` with `changes` (in text order, apart) made.
pub fn apply(text: &str, changes: &[Change]) -> String {
    let grows: usize = changes.iter().map(|change| change.insert.len()).sum();
    let mut out = String::with_capacity(text.len() + grows);
    let mut at = 0;
    for change in changes {
        out.push_str(&text[at..change.range.start]);
        out.push_str(&change.insert);
        at = change.range.end;
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_overlapping_selections_and_keeps_the_primary() {
        let selections = Selections::new(
            vec![Selection::caret(9), Selection::new(2, 5), Selection::caret(4), Selection::caret(7)],
            2,
        );
        assert_eq!(selections.all(), [Selection::new(2, 5), Selection::caret(7), Selection::caret(9)]);
        assert_eq!(selections.primary(), Selection::new(2, 5), "the caret joined into the stretch was primary");
        let touching = Selections::new(vec![Selection::new(0, 2), Selection::new(2, 4)], 0);
        assert_eq!(touching.count(), 2, "two stretches that only touch stay apart");
    }

    #[test]
    fn carries_offsets_over_changes() {
        let changes = [Change::new(2..4, "xyz"), Change::new(6..6, "!")];
        assert_eq!(apply("abcdefgh", &changes), "abxyzef!gh");
        assert_eq!(map(1, &changes, false), 1);
        assert_eq!(map(3, &changes, false), 2, "inside a replaced range: its start");
        assert_eq!(map(3, &changes, true), 5, "or its end");
        assert_eq!(map(4, &changes, false), 5, "at its end: after the replacement");
        assert_eq!(map(6, &changes, false), 7, "where text is inserted: before it");
        assert_eq!(map(6, &changes, true), 8, "or after it");
        assert_eq!(map(7, &changes, false), 9);
        assert_eq!(map(2, &changes, false), 2, "at a replaced range's start");
    }
}
