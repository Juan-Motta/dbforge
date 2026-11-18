//! Colours for a document shown as text, JSON or mongosh's literals
//! (`ObjectId("…")`, `NumberLong(5)`), without an editor per document: a
//! page can hold hundreds of them. The same runs highlight the document
//! editors and the mongosh console, which also greys out `//` comments.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use super::highlight::{Kind, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// A quoted name followed by `:`.
    Key,
    String,
    Number,
    /// `true`, `false`, `null`, `undefined`.
    Literal,
    /// A name followed by `(`: `ObjectId`, `find`.
    Call,
    Plain,
}

/// A run of one kind, by byte range in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

/// Quoted text (a key when a `:` follows), a number, a literal, or a name
/// called. Word boundaries and digits are ASCII, as in JavaScript.
static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')(\s*:)?"#,
        r"|(-?[0-9]+(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)(?-u:\b)",
        r"|(?-u:\b)(true|false|null|undefined)(?-u:\b)",
        r"|(?-u:\b)([A-Za-z_$][A-Za-z0-9_$]*)\(",
    ))
    .expect("a valid pattern")
});

/// The document split into runs of one kind; joined back they are the text.
pub fn document_tokens(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let plain = |tokens: &mut Vec<Token>, range: Range<usize>| {
        if !range.is_empty() {
            tokens.push(Token { range, kind: TokenKind::Plain });
        }
    };
    let mut last = 0;
    for captures in PATTERN.captures_iter(text) {
        let whole = captures.get(0).expect("the whole match");
        plain(&mut tokens, last..whole.start());
        let end = if let Some(quoted) = captures.get(1) {
            match captures.get(2) {
                Some(colon) => {
                    tokens.push(Token { range: quoted.range(), kind: TokenKind::Key });
                    tokens.push(Token { range: colon.range(), kind: TokenKind::Plain });
                }
                None => tokens.push(Token { range: quoted.range(), kind: TokenKind::String }),
            }
            whole.end()
        } else if let Some(number) = captures.get(3) {
            tokens.push(Token { range: number.range(), kind: TokenKind::Number });
            number.end()
        } else if let Some(literal) = captures.get(4) {
            tokens.push(Token { range: literal.range(), kind: TokenKind::Literal });
            literal.end()
        } else if let Some(call) = captures.get(5) {
            // The `(` is not the call's: it is matched only to tell a call from a name.
            tokens.push(Token { range: call.range(), kind: TokenKind::Call });
            call.end()
        } else {
            whole.end()
        };
        last = end;
    }
    plain(&mut tokens, last..text.len());
    tokens
}

impl TokenKind {
    /// How the editor colours it: keys as keywords, literals as numbers, calls as functions.
    pub fn highlight(self) -> Kind {
        match self {
            Self::Key => Kind::Key,
            Self::String => Kind::String,
            Self::Number | Self::Literal => Kind::Number,
            Self::Call => Kind::Function,
            Self::Plain => Kind::Plain,
        }
    }
}

/// A line's highlighted spans, covering it exactly. With `comments`
/// (mongosh), `//` outside a string greys out the rest of the line.
pub fn spans(line: &str, comments: bool) -> Vec<Span> {
    let mut spans = Vec::new();
    for token in document_tokens(line) {
        if comments
            && token.kind == TokenKind::Plain
            && let Some(at) = line[token.range.clone()].find("//")
        {
            let start = token.range.start + at;
            if start > token.range.start {
                spans.push(Span { range: token.range.start..start, kind: Kind::Plain });
            }
            spans.push(Span { range: start..line.len(), kind: Kind::Comment });
            return spans;
        }
        spans.push(Span { range: token.range, kind: token.kind.highlight() });
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(TokenKind, &str)> {
        document_tokens(text)
            .into_iter()
            .filter(|token| token.kind != TokenKind::Plain)
            .map(|token| (token.kind, &text[token.range]))
            .collect()
    }

    #[test]
    fn tells_keys_strings_numbers_literals_and_mongosh_calls_apart() {
        use TokenKind::*;
        let text = "{\n  \"_id\": ObjectId(\"65a0\"),\n  \"n\": NumberLong(5),\n  \"ok\": true,\n  \"note\": \"a: \\\"b\\\"\",\n  \"x\": -1.5e3\n}";
        let tokens = document_tokens(text);
        assert_eq!(tokens.iter().map(|token| &text[token.range.clone()]).collect::<std::string::String>(), text);
        assert_eq!(
            kinds(text),
            [
                (Key, "\"_id\""),
                (Call, "ObjectId"),
                (String, "\"65a0\""),
                (Key, "\"n\""),
                (Call, "NumberLong"),
                (Number, "5"),
                (Key, "\"ok\""),
                (Literal, "true"),
                (Key, "\"note\""),
                (String, "\"a: \\\"b\\\"\""),
                (Key, "\"x\""),
                (Number, "-1.5e3"),
            ]
        );
    }

    #[test]
    fn keeps_numbers_inside_words_and_strings_whole() {
        let text = "{\"k2\": \"v1\", \"ok\": null}";
        assert!(document_tokens(text).iter().all(|token| token.kind != TokenKind::Number));
    }

    #[test]
    fn greys_out_a_mongosh_comment_but_not_slashes_in_a_string() {
        let line = "db.find({url: \"http://x\"}) // all of them";
        let spans = spans(line, true);
        let comment = spans.last().unwrap();
        assert_eq!((comment.kind, &line[comment.range.clone()]), (Kind::Comment, "// all of them"));
        assert!(spans.iter().any(|span| span.kind == Kind::String && &line[span.range.clone()] == "\"http://x\""));
        assert_eq!(spans.iter().map(|span| span.range.len()).sum::<usize>(), line.len());
        assert!(super::spans(line, false).iter().all(|span| span.kind != Kind::Comment), "JSON has no comments");
    }
}
