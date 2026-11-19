//! What the find bar looks for, apart from drawing it, with CodeMirror's
//! search rules: plain text (where `\n`, `\t`, `\r` and `\\` stand for what
//! they escape) or a regular expression, matching case or not, and only
//! whole words or not. A replacement may use `\n` the same way and, with a
//! regular expression, `$&` (the match), `$1`… (its groups) and `$$`.

use std::ops::Range;

use regex::{Captures, Regex, RegexBuilder};

/// The find bar's query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub text: String,
    pub case_sensitive: bool,
    pub regex: bool,
    pub whole_word: bool,
}

/// A query ready to search with; `None` from [`SearchQuery::compile`] when
/// there is nothing to look for or the expression isn't valid.
pub struct Search {
    pattern: Regex,
    whole_word: bool,
    regex: bool,
}

impl SearchQuery {
    pub fn compile(&self) -> Option<Search> {
        if self.text.is_empty() {
            return None;
        }
        let source = if self.regex { self.text.clone() } else { regex::escape(&unquote(&self.text)) };
        let pattern =
            RegexBuilder::new(&source).case_insensitive(!self.case_sensitive).multi_line(true).build().ok()?;
        Some(Search { pattern, whole_word: self.whole_word, regex: self.regex })
    }
}

impl Search {
    /// Every match in `text`, in order; empty matches are skipped.
    pub fn matches(&self, text: &str) -> Vec<Range<usize>> {
        self.pattern
            .find_iter(text)
            .map(|found| found.range())
            .filter(|range| !range.is_empty() && (!self.whole_word || at_word_boundaries(text, range)))
            .collect()
    }

    /// What replaces the match `range` of `text`.
    pub fn replacement(&self, text: &str, range: Range<usize>, replace: &str) -> String {
        let replace = unquote(replace);
        if !self.regex {
            return replace;
        }
        // The match again, anchored where it was found, for its groups.
        let captures = self
            .pattern
            .captures_at(text, range.start)
            .filter(|found| found.get(0).map(|m| m.range()) == Some(range.clone()));
        match captures {
            Some(captures) => expand(&replace, &captures),
            None => replace,
        }
    }
}

/// The first match at or after `from`, else the first one (wrapping round).
pub fn next(matches: &[Range<usize>], from: usize) -> Option<Range<usize>> {
    let index = matches.partition_point(|found| found.start < from);
    matches.get(index).or(matches.first()).cloned()
}

/// The last match ending at or before `to`, else the last one (wrapping round).
pub fn previous(matches: &[Range<usize>], to: usize) -> Option<Range<usize>> {
    let index = matches.partition_point(|found| found.end <= to);
    index.checked_sub(1).and_then(|index| matches.get(index)).or(matches.last()).cloned()
}

/// `$&`, `$1`… and `$$` in a replacement, as CodeMirror reads them.
fn expand(replace: &str, captures: &Captures) -> String {
    let mut out = String::new();
    let mut rest = replace;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(stripped) = after.strip_prefix('&') {
            out.push_str(captures.get(0).map_or("", |m| m.as_str()));
            rest = stripped;
        } else if let Some(stripped) = after.strip_prefix('$') {
            out.push('$');
            rest = stripped;
        } else {
            let digits = after.chars().take_while(char::is_ascii_digit).count();
            // The longest group number that exists, as `$10` is group 1 then `0` with fewer groups.
            let group = (1..=digits).rev().find_map(|len| {
                let number: usize = after[..len].parse().ok()?;
                (number > 0 && number < captures.len()).then_some((len, number))
            });
            match group {
                Some((len, number)) => {
                    out.push_str(captures.get(number).map_or("", |m| m.as_str()));
                    rest = &after[len..];
                }
                None => {
                    out.push('$');
                    rest = after;
                }
            }
        }
    }
    out.push_str(rest);
    out
}

/// `\n`, `\r`, `\t` and `\\` as the characters they stand for.
fn unquote(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let escaped = match (c, chars.peek()) {
            ('\\', Some('n')) => Some('\n'),
            ('\\', Some('r')) => Some('\r'),
            ('\\', Some('t')) => Some('\t'),
            ('\\', Some('\\')) => Some('\\'),
            _ => None,
        };
        match escaped {
            Some(escaped) => {
                chars.next();
                out.push(escaped);
            }
            None => out.push(c),
        }
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether each end of `range` is a word's edge (CodeMirror's "by word").
fn at_word_boundaries(text: &str, range: &Range<usize>) -> bool {
    let edge = |at: usize| {
        let before = text[..at].chars().next_back().is_some_and(is_word);
        let after = text[at..].chars().next().is_some_and(is_word);
        !(before && after)
    };
    edge(range.start) && edge(range.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found<'a>(query: &SearchQuery, text: &'a str) -> Vec<&'a str> {
        query.compile().map(|search| search.matches(text)).unwrap_or_default().into_iter().map(|r| &text[r]).collect()
    }

    fn query(text: &str) -> SearchQuery {
        SearchQuery { text: text.into(), ..SearchQuery::default() }
    }

    #[test]
    fn finds_text_ignoring_case_unless_asked() {
        let text = "SELECT id FROM t; select Id";
        assert_eq!(found(&query("select"), text), ["SELECT", "select"]);
        assert_eq!(found(&SearchQuery { case_sensitive: true, ..query("select") }, text), ["select"]);
        assert_eq!(found(&query("a\\nb"), "a\nb ab"), ["a\nb"], "\\n stands for a line break");
        assert!(query("").compile().is_none(), "nothing to look for");
        assert_eq!(found(&query("(t"), "f(t)"), ["(t"], "plain text isn't an expression");
    }

    #[test]
    fn finds_regular_expressions_and_whole_words() {
        let text = "id, order_id, ids";
        assert_eq!(found(&SearchQuery { regex: true, ..query(r"\bid\w*") }, text), ["id", "ids"]);
        assert!(SearchQuery { regex: true, ..query("(") }.compile().is_none(), "an invalid expression finds nothing");
        assert_eq!(found(&SearchQuery { whole_word: true, ..query("id") }, text), ["id"]);
        assert_eq!(found(&SearchQuery { regex: true, ..query("x*") }, "ab"), Vec::<&str>::new(), "no empty matches");
        assert_eq!(found(&SearchQuery { regex: true, ..query("^a") }, "a\na"), ["a", "a"], "^ starts each line");
    }

    #[test]
    fn replaces_with_groups_and_wraps_round() {
        let search = SearchQuery { regex: true, ..query(r"(\w+)@(\w+)") }.compile().unwrap();
        let text = "to ann@example";
        let range = search.matches(text)[0].clone();
        assert_eq!(search.replacement(text, range.clone(), "$2: $1 ($&) $$5 $9"), "example: ann (ann@example) $5 $9");
        let plain = query("ann").compile().unwrap();
        assert_eq!(plain.replacement(text, 3..6, "$1\\t"), "$1\t", "plain text keeps $ but unquotes \\t");
        let matches = [2..4, 8..10];
        assert_eq!(next(&matches, 5), Some(8..10));
        assert_eq!(next(&matches, 9), Some(2..4), "past the last match: the first");
        assert_eq!(previous(&matches, 8), Some(2..4));
        assert_eq!(previous(&matches, 3), Some(8..10), "before the first: the last");
    }
}
