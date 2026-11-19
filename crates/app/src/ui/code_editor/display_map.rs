//! How the text is laid out in rows on screen: one row per line, unless
//! soft wrap splits a long line over several, or a fold hides lines and
//! joins its first row to its last behind a placeholder (`…`).
//!
//! The map is rebuilt when the text, the folds or the wrap width change, not
//! on every frame. Without wrap or folds a row is a line, read straight off
//! the line index. With wrap each line's breaks come from the wrap function
//! the editor passes, which caches them by the line's text, so an edit
//! re-wraps only the lines it changed.

use std::ops::Range;

/// Where a line breaks into rows: each break's offset in the line, where a row starts.
pub type Wrap<'a> = &'a mut dyn FnMut(&str) -> Vec<usize>;

/// What a fold shows in place of the text it hides.
pub const PLACEHOLDER: &str = "…";

/// One row on screen: a stretch of the text, without line breaks, besides
/// those a fold inside it hides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
    /// The line the row starts on.
    pub line: usize,
    /// The row starts its line (it shows the line's number).
    pub first: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DisplayMap {
    rows: Vec<Row>,
    /// Folded ranges, in text order, apart.
    folds: Vec<Range<usize>>,
}

/// A part of a row as shown: some of the text, or a fold's placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(Range<usize>),
    Fold(Range<usize>),
}

impl DisplayMap {
    /// The rows of a text whose lines are `lines` (each a byte range without
    /// its break), with `folds` hidden and long lines broken where `wrap`
    /// says (each break's offset in the line, where a row starts).
    pub fn build(
        text: &str,
        lines: impl ExactSizeIterator<Item = Range<usize>>,
        folds: &[Range<usize>],
        mut wrap: Option<Wrap>,
    ) -> Self {
        let mut rows: Vec<Row> = Vec::with_capacity(lines.len());
        let mut next_fold = 0;
        // The end of a fold being hidden, while rows join across it.
        let mut hidden_to: Option<usize> = None;
        for (line, range) in lines.enumerate() {
            if hidden_to.is_some_and(|to| to > range.end) {
                continue;
            }
            let breaks = match wrap.as_mut() {
                Some(wrap) => wrap(&text[range.clone()]),
                None => Vec::new(),
            };
            let starts = std::iter::once(range.start).chain(breaks.into_iter().map(|at| range.start + at));
            let ends = starts.clone().skip(1).chain(std::iter::once(range.end));
            for (start, end) in starts.zip(ends) {
                match hidden_to {
                    Some(to) if to > end => continue,
                    Some(_) => {
                        // The row the fold ends in joins the row it started in.
                        if let Some(last) = rows.last_mut() {
                            last.end = end;
                        }
                        hidden_to = None;
                    }
                    None => rows.push(Row { start, end, line, first: start == range.start }),
                }
                let Some(row) = rows.last().copied() else { continue };
                // A fold starting in this row hides what follows, up to its end.
                while let Some(fold) = folds.get(next_fold) {
                    if fold.start < row.start {
                        next_fold += 1;
                        continue;
                    }
                    if fold.start > row.end {
                        break;
                    }
                    next_fold += 1;
                    if fold.end > row.end {
                        hidden_to = Some(fold.end);
                        break;
                    }
                }
            }
        }
        if rows.is_empty() {
            rows.push(Row { start: 0, end: 0, line: 0, first: true });
        }
        Self { rows, folds: folds.to_vec() }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn row(&self, index: usize) -> Row {
        self.rows[index.min(self.rows.len() - 1)]
    }

    /// The row `offset` is shown on (one hidden by a fold: the fold's row).
    pub fn row_of(&self, offset: usize) -> usize {
        self.rows.partition_point(|row| row.start <= offset).saturating_sub(1)
    }

    /// The folds hidden inside row `index`.
    pub fn folds_in(&self, index: usize) -> &[Range<usize>] {
        let row = self.row(index);
        let first = self.folds.partition_point(|fold| fold.start < row.start);
        let count = self.folds[first..].iter().take_while(|fold| fold.end <= row.end && fold.start <= row.end).count();
        &self.folds[first..first + count]
    }

    /// Row `index` as shown: its text and placeholders.
    pub fn pieces(&self, index: usize) -> Vec<Piece> {
        let row = self.row(index);
        let mut pieces = Vec::new();
        let mut at = row.start;
        for fold in self.folds_in(index) {
            pieces.push(Piece::Text(at..fold.start));
            pieces.push(Piece::Fold(fold.clone()));
            at = fold.end;
        }
        pieces.push(Piece::Text(at..row.end));
        pieces
    }

    /// Row `index`'s text as shown, a placeholder for each fold.
    pub fn row_text(&self, text: &str, index: usize) -> String {
        self.pieces(index)
            .into_iter()
            .map(|piece| match piece {
                Piece::Text(range) => &text[range],
                Piece::Fold(_) => PLACEHOLDER,
            })
            .collect()
    }

    /// Where `offset` is in row `index`'s shown text, in bytes (a hidden
    /// offset: at its placeholder).
    pub fn display_index(&self, index: usize, offset: usize) -> usize {
        let row = self.row(index);
        let offset = offset.clamp(row.start, row.end);
        let mut shown = 0;
        let mut at = row.start;
        for fold in self.folds_in(index) {
            if offset <= fold.start {
                break;
            }
            shown += fold.start - at;
            if offset < fold.end {
                return shown;
            }
            shown += PLACEHOLDER.len();
            at = fold.end;
        }
        shown + offset - at
    }

    /// The offset at `shown` bytes into row `index`'s shown text; on a
    /// placeholder, the fold's start, or its end past the placeholder's middle.
    pub fn offset_at(&self, index: usize, shown: usize) -> usize {
        let mut left = shown;
        for piece in self.pieces(index) {
            match piece {
                Piece::Text(range) if left <= range.len() => return range.start + left,
                Piece::Text(range) => left -= range.len(),
                Piece::Fold(fold) if left < PLACEHOLDER.len() => {
                    return if left * 2 < PLACEHOLDER.len() { fold.start } else { fold.end };
                }
                Piece::Fold(_) => left -= PLACEHOLDER.len(),
            }
        }
        self.row(index).end
    }

    /// The fold whose placeholder is at `shown` bytes into row `index`.
    pub fn fold_at(&self, index: usize, shown: usize) -> Option<Range<usize>> {
        let mut at = 0;
        for piece in self.pieces(index) {
            let len = match &piece {
                Piece::Text(range) => range.len(),
                Piece::Fold(_) => PLACEHOLDER.len(),
            };
            if let Piece::Fold(fold) = piece
                && (at..at + len).contains(&shown)
            {
                return Some(fold);
            }
            at += len;
        }
        None
    }

    /// The column (in characters of the shown text) `offset` is at in its row.
    pub fn column(&self, text: &str, offset: usize) -> (usize, usize) {
        let index = self.row_of(offset);
        let shown = self.row_text(text, index);
        let at = self.display_index(index, offset).min(shown.len());
        (index, shown[..at].chars().count())
    }

    /// The offset at `column` characters into row `index`, or its end.
    pub fn offset_at_column(&self, text: &str, index: usize, column: usize) -> usize {
        let shown = self.row_text(text, index);
        let at = shown.char_indices().nth(column).map_or(shown.len(), |(at, _)| at);
        let offset = self.offset_at(index, at);
        // A wrapped row's end is the next row's start: stay on this row.
        let row = self.row(index);
        if offset == row.end
            && index + 1 < self.row_count()
            && self.row(index + 1).start == row.end
            && offset > row.start
        {
            let mut before = offset - 1;
            while !text.is_char_boundary(before) {
                before -= 1;
            }
            return before;
        }
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<Range<usize>> {
        let mut start = 0;
        text.split('\n')
            .map(|line| {
                let range = start..start + line.len();
                start = range.end + 1;
                range
            })
            .collect()
    }

    /// Wraps every `width` characters, as a monospaced font at that width would.
    fn every(width: usize) -> impl FnMut(&str) -> Vec<usize> {
        move |line: &str| (width..line.len()).step_by(width).collect()
    }

    fn shown(map: &DisplayMap, text: &str) -> Vec<String> {
        (0..map.row_count()).map(|index| map.row_text(text, index)).collect()
    }

    #[test]
    fn a_row_is_a_line_without_wrap_or_folds() {
        let text = "ab\n\ncd";
        let map = DisplayMap::build(text, lines(text).into_iter(), &[], None);
        assert_eq!(shown(&map, text), ["ab", "", "cd"]);
        assert_eq!(map.row_of(4), 2);
        assert_eq!(map.column(text, 5), (2, 1));
    }

    #[test]
    fn wraps_long_lines_into_rows() {
        let text = "abcdefgh\nij";
        let mut wrap = every(3);
        let map = DisplayMap::build(text, lines(text).into_iter(), &[], Some(&mut wrap));
        assert_eq!(shown(&map, text), ["abc", "def", "gh", "ij"]);
        assert!(map.row(0).first && !map.row(1).first && map.row(3).first);
        assert_eq!(map.row_of(3), 1, "a break belongs to the row it starts");
        assert_eq!(map.offset_at_column(text, 0, 9), 2, "past a wrapped row's end: its last character");
        assert_eq!(map.offset_at_column(text, 2, 9), 8, "the line's last row takes its end");
    }

    #[test]
    fn a_fold_joins_its_rows_behind_a_placeholder() {
        let text = "{\n  a,\n  b\n}\nx";
        let fold = 1..text.find('}').unwrap();
        let map = DisplayMap::build(text, lines(text).into_iter(), std::slice::from_ref(&fold), None);
        assert_eq!(shown(&map, text), ["{…}", "x"]);
        assert_eq!(map.row(1).line, 4);
        assert_eq!(map.row_of(5), 0, "a hidden offset is on the fold's row");
        assert_eq!(map.display_index(0, fold.end), 1 + PLACEHOLDER.len());
        assert_eq!(map.offset_at(0, 1 + PLACEHOLDER.len()), fold.end);
        assert_eq!(map.fold_at(0, 1), Some(fold.clone()));
        let mut wrap = every(2);
        let wrapped = DisplayMap::build(text, lines(text).into_iter(), &[fold], Some(&mut wrap));
        assert_eq!(shown(&wrapped, text), ["{…}", "x"]);
    }
}
