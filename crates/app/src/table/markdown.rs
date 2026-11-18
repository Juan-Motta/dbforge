//! A cell's text read as Markdown, into the blocks the inspector draws:
//! headings, paragraphs, list items, quotes, code blocks and rules. Inline
//! marks (emphasis, code, links) are dropped for their text; links and
//! images never load anything, an image shows as `[alt]`.

use std::sync::OnceLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// A heading, 1 to 6.
    Heading(u8, String),
    Paragraph(String),
    /// A list item with its marker as shown (`•`, `1.`) and its nesting.
    Item {
        marker: String,
        depth: usize,
        text: String,
    },
    Quote(String),
    Code(String),
    Rule,
}

/// Inline marks taken off, links and images as their text.
pub fn inline(text: &str, image: &str) -> String {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        [
            (r"!\[([^\]]*)\]\([^)]*\)", "[$1]"),
            (r"\[([^\]]*)\]\([^)]*\)", "$1"),
            (r"`([^`]*)`", "$1"),
            (r"\*\*([^*]+)\*\*", "$1"),
            (r"__([^_]+)__", "$1"),
            (r"\*([^*\s][^*]*)\*", "$1"),
            (r"\b_([^_]+)_\b", "$1"),
            (r"~~([^~]+)~~", "$1"),
        ]
        .into_iter()
        .map(|(pattern, with)| (Regex::new(pattern).expect("a valid pattern"), with))
        .collect()
    });
    let text =
        rules.iter().fold(text.to_owned(), |text, (pattern, with)| pattern.replace_all(&text, *with).into_owned());
    // An image without alt text is named by what it is.
    text.replace("[]", &format!("[{image}]"))
}

/// The blocks of `text`; `image` names an image that has no alt text.
pub fn blocks(text: &str, image: &str) -> Vec<Block> {
    static ITEM: OnceLock<Regex> = OnceLock::new();
    let item = ITEM.get_or_init(|| Regex::new(r"^(\s*)([-*+]|\d+[.)])\s+(.*)$").expect("a valid pattern"));
    let mut out = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut code: Option<Vec<&str>> = None;
    let flush = |paragraph: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            out.push(Block::Paragraph(inline(&paragraph.join(" "), image)));
            paragraph.clear();
        }
    };
    for line in text.lines() {
        if let Some(lines) = code.as_mut() {
            if line.trim_start().starts_with("```") {
                out.push(Block::Code(lines.join("\n")));
                code = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            flush(&mut paragraph, &mut out);
            code = Some(Vec::new());
        } else if trimmed.is_empty() {
            flush(&mut paragraph, &mut out);
        } else if let Some(level) = heading_level(trimmed) {
            flush(&mut paragraph, &mut out);
            out.push(Block::Heading(level, inline(trimmed[level as usize..].trim(), image)));
        } else if ["---", "***", "___"].contains(&trimmed) {
            flush(&mut paragraph, &mut out);
            out.push(Block::Rule);
        } else if let Some(quote) = trimmed.strip_prefix('>') {
            flush(&mut paragraph, &mut out);
            out.push(Block::Quote(inline(quote.trim(), image)));
        } else if let Some(captures) = item.captures(line) {
            flush(&mut paragraph, &mut out);
            let marker = &captures[2];
            let marker = if marker.ends_with(['.', ')']) { marker.to_owned() } else { "•".to_owned() };
            out.push(Block::Item { marker, depth: captures[1].len() / 2, text: inline(&captures[3], image) });
        } else {
            paragraph.push(trimmed);
        }
    }
    if let Some(lines) = code {
        out.push(Block::Code(lines.join("\n")));
    }
    flush(&mut paragraph, &mut out);
    out
}

fn heading_level(line: &str) -> Option<u8> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    ((1..=6).contains(&hashes) && line[hashes..].starts_with(' ')).then_some(hashes as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_headings_paragraphs_lists_quotes_code_and_rules() {
        let text = "# Title\n\nSome **bold** and `code`\ncontinued.\n\n- one\n  - two\n1. first\n> said\n---\n```\nlet x = 1;\n```";
        assert_eq!(
            blocks(text, "image"),
            [
                Block::Heading(1, "Title".into()),
                Block::Paragraph("Some bold and code continued.".into()),
                Block::Item { marker: "•".into(), depth: 0, text: "one".into() },
                Block::Item { marker: "•".into(), depth: 1, text: "two".into() },
                Block::Item { marker: "1.".into(), depth: 0, text: "first".into() },
                Block::Quote("said".into()),
                Block::Rule,
                Block::Code("let x = 1;".into()),
            ]
        );
    }

    #[test]
    fn shows_links_as_their_text_and_images_as_their_alt_never_loading_them() {
        assert_eq!(inline("[docs](https://x.y) ![logo](a.png) ![](b.png)", "image"), "docs [logo] [image]");
        assert_eq!(blocks("#nospace", "image"), [Block::Paragraph("#nospace".into())]);
    }
}
