//! Where a script's statements start and end: split at each `;` that is not
//! inside a string, a quoted name or a comment. The console runs the one at
//! the caret.

use std::ops::Range;

use super::Dialect;
use super::highlight::{Kind, LineState, tokenize};

/// The byte ranges of `text`'s statements, without the `;` that ends each or
/// the space around them; blank ones are left out.
pub fn statements(text: &str, dialect: Dialect) -> Vec<Range<usize>> {
    pieces(text, dialect).into_iter().map(|piece| trimmed(text, piece)).filter(|range| !range.is_empty()).collect()
}

/// The statement the caret at `offset` is in, without its `;`. A caret just
/// after a statement on the same line (after its `;`) is in that statement;
/// one on a blank line or in space before a statement is in the next one,
/// or the previous one when there is no next. `None` for a blank script.
pub fn statement_at(text: &str, offset: usize, dialect: Dialect) -> Option<Range<usize>> {
    let offset = offset.min(text.len());
    let pieces = pieces(text, dialect);
    let index = pieces.iter().position(|piece| offset < piece.end).unwrap_or(pieces.len().saturating_sub(1));
    let current = trimmed(text, pieces.get(index)?.clone());
    let previous = || pieces[..index].iter().rev().map(|piece| trimmed(text, piece.clone())).find(|r| !r.is_empty());
    // Only space from the previous `;` to the caret, on the same line: the caret ends that statement.
    let before = &text[pieces[index].start..offset];
    let ends_previous = before.trim().is_empty() && !before.contains('\n');
    if (current.is_empty() || offset < current.start) && ends_previous {
        return previous().or((!current.is_empty()).then_some(current));
    }
    if current.is_empty() { previous() } else { Some(current) }
}

/// `text` cut after each separating `;`: the pieces cover it exactly.
fn pieces(text: &str, dialect: Dialect) -> Vec<Range<usize>> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut line_start = 0;
    let mut state = LineState::Normal;
    for line in text.split('\n') {
        let (spans, next) = tokenize(line, state, dialect);
        // Strings, quoted names and comments are spans of their own, which never separate.
        for span in spans.iter().filter(|span| !matches!(span.kind, Kind::String | Kind::Identifier | Kind::Comment)) {
            for (index, _) in line[span.range.clone()].match_indices(';') {
                let end = line_start + span.range.start + index + 1;
                pieces.push(start..end);
                start = end;
            }
        }
        state = next;
        line_start += line.len() + 1;
    }
    pieces.push(start..text.len());
    pieces
}

/// `range` of `text` without the space around it or its closing `;`.
fn trimmed(text: &str, range: Range<usize>) -> Range<usize> {
    let piece = &text[range.clone()];
    let piece_trimmed = piece.trim_end().strip_suffix(';').unwrap_or(piece.trim_end());
    let start = range.start + (piece_trimmed.len() - piece_trimmed.trim_start().len());
    let end = range.start + piece_trimmed.trim_end().len();
    start..end.max(start)
}

/// Whether `sql` may change which tables or columns exist, so completion reads them again.
pub fn changes_structure(sql: &str) -> bool {
    sql.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| ["create", "alter", "drop", "rename"].iter().any(|verb| word.eq_ignore_ascii_case(verb)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PG: Dialect = Dialect::Postgres;

    fn texts(text: &str) -> Vec<&str> {
        statements(text, PG).into_iter().map(|range| &text[range]).collect()
    }

    fn at(text: &str) -> Option<String> {
        let offset = text.find('|').expect("a caret");
        let text = text.replacen('|', "", 1);
        statement_at(&text, offset, PG).map(|range| text[range].to_owned())
    }

    #[test]
    fn splits_at_semicolons_outside_strings_names_and_comments() {
        let script = "SELECT ';' AS a;\n-- one; two\nSELECT \"x;y\" FROM t; /* ; */ UPDATE t SET a = 1";
        assert_eq!(
            texts(script),
            ["SELECT ';' AS a", "-- one; two\nSELECT \"x;y\" FROM t", "/* ; */ UPDATE t SET a = 1"]
        );
        assert_eq!(texts("  ;\n ; "), Vec::<&str>::new(), "blank statements are left out");
        assert_eq!(statements("SELECT `a;b`; SELECT 2", Dialect::Mysql).len(), 2);
    }

    #[test]
    fn a_semicolon_in_a_dollar_quoted_body_does_not_split() {
        let body = "CREATE FUNCTION f() RETURNS int AS $$\nSELECT 1;\n$$ LANGUAGE sql;\nSELECT f()";
        assert_eq!(texts(body).len(), 2);
    }

    #[test]
    fn finds_the_statement_at_the_caret() {
        assert_eq!(at("SELECT 1;\nSEL|ECT 2;\nSELECT 3;").as_deref(), Some("SELECT 2"));
        assert_eq!(at("SELECT 1;|\nSELECT 2;").as_deref(), Some("SELECT 1"), "just after a statement's `;`");
        assert_eq!(at("SELECT 1;  |\nSELECT 2;").as_deref(), Some("SELECT 1"));
        assert_eq!(at("SELECT 1;\n|\nSELECT 2;").as_deref(), Some("SELECT 2"), "a blank line belongs to the next");
        assert_eq!(at("SELECT 1;\nSELECT 2;\n\n|").as_deref(), Some("SELECT 2"), "or the previous when none follows");
        assert_eq!(at("|SELECT 1").as_deref(), Some("SELECT 1"));
        assert_eq!(at("SELECT\n  id\nFROM t|").as_deref(), Some("SELECT\n  id\nFROM t"));
        assert_eq!(at("  | "), None);
    }

    #[test]
    fn notices_scripts_that_change_the_structure() {
        assert!(changes_structure("create table t (id int)"));
        assert!(changes_structure("SELECT 1; ALTER TABLE t ADD c int"));
        assert!(!changes_structure("SELECT created_at FROM t"));
    }
}
