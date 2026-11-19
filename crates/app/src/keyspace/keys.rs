//! The key list apart from drawing it: keys grouped into folders by their
//! `:` prefixes, the rows those folders show, and how pages of a `SCAN`
//! join. A port of the web UI's `keyspace.ts` (`keyTree`, `visibleKeyRows`).

use std::collections::{BTreeMap, HashSet};

use dbforge_core::drivers::redis::KeyInfo;

/// The types a key can be filtered by, in the web UI's order.
pub const KINDS: [&str; 6] = ["string", "hash", "list", "set", "zset", "stream"];

/// A folder of keys sharing a prefix, or one key.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyNode {
    /// `prefix` ends with `:`; `count` keys are under it, at any depth.
    Folder {
        prefix: String,
        count: usize,
        children: Vec<KeyNode>,
    },
    Key(KeyInfo),
}

impl KeyNode {
    /// The folder's whole prefix, or the key's whole name.
    pub fn label(&self) -> &str {
        match self {
            KeyNode::Folder { prefix, .. } => prefix,
            KeyNode::Key(key) => &key.name,
        }
    }
}

/// Groups keys into folders by their `:`-separated prefixes, as most Redis
/// apps name them (`session:7f3a:roles`). Folders come first; each counts
/// the keys under it. Both are sorted by their bytes, as the web UI did.
pub fn key_tree(keys: &[KeyInfo]) -> Vec<KeyNode> {
    tree_under(keys.iter().collect(), "")
}

fn tree_under(keys: Vec<&KeyInfo>, prefix: &str) -> Vec<KeyNode> {
    let mut folders: BTreeMap<String, Vec<&KeyInfo>> = BTreeMap::new();
    let mut leaves = Vec::new();
    for key in keys {
        let rest = &key.name[prefix.len()..];
        match rest.find(':') {
            None => leaves.push(KeyNode::Key(key.clone())),
            Some(cut) => folders.entry(format!("{prefix}{}", &rest[..=cut])).or_default().push(key),
        }
    }
    leaves.sort_by(|a, b| a.label().cmp(b.label()));
    let folders = folders.into_iter().map(|(folder, inside)| KeyNode::Folder {
        count: inside.len(),
        children: tree_under(inside, &folder),
        prefix: folder,
    });
    folders.chain(leaves).collect()
}

/// One row of the list: a folder or a key, at its depth in the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyRow<'a> {
    pub node: &'a KeyNode,
    pub depth: usize,
}

/// The rows on screen: folders, and the children of expanded ones.
pub fn visible_key_rows<'a>(nodes: &'a [KeyNode], is_expanded: &dyn Fn(&str) -> bool) -> Vec<KeyRow<'a>> {
    let mut rows = Vec::new();
    push_rows(nodes, is_expanded, 0, &mut rows);
    rows
}

fn push_rows<'a>(nodes: &'a [KeyNode], is_expanded: &dyn Fn(&str) -> bool, depth: usize, rows: &mut Vec<KeyRow<'a>>) {
    for node in nodes {
        rows.push(KeyRow { node, depth });
        if let KeyNode::Folder { prefix, children, .. } = node
            && is_expanded(prefix)
        {
            push_rows(children, is_expanded, depth + 1, rows);
        }
    }
}

/// What a row shows of a name nested in the tree: its last segment
/// (`a:b:c` → `c`, folder `a:b:` → `b:`).
pub fn last_segment(name: &str) -> &str {
    let trimmed = name.strip_suffix(':').unwrap_or(name);
    let start = trimmed.rfind(':').map_or(0, |cut| cut + 1);
    &name[start..]
}

/// `page` after the keys listed already: `SCAN` may return a key twice across pages.
pub fn join_page(mut keys: Vec<KeyInfo>, page: Vec<KeyInfo>) -> Vec<KeyInfo> {
    let known: HashSet<String> = keys.iter().map(|key| key.name.clone()).collect();
    keys.extend(page.into_iter().filter(|key| !known.contains(&key.name)));
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> KeyInfo {
        KeyInfo { name: name.into(), binary: false, kind: "string".into(), ttl_ms: None }
    }

    fn summary(nodes: &[KeyNode]) -> Vec<(&'static str, &str, Option<usize>)> {
        nodes
            .iter()
            .map(|node| match node {
                KeyNode::Folder { prefix, count, .. } => ("folder", prefix.as_str(), Some(*count)),
                KeyNode::Key(key) => ("key", key.name.as_str(), None),
            })
            .collect()
    }

    #[test]
    fn groups_keys_by_their_prefixes_folders_first_with_key_counts() {
        let tree = key_tree(&[key("session:b"), key("session:a:roles"), key("session:a"), key("plain"), key("cart:1")]);
        assert_eq!(
            summary(&tree),
            [("folder", "cart:", Some(1)), ("folder", "session:", Some(3)), ("key", "plain", None)]
        );
        let KeyNode::Folder { children, .. } = &tree[1] else { panic!("a folder") };
        let labels: Vec<&str> = children.iter().map(KeyNode::label).collect();
        assert_eq!(labels, ["session:a:", "session:a", "session:b"]);
    }

    #[test]
    fn lists_collapsed_folders_as_one_row_and_expanded_ones_with_their_children() {
        let tree = key_tree(&[key("a:1"), key("a:2"), key("b")]);
        let rows = |expanded: &dyn Fn(&str) -> bool| -> Vec<(usize, String)> {
            visible_key_rows(&tree, expanded).iter().map(|row| (row.depth, row.node.label().to_owned())).collect()
        };
        assert_eq!(rows(&|_| false), [(0, "a:".to_owned()), (0, "b".to_owned())]);
        assert_eq!(
            rows(&|prefix| prefix == "a:"),
            [(0, "a:".to_owned()), (1, "a:1".to_owned()), (1, "a:2".to_owned()), (0, "b".to_owned())]
        );
    }

    #[test]
    fn shows_the_last_segment_of_a_nested_name() {
        assert_eq!(last_segment("a:b:c"), "c");
        assert_eq!(last_segment("a:b:"), "b:");
        assert_eq!(last_segment("plain"), "plain");
    }

    #[test]
    fn joins_scan_pages_without_listing_a_key_twice() {
        let joined = join_page(vec![key("a"), key("b")], vec![key("b"), key("c")]);
        let names: Vec<&str> = joined.iter().map(|key| key.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
    }
}
