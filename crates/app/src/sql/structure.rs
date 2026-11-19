//! A text's brackets and what folds, read once per change: where each `(`,
//! `[` and `{` outside strings and comments is closed, where strings run
//! (typing a quote inside one doesn't close it), and the ranges the editor's
//! gutter folds, as CodeMirror's languages fold them: in SQL a statement
//! over several lines (from the end of its first line) and a block comment;
//! in JSON an object or an array (inside its brackets); in mongosh those and
//! its block comments. Plain text has no strings and folds nothing.

use std::ops::Range;

use super::highlight::{Kind, LineState, tokenize};
use super::statements::statements;
use super::{Dialect, Syntax};

/// A bracket outside strings and comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bracket {
    pub at: usize,
    pub ch: char,
    /// The bracket that closes (or opens) it, by offset; `None` when none does.
    pub partner: Option<usize>,
    /// Whether the partner is the matching kind (`(` with `)`).
    pub matched: bool,
}

impl Bracket {
    fn opens(self) -> bool {
        matches!(self.ch, '(' | '[' | '{')
    }
}

/// A bracket next to a caret and the one it pairs with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BracketMatch {
    pub at: usize,
    pub partner: Option<usize>,
    pub matched: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Structure {
    /// In text order.
    brackets: Vec<Bracket>,
    /// Strings and quoted names, in text order.
    strings: Vec<Quoted>,
    /// What can fold, by start.
    folds: Vec<Range<usize>>,
}

impl Structure {
    pub fn analyze(text: &str, syntax: Syntax) -> Self {
        let mut structure = match syntax {
            Syntax::Sql(dialect) => sql(text, dialect),
            Syntax::Json => script(text, false),
            Syntax::Mongosh => script(text, true),
            Syntax::Plain => Self { brackets: plain_brackets(text), ..Self::default() },
        };
        pair(&mut structure.brackets);
        if matches!(syntax, Syntax::Json | Syntax::Mongosh) {
            // An object's or an array's inside, when it runs over lines.
            let inside = structure.brackets.iter().filter(|bracket| matches!(bracket.ch, '{' | '['));
            let folds = inside
                .filter(|bracket| bracket.matched)
                .filter_map(|bracket| Some(bracket.at + 1..bracket.partner?))
                .filter(|range| text[range.clone()].contains('\n'));
            structure.folds.extend(folds.collect::<Vec<_>>());
        }
        structure.folds.sort_by_key(|range| (range.start, std::cmp::Reverse(range.end)));
        structure
    }

    pub fn brackets(&self) -> &[Bracket] {
        &self.brackets
    }

    pub fn folds(&self) -> &[Range<usize>] {
        &self.folds
    }

    /// Whether `offset` is inside a string or a quoted name: past its
    /// opening quote and before its closing one (or the end of one left open).
    pub fn in_string(&self, offset: usize) -> bool {
        let index = self.strings.partition_point(|string| string.range.start < offset);
        index > 0 && {
            let string = &self.strings[index - 1];
            offset < string.range.end || offset == string.range.end && !string.closed
        }
    }

    fn bracket(&self, at: usize) -> Option<&Bracket> {
        self.brackets.binary_search_by_key(&at, |bracket| bracket.at).ok().map(|index| &self.brackets[index])
    }

    /// The bracket a caret at `caret` is beside, as CodeMirror looks for it:
    /// a closing one before it, an opening one before it, an opening one
    /// after it, then a closing one after it.
    pub fn matching(&self, caret: usize) -> Option<BracketMatch> {
        let before = caret.checked_sub(1).and_then(|at| self.bracket(at));
        let after = self.bracket(caret);
        let found = [before.filter(|b| !b.opens()), before.filter(|b| b.opens()), after.filter(|b| b.opens())]
            .into_iter()
            .chain([after.filter(|b| !b.opens())])
            .flatten()
            .next()?;
        Some(BracketMatch { at: found.at, partner: found.partner, matched: found.matched })
    }

    /// The fold that starts on the line `line` (a byte range, without its
    /// break) and runs past it: the outermost, as CodeMirror picks.
    pub fn fold_on_line(&self, line: Range<usize>) -> Option<Range<usize>> {
        let first = self.folds.partition_point(|fold| fold.start < line.start);
        self.folds[first..]
            .iter()
            .take_while(|fold| fold.start <= line.end)
            .filter(|fold| fold.end > line.end)
            .max_by_key(|fold| fold.end)
            .cloned()
    }
}

/// A string or a quoted name, and whether its closing quote ends it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Quoted {
    range: Range<usize>,
    closed: bool,
}

impl Quoted {
    fn new(text: &str, range: Range<usize>) -> Self {
        let quoted = &text[range.clone()];
        let open = quoted.chars().next().unwrap_or(' ');
        let close = if open == '[' { ']' } else { open };
        Self { closed: quoted.chars().count() >= 2 && quoted.ends_with(close), range }
    }
}

/// Pairs each opening bracket with the next closing one at its depth.
fn pair(brackets: &mut [Bracket]) {
    let mut open: Vec<usize> = Vec::new();
    for index in 0..brackets.len() {
        if brackets[index].opens() {
            open.push(index);
            continue;
        }
        let Some(start) = open.pop() else { continue };
        let matched = matches!((brackets[start].ch, brackets[index].ch), ('(', ')') | ('[', ']') | ('{', '}'));
        let (start_at, end_at) = (brackets[start].at, brackets[index].at);
        brackets[start].partner = Some(end_at);
        brackets[start].matched = matched;
        brackets[index].partner = Some(start_at);
        brackets[index].matched = matched;
    }
}

fn is_bracket(c: char) -> bool {
    matches!(c, '(' | ')' | '[' | ']' | '{' | '}')
}

fn bracket(at: usize, ch: char) -> Bracket {
    Bracket { at, ch, partner: None, matched: false }
}

/// Text without a language: every bracket counts.
fn plain_brackets(text: &str) -> Vec<Bracket> {
    text.char_indices().filter(|(_, c)| is_bracket(*c)).map(|(at, c)| bracket(at, c)).collect()
}

/// SQL, read with the highlighter's tokens: brackets outside strings, quoted
/// names and comments; statements and block comments over lines fold.
fn sql(text: &str, dialect: Dialect) -> Structure {
    let mut structure = Structure::default();
    let mut strings: Vec<Range<usize>> = Vec::new();
    let mut comments: Vec<Range<usize>> = Vec::new();
    let mut state = LineState::Normal;
    let mut line_start = 0;
    for line in text.split('\n') {
        let continues = state.clone();
        let (spans, next) = tokenize(line, state, dialect);
        for span in spans {
            let range = line_start + span.range.start..line_start + span.range.end;
            let carried = span.range.start == 0;
            match span.kind {
                Kind::String | Kind::Identifier => {
                    let open = carried && matches!(continues, LineState::String(_) | LineState::Dollar(_));
                    let open = open || carried && matches!(continues, LineState::Identifier(_));
                    extend(&mut strings, range, open);
                }
                Kind::Comment => extend(&mut comments, range, carried && continues == LineState::BlockComment),
                _ => structure.brackets.extend(
                    line[span.range.clone()]
                        .char_indices()
                        .filter(|(_, c)| is_bracket(*c))
                        .map(|(at, c)| bracket(range.start + at, c)),
                ),
            }
        }
        state = next;
        line_start += line.len() + 1;
    }
    structure.strings = strings.into_iter().map(|range| Quoted::new(text, range)).collect();
    for mut statement in statements(text, dialect) {
        // A statement starts at its first word, past comments before it (as the parser skips them).
        loop {
            statement.start +=
                text[statement.start..statement.end].len() - text[statement.start..statement.end].trim_start().len();
            match comments.iter().find(|comment| comment.start == statement.start) {
                Some(comment) => statement.start = comment.end.min(statement.end),
                None => break,
            }
        }
        if statement.is_empty() {
            continue;
        }
        let first_line_end = text[statement.start..].find('\n').map_or(text.len(), |at| statement.start + at);
        let mut from = (statement.start + 100).min(first_line_end);
        while !text.is_char_boundary(from) {
            from -= 1;
        }
        if statement.end > first_line_end {
            structure.folds.push(from..statement.end);
        }
    }
    structure.folds.extend(block_comment_folds(text, &comments));
    structure
}

/// Adds `range` to `ranges`, joined to the last one when it carries it on from the line before.
fn extend(ranges: &mut Vec<Range<usize>>, range: Range<usize>, carried: bool) {
    match ranges.last_mut() {
        Some(last) if carried => last.end = range.end,
        _ => ranges.push(range),
    }
}

/// A block comment over several lines folds inside its `/*` and `*/`.
fn block_comment_folds<'a>(text: &'a str, comments: &'a [Range<usize>]) -> impl Iterator<Item = Range<usize>> + 'a {
    comments
        .iter()
        .filter(|range| text[(*range).clone()].starts_with("/*") && text[(*range).clone()].contains('\n'))
        .map(|range| {
            let end = if text[range.clone()].ends_with("*/") && range.len() >= 4 { range.end - 2 } else { range.end };
            range.start + 2..end
        })
}

/// JSON, or mongosh's JavaScript (`comments`): strings in `"` (and `'` and
/// `` ` `` in mongosh), and mongosh's `//` and `/* */` comments.
fn script(text: &str, comments: bool) -> Structure {
    let mut structure = Structure::default();
    let mut blocks = Vec::new();
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let c = bytes[at];
        match c {
            b'"' | b'\'' | b'`' if c == b'"' || comments => {
                let start = at;
                at += 1;
                while at < bytes.len() && bytes[at] != c {
                    // A line break ends an unclosed string, except a template's.
                    if bytes[at] == b'\n' && c != b'`' {
                        break;
                    }
                    at += if bytes[at] == b'\\' { 2 } else { 1 };
                }
                at = (at + 1).min(bytes.len());
                structure.strings.push(Quoted::new(text, start..at));
                continue;
            }
            b'/' if comments && bytes.get(at + 1) == Some(&b'/') => {
                at = text[at..].find('\n').map_or(bytes.len(), |end| at + end);
                continue;
            }
            b'/' if comments && bytes.get(at + 1) == Some(&b'*') => {
                let end = text[at + 2..].find("*/").map_or(bytes.len(), |end| at + 2 + end + 2);
                blocks.push(at..end);
                at = end;
                continue;
            }
            b'(' | b')' | b'[' | b']' | b'{' | b'}' => structure.brackets.push(bracket(at, c as char)),
            _ => {}
        }
        at += 1;
    }
    structure.folds.extend(block_comment_folds(text, &blocks));
    structure
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folded(text: &str, syntax: Syntax) -> Vec<&str> {
        Structure::analyze(text, syntax).folds().iter().map(|range| &text[range.clone()]).collect()
    }

    #[test]
    fn pairs_brackets_outside_strings_and_comments() {
        let text = "SELECT f(a, '(', \"x)\") -- )\nFROM t /* ( */ WHERE (b)";
        let structure = Structure::analyze(text, Syntax::Sql(Dialect::Postgres));
        let shown: Vec<(char, Option<usize>)> =
            structure.brackets().iter().map(|bracket| (bracket.ch, bracket.partner)).collect();
        let open = text.find("f(").unwrap() + 1;
        let close = text.find("\") --").unwrap() + 1;
        let (where_open, where_close) = (text.rfind('(').unwrap(), text.rfind(')').unwrap());
        assert_eq!(
            shown,
            [('(', Some(close)), (')', Some(open)), ('(', Some(where_close)), (')', Some(where_open))],
            "brackets in strings, quoted names and comments don't count"
        );
        assert!(structure.in_string(text.find("'(").unwrap() + 1));
        assert!(!structure.in_string(open));
    }

    #[test]
    fn finds_the_bracket_beside_a_caret_as_codemirror_does() {
        let text = "(ab[1]) )";
        let structure = Structure::analyze(text, Syntax::Plain);
        let found = |caret| structure.matching(caret).map(|found| (found.at, found.partner, found.matched));
        assert_eq!(found(0), Some((0, Some(6), true)), "an opening bracket after the caret");
        assert_eq!(found(1), Some((0, Some(6), true)), "an opening bracket before the caret");
        assert_eq!(found(7), Some((6, Some(0), true)), "a closing bracket before the caret first");
        assert_eq!(found(6), Some((5, Some(3), true)), "a closing one before, over a closing one after");
        assert_eq!(found(2), None, "nothing beside the caret");
        assert_eq!(found(9), Some((8, None, false)), "a closing bracket nothing opened");
        let mismatched = Structure::analyze("(]", Syntax::Plain);
        assert_eq!(mismatched.matching(0).map(|found| found.matched), Some(false));
    }

    #[test]
    fn folds_statements_and_block_comments_in_sql() {
        let text = "SELECT a,\n  b\nFROM t;\nSELECT 1;\n/* one\n two */";
        assert_eq!(
            folded(text, Syntax::Sql(Dialect::Postgres)),
            ["\n  b\nFROM t", " one\n two "],
            "a statement folds from its first line's end; a one-line one doesn't fold"
        );
        let structure = Structure::analyze(text, Syntax::Sql(Dialect::Postgres));
        assert_eq!(structure.fold_on_line(0..9), Some(9..20));
        assert_eq!(structure.fold_on_line(10..13), None, "nothing starts on the second line");
    }

    #[test]
    fn folds_objects_and_arrays_in_json_and_blocks_in_mongosh() {
        let text = "{\n  \"a\": [1, 2],\n  \"b\": [\n    \"}\"\n  ]\n}";
        assert_eq!(folded(text, Syntax::Json), [&text[1..text.len() - 1], "\n    \"}\"\n  "]);
        let structure = Structure::analyze(text, Syntax::Json);
        assert_eq!(structure.fold_on_line(0..1), Some(1..text.len() - 1), "the outermost that starts on the line");
        let script = "db.t.find({\n  a: 1 // }\n})\n/* a\n b */";
        assert_eq!(folded(script, Syntax::Mongosh), ["\n  a: 1 // }\n", " a\n b "]);
        assert!(folded(text, Syntax::Plain).is_empty(), "plain text folds nothing");
    }
}
