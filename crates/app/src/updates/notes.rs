//! The release notes as blocks to draw, as the former web UI's `notes.ts`: the
//! Markdown `scripts/release-notes.sh` writes, `## ` headings and `- `
//! items with `**scope:**` in bold; any other line is a line of text.

use std::sync::LazyLock;

use regex::Regex;

/// A piece of a line: plain, or bold where the notes wrote `**…**`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(String),
    Item(Vec<Run>),
    Text(Vec<Run>),
}

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#{1,6}\s+(.*)$").expect("a valid pattern"));
static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[-*]\s+(.*)$").expect("a valid pattern"));
static BOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*[^*]+\*\*").expect("a valid pattern"));

pub fn read_notes(markdown: &str) -> Vec<Block> {
    markdown
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            if let Some(heading) = HEADING.captures(line) {
                Block::Heading(heading[1].to_owned())
            } else if let Some(item) = ITEM.captures(line) {
                Block::Item(runs_of(&item[1]))
            } else {
                Block::Text(runs_of(line))
            }
        })
        .collect()
}

/// `line` cut at its `**bold**` parts, empty pieces left out.
fn runs_of(line: &str) -> Vec<Run> {
    let mut runs = Vec::new();
    let plain = |text: &str, runs: &mut Vec<Run>| {
        if !text.is_empty() {
            runs.push(Run { text: text.to_owned(), bold: false });
        }
    };
    let mut start = 0;
    for bold in BOLD.find_iter(line) {
        plain(&line[start..bold.start()], &mut runs);
        runs.push(Run { text: line[bold.start() + 2..bold.end() - 2].to_owned(), bold: true });
        start = bold.end();
    }
    plain(&line[start..], &mut runs);
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, bold: bool) -> Run {
        Run { text: text.to_owned(), bold }
    }

    #[test]
    fn reads_the_headings_and_items_the_release_notes_script_writes() {
        let notes = "## Features\n\n- **import:** import rows from a file\n- plain item\n\n## Fixes\n\nSee the site.\n";
        assert_eq!(
            read_notes(notes),
            [
                Block::Heading("Features".into()),
                Block::Item(vec![run("import:", true), run(" import rows from a file", false)]),
                Block::Item(vec![run("plain item", false)]),
                Block::Heading("Fixes".into()),
                Block::Text(vec![run("See the site.", false)]),
            ]
        );
    }

    #[test]
    fn reads_no_notes_as_nothing() {
        assert_eq!(read_notes(""), []);
        assert_eq!(read_notes("\n  \n"), []);
    }

    #[test]
    fn keeps_an_unclosed_bold_mark_as_text() {
        assert_eq!(read_notes("* **a** and **b"), [Block::Item(vec![run("a", true), run(" and **b", false)])]);
    }
}
