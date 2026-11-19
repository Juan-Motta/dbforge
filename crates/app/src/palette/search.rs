//! The palette's search, as the former web UI's `search.ts`: a case-insensitive
//! substring match scored so an exact name beats a prefix, a prefix beats
//! the start of a word, which beats any other hit; a leading `>` limits the
//! search to commands. Pure, so it is tested without a window.

use std::cmp::Ordering;

/// Where the query was found in a label, and how good a hit it is (lower is
/// better). `start` and `length` are byte offsets into the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub length: usize,
    pub score: u8,
}

/// What separates words in a name: `order` starts a word in `sales_order`.
fn separates_words(c: char) -> bool {
    c.is_whitespace() || matches!(c, '_' | '.' | '-' | ':')
}

/// Whether `label` holds `wanted` (a query in lower case) from byte `at`
/// on, ignoring case; the matched part's length in bytes.
fn matches_at(label: &str, at: usize, wanted: &[char]) -> Option<usize> {
    let mut matched = 0;
    for (offset, c) in label[at..].char_indices() {
        if matched == wanted.len() {
            return Some(offset);
        }
        for lower in c.to_lowercase() {
            if wanted.get(matched) != Some(&lower) {
                return None;
            }
            matched += 1;
        }
    }
    (matched == wanted.len()).then(|| label.len() - at)
}

/// `query` in `label`, ignoring case; `None` without a hit. An empty
/// query matches everything.
pub fn match_label(label: &str, query: &str) -> Option<Match> {
    if query.is_empty() {
        return Some(Match { start: 0, length: 0, score: 0 });
    }
    let wanted: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let (start, length) = label.char_indices().find_map(|(at, _)| matches_at(label, at, &wanted).map(|l| (at, l)))?;
    let score = if length == label.len() && start == 0 {
        0
    } else if start == 0 {
        1
    } else if label[..start].chars().next_back().is_some_and(separates_words) {
        2
    } else {
        3
    };
    Some(Match { start, length, score })
}

/// Splits a label around its match, for highlighting.
pub fn split_at_match<'a>(label: &'a str, hit: &Match) -> (&'a str, &'a str, &'a str) {
    let end = hit.start + hit.length;
    (&label[..hit.start], &label[hit.start..end], &label[end..])
}

/// The query as searched: its text, and whether a leading `>` asked for commands only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub commands_only: bool,
}

/// A leading `>` limits the search to commands, as in most editors.
pub fn parse_query(raw: &str) -> Query {
    match raw.trim_start().strip_prefix('>') {
        Some(rest) => Query { text: rest.trim().to_owned(), commands_only: true },
        None => Query { text: raw.trim().to_owned(), commands_only: false },
    }
}

/// Names in alphabetical order, as `localeCompare` sorts them: ignoring
/// case first, then lower case before upper case (`alfa` before `Alfa`).
fn alphabetical(a: &str, b: &str) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| b.cmp(a))
}

/// The items whose label matches, best first, then shortest, then
/// alphabetical; at most `limit` of them.
pub fn rank<T>(items: Vec<T>, label: impl Fn(&T) -> &str, query: &str, limit: usize) -> Vec<(T, Match)> {
    let mut ranked: Vec<(T, Match)> =
        items.into_iter().filter_map(|item| match_label(label(&item), query).map(|hit| (item, hit))).collect();
    ranked.sort_by(|(a, hit_a), (b, hit_b)| {
        let (a, b) = (label(a), label(b));
        hit_a
            .score
            .cmp(&hit_b.score)
            .then_with(|| a.chars().count().cmp(&b.chars().count()))
            .then_with(|| alphabetical(a, b))
    });
    ranked.truncate(limit);
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_case() {
        assert_eq!(match_label("Orders", "ORD"), Some(Match { start: 0, length: 3, score: 1 }));
    }

    #[test]
    fn scores_exact_prefix_word_start_and_inner_hits_in_that_order() {
        assert_eq!(match_label("orders", "orders").map(|hit| hit.score), Some(0));
        assert_eq!(match_label("orders", "ord").map(|hit| hit.score), Some(1));
        assert_eq!(match_label("sales_order", "ord").map(|hit| hit.score), Some(2));
        assert_eq!(match_label("reorder_level", "ord").map(|hit| hit.score), Some(3));
        assert_eq!(match_label("Sales Order", "order").map(|hit| hit.score), Some(2));
        assert_eq!(match_label("ORDERS", "orders").map(|hit| hit.score), Some(0), "exact ignores case too");
    }

    #[test]
    fn returns_none_without_a_hit() {
        assert_eq!(match_label("customers", "ord"), None);
    }

    #[test]
    fn matches_everything_for_an_empty_query() {
        assert_eq!(match_label("customers", ""), Some(Match { start: 0, length: 0, score: 0 }));
    }

    #[test]
    fn splits_around_the_hit() {
        assert_eq!(
            split_at_match("v_orders_daily", &Match { start: 2, length: 3, score: 2 }),
            ("v_", "ord", "ers_daily")
        );
        let hit = match_label("año_ÉTÉ", "été").unwrap();
        assert_eq!(split_at_match("año_ÉTÉ", &hit), ("año_", "ÉTÉ", ""), "offsets stay on character boundaries");
    }

    #[test]
    fn reads_a_leading_greater_than_as_commands_only() {
        assert_eq!(parse_query(" > theme"), Query { text: "theme".into(), commands_only: true });
    }

    #[test]
    fn searches_everything_otherwise() {
        assert_eq!(parse_query(" ord "), Query { text: "ord".into(), commands_only: false });
    }

    fn labels() -> Vec<&'static str> {
        vec!["reorder_level", "order_items", "sales_order", "orders", "customers"]
    }

    #[test]
    fn orders_by_score_then_length_then_name() {
        let ranked: Vec<&str> =
            rank(labels(), |label| label, "order", usize::MAX).into_iter().map(|(l, _)| l).collect();
        assert_eq!(ranked, ["orders", "order_items", "sales_order", "reorder_level"]);
        let tied: Vec<&str> =
            rank(vec!["beta", "Alfa", "alfa"], |l| l, "", usize::MAX).into_iter().map(|(l, _)| l).collect();
        assert_eq!(tied, ["alfa", "Alfa", "beta"]);
    }

    #[test]
    fn keeps_at_most_limit_results() {
        assert_eq!(rank(labels(), |label| label, "", 2).len(), 2);
    }
}
