//! JSON text laid out again without parsing it into numbers, so big integers
//! and exact decimals stay as the database wrote them. Only whitespace
//! outside strings changes.

/// Whether the text is a JSON object or array.
pub fn is_json_document(text: &str) -> bool {
    let trimmed = text.trim();
    (trimmed.starts_with('{') || trimmed.starts_with('[')) && json_error(trimmed).is_none()
}

/// Why the text is not valid JSON, or `None` when it is.
pub fn json_error(text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(text).err().map(|error| error.to_string())
}

/// The tokens of JSON text: strings whole, every other run of non-space
/// characters split at punctuation.
fn tokens(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'"' {
            let mut end = i + 1;
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            let end = (end + 1).min(bytes.len());
            out.push(&text[i..end]);
            i = end;
        } else if b"{}[],:".contains(&byte) {
            out.push(&text[i..i + 1]);
            i += 1;
        } else if byte.is_ascii_whitespace() {
            i += 1;
        } else {
            let mut end = i;
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() && !b"{}[],:\"".contains(&bytes[end]) {
                end += 1;
            }
            // Never splits a character: the bytes stopped at are ASCII.
            out.push(&text[i..end]);
            i = end;
        }
    }
    out
}

/// JSON text on one line, without spaces.
pub fn compact_json(text: &str) -> String {
    tokens(text).concat()
}

/// JSON text indented by two spaces; empty objects and arrays stay on one line.
pub fn pretty_json(text: &str) -> String {
    let list = tokens(text);
    let mut out = String::new();
    let mut depth = 0usize;
    let newline = |depth: usize| format!("\n{}", "  ".repeat(depth));
    for (index, token) in list.iter().enumerate() {
        match *token {
            "{" | "[" => {
                let close = if *token == "{" { "}" } else { "]" };
                out.push_str(token);
                depth += 1;
                if list.get(index + 1) != Some(&close) {
                    out.push_str(&newline(depth));
                }
            }
            "}" | "]" => {
                depth = depth.saturating_sub(1);
                let open = if *token == "}" { "{" } else { "[" };
                if index == 0 || list[index - 1] != open {
                    out.push_str(&newline(depth));
                }
                out.push_str(token);
            }
            "," => {
                out.push(',');
                out.push_str(&newline(depth));
            }
            ":" => out.push_str(": "),
            _ => out.push_str(token),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indents_objects_and_arrays_by_two_spaces() {
        assert_eq!(
            pretty_json(r#"{"a":1,"b":[true,null]}"#),
            "{\n  \"a\": 1,\n  \"b\": [\n    true,\n    null\n  ]\n}"
        );
    }

    #[test]
    fn keeps_big_integers_and_decimals_exactly_as_written() {
        assert_eq!(
            pretty_json(r#"{"id":12345678901234567890,"price":1.50}"#),
            "{\n  \"id\": 12345678901234567890,\n  \"price\": 1.50\n}"
        );
    }

    #[test]
    fn leaves_punctuation_and_escaped_quotes_inside_strings_alone() {
        assert_eq!(pretty_json(r#"{"say":"a, b: {c} \"d\""}"#), "{\n  \"say\": \"a, b: {c} \\\"d\\\"\"\n}");
    }

    #[test]
    fn keeps_empty_objects_and_arrays_on_one_line() {
        assert_eq!(pretty_json(r#"{"a":{},"b":[]}"#), "{\n  \"a\": {},\n  \"b\": []\n}");
    }

    #[test]
    fn compacts_whitespace_outside_strings_only_and_undoes_pretty() {
        assert_eq!(compact_json("{\n  \"a b\": 1,\n  \"c\": [ 2 ]\n}"), r#"{"a b":1,"c":[2]}"#);
        let text = r#"{"id":12345678901234567890,"tags":["x, y"],"o":{}}"#;
        assert_eq!(compact_json(&pretty_json(text)), text);
    }

    #[test]
    fn accepts_objects_and_arrays_only() {
        assert!(is_json_document(r#" {"a": 1} "#));
        assert!(is_json_document("[1]"));
        assert!(!is_json_document("42"));
        assert!(!is_json_document("\"text\""));
        assert!(!is_json_document("{oops}"));
    }

    #[test]
    fn says_why_text_is_not_json() {
        assert_eq!(json_error(r#"{"a":1}"#), None);
        assert!(json_error(r#"{"a":}"#).is_some());
    }
}
