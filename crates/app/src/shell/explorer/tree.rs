//! The explorer tree, apart from drawing it: connections → databases →
//! schemas → the Tables and Views folders → tables and views. What the core
//! listed under each node is kept as [`ChildState`]; the tree is rebuilt from
//! it, so folder names follow the language and a reload never empties a level
//! before its new list arrives.

use std::collections::HashMap;

use dbforge_core::connections::model::{Connection, Engine};
use dbforge_core::drivers::{DbObject, ObjectKind};

use crate::i18n::Messages;

/// Joins path segments into ids; object names cannot contain this control character.
const SEP: char = '\u{1f}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Connection,
    Database,
    Schema,
    Folder,
    Table,
    View,
}

impl From<ObjectKind> for NodeKind {
    fn from(kind: ObjectKind) -> Self {
        match kind {
            ObjectKind::Database => Self::Database,
            ObjectKind::Schema => Self::Schema,
            ObjectKind::Table => Self::Table,
            ObjectKind::View => Self::View,
        }
    }
}

/// One node of the tree, without its children: what a row shows and acts on.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub kind: NodeKind,
    /// Short text on the right: a row estimate, a folder's count.
    pub meta: Option<String>,
    pub connection_id: String,
    pub engine: Engine,
    /// What the core is asked to list this node's children.
    pub path: Vec<String>,
    pub expandable: bool,
    /// Its children are being listed; any shown meanwhile are the last list.
    pub loading: bool,
    pub error: Option<String>,
}

impl Node {
    /// Whether its children come from the core (folders arrive with theirs).
    pub fn loads_children(&self) -> bool {
        self.expandable && self.kind != NodeKind::Folder
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeNode {
    pub node: Node,
    /// `None` until listed.
    pub children: Option<Vec<TreeNode>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Muted,
    Error,
}

/// One line of the tree as shown.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeRow {
    Node {
        node: Node,
        depth: usize,
        expanded: bool,
    },
    /// An error or "No objects" under an expanded node.
    Message {
        id: String,
        depth: usize,
        text: String,
        tone: Tone,
    },
}

impl TreeRow {
    pub fn depth(&self) -> usize {
        match self {
            Self::Node { depth, .. } | Self::Message { depth, .. } => *depth,
        }
    }

    pub fn node(&self) -> Option<&Node> {
        match self {
            Self::Node { node, .. } => Some(node),
            Self::Message { .. } => None,
        }
    }
}

/// A connection's node; its id is the connection's own, so a refresh can name it.
pub fn connection_node(connection: &Connection) -> TreeNode {
    TreeNode {
        node: Node {
            id: connection.id.clone(),
            name: connection.name.clone(),
            kind: NodeKind::Connection,
            meta: None,
            connection_id: connection.id.clone(),
            engine: connection.engine,
            path: Vec::new(),
            expandable: true,
            loading: false,
            error: None,
        },
        children: None,
    }
}

/// A row estimate as the former web UI wrote it: `612`, `48.2k`, `1.5m`.
pub fn compact(count: i64) -> String {
    const UNITS: [(f64, &str); 4] = [(1e12, "t"), (1e9, "b"), (1e6, "m"), (1e3, "k")];
    let value = count as f64;
    let magnitude = value.abs();
    // From the largest unit down; a value that rounds up to 1000 of a unit moves to the next one.
    for (index, (size, unit)) in UNITS.iter().enumerate() {
        if magnitude >= *size {
            let scaled = (magnitude / size * 10.).round() / 10.;
            if scaled >= 1000. && index > 0 {
                let (bigger, bigger_unit) = UNITS[index - 1];
                return format!("{}{}", signed(value, magnitude / bigger), bigger_unit);
            }
            return format!("{}{unit}", signed(value, scaled));
        }
    }
    count.to_string()
}

/// `scaled` with `value`'s sign and at most one decimal, without a trailing `.0`.
fn signed(value: f64, scaled: f64) -> String {
    let rounded = (scaled * 10.).round() / 10.;
    let text = if rounded.fract() == 0. { format!("{rounded:.0}") } else { format!("{rounded:.1}") };
    if value < 0. { format!("-{text}") } else { text }
}

/// The objects the core listed under `parent` as nodes. Tables and views are
/// grouped into "Tables" and "Views" folders, named in `m`'s language, which
/// arrive already listed.
pub fn to_child_nodes(parent: &Node, objects: &[DbObject], m: &Messages) -> Vec<TreeNode> {
    // A Redis database holds keys, not schemas: it opens in the key browser.
    let keyspaces = parent.kind == NodeKind::Connection && parent.engine == Engine::Redis;
    let to_node = |object: &DbObject| {
        let mut path = parent.path.clone();
        path.push(object.name.clone());
        TreeNode {
            node: Node {
                id: format!("{}{SEP}{}", parent.id, object.name),
                name: object.name.clone(),
                kind: object.kind.into(),
                meta: object.row_estimate.map(compact),
                connection_id: parent.connection_id.clone(),
                engine: parent.engine,
                path,
                expandable: (object.kind == ObjectKind::Database && !keyspaces) || object.kind == ObjectKind::Schema,
                loading: false,
                error: None,
            },
            children: None,
        }
    };
    // `key` names the folder in its id, the same in every language, so it stays expanded.
    let folder = |key: &str, name: &str, kind: ObjectKind| -> Option<TreeNode> {
        let items: Vec<TreeNode> = objects.iter().filter(|object| object.kind == kind).map(to_node).collect();
        (!items.is_empty()).then(|| TreeNode {
            node: Node {
                id: format!("{}{SEP}#{key}", parent.id),
                name: name.to_owned(),
                kind: NodeKind::Folder,
                meta: Some(items.len().to_string()),
                connection_id: parent.connection_id.clone(),
                engine: parent.engine,
                path: parent.path.clone(),
                expandable: true,
                loading: false,
                error: None,
            },
            children: Some(items),
        })
    };
    let others = objects.iter().filter(|object| !matches!(object.kind, ObjectKind::Table | ObjectKind::View));
    others
        .map(to_node)
        .chain(folder("Tables", m.tables, ObjectKind::Table))
        .chain(folder("Views", m.views, ObjectKind::View))
        .collect()
}

/// The tree as shown, flattened into rows, from depth `depth`.
pub fn visible_rows(nodes: &[TreeNode], is_expanded: &dyn Fn(&str) -> bool, m: &Messages) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    push_rows(nodes, is_expanded, m, 0, &mut rows);
    rows
}

fn push_rows(
    nodes: &[TreeNode],
    is_expanded: &dyn Fn(&str) -> bool,
    m: &Messages,
    depth: usize,
    rows: &mut Vec<TreeRow>,
) {
    for tree in nodes {
        let node = &tree.node;
        let has_content = tree.children.is_some() || node.loading || node.error.is_some();
        let expanded = node.expandable && is_expanded(&node.id) && has_content;
        rows.push(TreeRow::Node { node: node.clone(), depth, expanded });
        if !expanded {
            continue;
        }
        let message = |text: &str, tone| TreeRow::Message {
            id: format!("{}{SEP}#message", node.id),
            depth: depth + 1,
            text: text.to_owned(),
            tone,
        };
        match (&node.error, &tree.children) {
            (Some(error), _) => rows.push(message(error, Tone::Error)),
            // Still being listed: the row's spinner says so; nothing to show under it yet.
            (None, None) => {}
            (None, Some(children)) if children.is_empty() => rows.push(message(m.no_objects, Tone::Muted)),
            (None, Some(children)) => push_rows(children, is_expanded, m, depth + 1, rows),
        }
    }
}

/// Keeps the nodes whose name contains `query` (ignoring case), with the
/// ancestors that lead to them; a matching node keeps all its children. Only
/// listed nodes are searched.
pub fn filter_tree(nodes: Vec<TreeNode>, query: &str) -> Vec<TreeNode> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return nodes;
    }
    filter_by(nodes, &needle)
}

fn filter_by(nodes: Vec<TreeNode>, needle: &str) -> Vec<TreeNode> {
    nodes
        .into_iter()
        .filter_map(|tree| {
            if tree.node.name.to_lowercase().contains(needle) {
                return Some(tree);
            }
            let children = filter_by(tree.children.unwrap_or_default(), needle);
            (!children.is_empty()).then_some(TreeNode { node: tree.node, children: Some(children) })
        })
        .collect()
}

/// What is known of a node's children. The listed objects are kept, not
/// their nodes, so folder names follow the language.
#[derive(Debug, Clone, PartialEq)]
pub enum ChildState {
    /// Being listed; `objects` is the last list, shown meanwhile.
    Loading(Option<Vec<DbObject>>),
    Loaded(Vec<DbObject>),
    Error(String),
}

/// A node's children as a (re)load starts: what is listed stays on screen
/// until the new list replaces it, so the tree never empties and refills.
pub fn start_loading(current: Option<&ChildState>) -> ChildState {
    match current {
        Some(ChildState::Loaded(objects)) => ChildState::Loading(Some(objects.clone())),
        _ => ChildState::Loading(None),
    }
}

/// The tree of `connections` with what has been listed under each node.
pub fn build_tree(connections: &[Connection], children: &HashMap<String, ChildState>, m: &Messages) -> Vec<TreeNode> {
    connections.iter().map(|connection| attach(connection_node(connection), children, m)).collect()
}

fn attach(mut tree: TreeNode, children: &HashMap<String, ChildState>, m: &Messages) -> TreeNode {
    if let Some(listed) = tree.children.take() {
        tree.children = Some(listed.into_iter().map(|child| attach(child, children, m)).collect());
        return tree;
    }
    let objects = match children.get(&tree.node.id) {
        None => return tree,
        Some(ChildState::Error(message)) => {
            tree.node.error = Some(message.clone());
            return tree;
        }
        Some(ChildState::Loading(objects)) => {
            tree.node.loading = true;
            objects.as_deref()
        }
        Some(ChildState::Loaded(objects)) => Some(objects.as_slice()),
    };
    tree.children = objects.map(|objects| {
        to_child_nodes(&tree.node, objects, m).into_iter().map(|child| attach(child, children, m)).collect()
    });
    tree
}

/// The nodes to list again on a refresh. Without `scope`, every expanded node
/// of every connection. With it, the node that lists the scope's children (a
/// folder's parent) and its expanded descendants: refreshing one connection
/// leaves the others alone.
pub fn nodes_to_reload(roots: &[TreeNode], is_expanded: &dyn Fn(&str) -> bool, scope: Option<&str>) -> Vec<Node> {
    let mut reload = Vec::new();
    let Some(scope) = scope else {
        expanded_below(roots, is_expanded, &mut reload);
        return reload;
    };
    let Some(start) = lister(roots, scope, None) else {
        return reload;
    };
    let listed = is_expanded(&start.node.id) || start.children.is_some() || start.node.error.is_some();
    if listed {
        reload.push(start.node.clone());
    }
    expanded_below(start.children.as_deref().unwrap_or_default(), is_expanded, &mut reload);
    reload
}

fn expanded_below(nodes: &[TreeNode], is_expanded: &dyn Fn(&str) -> bool, reload: &mut Vec<Node>) {
    for tree in nodes.iter().filter(|tree| is_expanded(&tree.node.id)) {
        if tree.node.loads_children() {
            reload.push(tree.node.clone());
        }
        expanded_below(tree.children.as_deref().unwrap_or_default(), is_expanded, reload);
    }
}

/// The deepest node on the way to `id` whose children come from the core.
fn lister<'a>(nodes: &'a [TreeNode], id: &str, nearest: Option<&'a TreeNode>) -> Option<&'a TreeNode> {
    for tree in nodes {
        let candidate = if tree.node.loads_children() { Some(tree) } else { nearest };
        if tree.node.id == id {
            return candidate;
        }
        if let Some(found) = lister(tree.children.as_deref().unwrap_or_default(), id, candidate) {
            return Some(found);
        }
    }
    None
}

/// The id of the connection node `id` is under (its own, for a connection).
pub fn connection_of(id: &str) -> &str {
    id.split(SEP).next().unwrap_or(id)
}

/// The index of the row holding node `id`.
pub fn row_of(rows: &[TreeRow], id: &str) -> Option<usize> {
    rows.iter().position(|row| row.node().is_some_and(|node| node.id == id))
}

/// The row of the node above `index` one level up: its parent.
pub fn parent_row(rows: &[TreeRow], index: usize) -> Option<usize> {
    let depth = rows.get(index)?.depth();
    rows[..index].iter().rposition(|row| row.depth() + 1 == depth && row.node().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    fn m() -> &'static Messages {
        Language::En.messages()
    }

    fn connection(id: &str, name: &str, engine: Engine) -> Connection {
        let json = serde_json::json!({
            "id": id, "name": name, "engine": engine, "environment": null, "host": "localhost",
            "port": 5432, "database": "shop", "username": "app",
        });
        serde_json::from_value(json).expect("a saved connection")
    }

    fn pg() -> Connection {
        connection("pg", "local", Engine::Postgres)
    }

    fn object(name: &str, kind: ObjectKind, row_estimate: Option<i64>) -> DbObject {
        DbObject { name: name.into(), kind, row_estimate }
    }

    fn first(nodes: Vec<TreeNode>) -> TreeNode {
        nodes.into_iter().next().expect("a node")
    }

    /// connection → shop → public → Tables(customers, orders) + Views(active)
    fn sample_tree() -> Vec<TreeNode> {
        let mut root = connection_node(&pg());
        let mut shop = first(to_child_nodes(&root.node, &[object("shop", ObjectKind::Database, None)], m()));
        let mut public = first(to_child_nodes(&shop.node, &[object("public", ObjectKind::Schema, None)], m()));
        public.children = Some(to_child_nodes(
            &public.node,
            &[
                object("customers", ObjectKind::Table, Some(48_210)),
                object("orders", ObjectKind::Table, Some(612)),
                object("active", ObjectKind::View, None),
            ],
            m(),
        ));
        shop.children = Some(vec![public]);
        root.children = Some(vec![shop]);
        vec![root]
    }

    fn labels(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                TreeRow::Node { node, depth, .. } => format!("{depth}:{}", node.name),
                TreeRow::Message { depth, text, .. } => format!("{depth}:({text})"),
            })
            .collect()
    }

    fn every(_: &str) -> bool {
        true
    }

    fn none(_: &str) -> bool {
        false
    }

    #[test]
    fn builds_paths_and_ids_from_the_parent() {
        let shop =
            first(to_child_nodes(&connection_node(&pg()).node, &[object("shop", ObjectKind::Database, None)], m()));
        assert_eq!(
            (shop.node.connection_id.as_str(), shop.node.path.clone(), shop.node.expandable),
            ("pg", vec!["shop".to_string()], true)
        );
        assert_ne!(shop.node.id, "pg");
    }

    #[test]
    fn groups_tables_and_views_into_loaded_folders_with_counts_and_compact_row_estimates() {
        let tree = sample_tree();
        let public = &tree[0].children.as_ref().unwrap()[0].children.as_ref().unwrap()[0];
        let folders = public.children.as_ref().unwrap();
        let (tables, views) = (&folders[0], &folders[1]);
        assert_eq!(
            (tables.node.kind, tables.node.name.as_str(), tables.node.meta.as_deref(), tables.node.path.clone()),
            (NodeKind::Folder, "Tables", Some("2"), vec!["shop".to_string(), "public".to_string()])
        );
        let items: Vec<_> = tables
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|t| (t.node.name.as_str(), t.node.meta.as_deref(), t.node.expandable))
            .collect();
        assert_eq!(items, [("customers", Some("48.2k"), false), ("orders", Some("612"), false)]);
        assert_eq!((views.node.name.as_str(), views.node.meta.as_deref()), ("Views", Some("1")));
    }

    #[test]
    fn omits_empty_folders() {
        let nodes = to_child_nodes(&connection_node(&pg()).node, &[object("t", ObjectKind::Table, None)], m());
        assert_eq!(nodes.iter().map(|n| n.node.name.as_str()).collect::<Vec<_>>(), ["Tables"]);
    }

    #[test]
    fn names_the_folders_in_the_language_it_is_given_under_the_same_ids() {
        let objects = [object("t", ObjectKind::Table, None), object("v", ObjectKind::View, None)];
        let root = connection_node(&pg());
        let english = to_child_nodes(&root.node, &objects, m());
        let spanish = to_child_nodes(&root.node, &objects, Language::Es.messages());
        assert_eq!(spanish.iter().map(|n| n.node.name.as_str()).collect::<Vec<_>>(), ["Tablas", "Vistas"]);
        let ids = |nodes: &[TreeNode]| nodes.iter().map(|n| n.node.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&spanish), ids(&english));
    }

    #[test]
    fn redis_databases_open_in_the_key_browser_instead_of_expanding() {
        let redis = connection_node(&connection("cache", "cache", Engine::Redis));
        let db0 = first(to_child_nodes(&redis.node, &[object("db0", ObjectKind::Database, Some(12))], m()));
        assert_eq!(
            (db0.node.expandable, db0.node.meta.as_deref(), db0.node.path.clone()),
            (false, Some("12"), vec!["db0".to_string()])
        );
        let shop =
            first(to_child_nodes(&connection_node(&pg()).node, &[object("shop", ObjectKind::Database, None)], m()));
        assert!(shop.node.expandable);
    }

    #[test]
    fn shows_only_roots_when_nothing_is_expanded() {
        assert_eq!(labels(&visible_rows(&sample_tree(), &none, m())), ["0:local"]);
    }

    #[test]
    fn shows_children_of_expanded_loaded_nodes() {
        assert_eq!(
            labels(&visible_rows(&sample_tree(), &every, m())),
            ["0:local", "1:shop", "2:public", "3:Tables", "4:customers", "4:orders", "3:Views", "4:active"]
        );
    }

    #[test]
    fn shows_error_and_empty_states_under_an_expanded_node_and_loading_is_the_rows_own_spinner() {
        let root = connection_node(&pg());
        let mut loading = root.clone();
        loading.node.loading = true;
        assert_eq!(labels(&visible_rows(&[loading.clone()], &every, m())), ["0:local"]);
        loading.children = Some(to_child_nodes(&root.node, &[object("shop", ObjectKind::Database, None)], m()));
        assert_eq!(labels(&visible_rows(&[loading], &every, m())), ["0:local", "1:shop"]);
        let mut failed = root.clone();
        failed.node.error = Some("refused".into());
        assert_eq!(labels(&visible_rows(&[failed], &every, m())), ["0:local", "1:(refused)"]);
        let mut empty = root;
        empty.children = Some(Vec::new());
        assert_eq!(labels(&visible_rows(&[empty], &every, m())), ["0:local", "1:(No objects)"]);
    }

    #[test]
    fn treats_an_expanded_node_with_nothing_loaded_as_collapsed() {
        let rows = visible_rows(&[connection_node(&pg())], &every, m());
        assert!(matches!(rows[0], TreeRow::Node { expanded: false, .. }));
    }

    #[test]
    fn filter_returns_the_same_tree_for_an_empty_query() {
        assert_eq!(filter_tree(sample_tree(), "  "), sample_tree());
    }

    #[test]
    fn filter_keeps_matches_and_the_ancestors_that_lead_to_them() {
        let rows = visible_rows(&filter_tree(sample_tree(), "CUST"), &every, m());
        assert_eq!(labels(&rows), ["0:local", "1:shop", "2:public", "3:Tables", "4:customers"]);
    }

    #[test]
    fn filter_keeps_all_children_of_a_matching_node() {
        let rows = visible_rows(&filter_tree(sample_tree(), "views"), &every, m());
        assert_eq!(labels(&rows), ["0:local", "1:shop", "2:public", "3:Views", "4:active"]);
    }

    #[test]
    fn filter_returns_nothing_when_no_node_matches() {
        assert_eq!(filter_tree(sample_tree(), "zzz"), Vec::new());
    }

    /// Two connections, every listed node expanded.
    fn two_connections() -> Vec<TreeNode> {
        let mut roots = sample_tree();
        let mut other = connection_node(&connection("other", "remote", Engine::Postgres));
        let mut other_shop = first(to_child_nodes(&other.node, &[object("shop", ObjectKind::Database, None)], m()));
        other_shop.children = Some(Vec::new());
        other.children = Some(vec![other_shop]);
        roots.push(other);
        roots
    }

    fn paths(nodes: &[Node]) -> Vec<String> {
        nodes.iter().map(|n| format!("{}:{}", n.connection_id, n.path.join("/"))).collect()
    }

    #[test]
    fn reloads_every_expanded_node_of_every_connection_without_a_scope() {
        assert_eq!(
            paths(&nodes_to_reload(&two_connections(), &every, None)),
            ["pg:", "pg:shop", "pg:shop/public", "other:", "other:shop"]
        );
    }

    #[test]
    fn reloads_only_the_chosen_connection_and_its_expanded_nodes() {
        assert_eq!(
            paths(&nodes_to_reload(&two_connections(), &every, Some("pg"))),
            ["pg:", "pg:shop", "pg:shop/public"]
        );
    }

    #[test]
    fn reloads_a_folder_through_the_node_that_lists_it() {
        let roots = two_connections();
        let public = &roots[0].children.as_ref().unwrap()[0].children.as_ref().unwrap()[0];
        let tables = &public.children.as_ref().unwrap()[0];
        assert_eq!(paths(&nodes_to_reload(&roots, &every, Some(&tables.node.id))), ["pg:shop/public"]);
    }

    #[test]
    fn skips_collapsed_descendants_but_still_reloads_the_chosen_loaded_node() {
        let roots = two_connections();
        let shop = roots[0].children.as_ref().unwrap()[0].node.id.clone();
        assert_eq!(paths(&nodes_to_reload(&roots, &|id| id == "pg", Some(&shop))), ["pg:shop"]);
    }

    #[test]
    fn reloads_nothing_for_a_node_that_is_gone() {
        assert!(nodes_to_reload(&two_connections(), &every, Some("missing")).is_empty());
    }

    #[test]
    fn starts_with_nothing_listed_when_nothing_was_or_the_last_try_failed() {
        assert_eq!(start_loading(None), ChildState::Loading(None));
        assert_eq!(start_loading(Some(&ChildState::Error("refused".into()))), ChildState::Loading(None));
    }

    #[test]
    fn keeps_what_is_listed_while_it_reloads() {
        let objects = vec![object("shop", ObjectKind::Database, None)];
        assert_eq!(start_loading(Some(&ChildState::Loaded(objects.clone()))), ChildState::Loading(Some(objects)));
    }

    #[test]
    fn builds_the_tree_from_what_each_node_listed_keeping_it_while_it_reloads() {
        let mut children = HashMap::new();
        children.insert("pg".to_string(), ChildState::Loading(Some(vec![object("shop", ObjectKind::Database, None)])));
        let tree = build_tree(&[pg(), connection("down", "down", Engine::Mysql)], &children, m());
        assert!(tree[0].node.loading);
        assert_eq!(labels(&visible_rows(&tree, &every, m())), ["0:local", "1:shop", "0:down"]);
        children.insert("down".to_string(), ChildState::Error("refused".into()));
        let tree = build_tree(&[pg(), connection("down", "down", Engine::Mysql)], &children, m());
        assert_eq!(labels(&visible_rows(&tree, &every, m())), ["0:local", "1:shop", "0:down", "1:(refused)"]);
    }

    #[test]
    fn writes_row_estimates_compactly() {
        let cases = [
            (0, "0"),
            (612, "612"),
            (999, "999"),
            (1_000, "1k"),
            (48_210, "48.2k"),
            (999_950, "1m"),
            (1_500_000, "1.5m"),
            (2_000_000_000, "2b"),
            (-1_500, "-1.5k"),
        ];
        for (count, written) in cases {
            assert_eq!(compact(count), written, "{count}");
        }
    }

    #[test]
    fn finds_a_rows_parent() {
        let rows = visible_rows(&sample_tree(), &every, m());
        let orders = row_of(&rows, &rows[5].node().unwrap().id).unwrap();
        assert_eq!(parent_row(&rows, orders), Some(3), "orders → Tables");
        assert_eq!(parent_row(&rows, 0), None);
    }

    #[test]
    fn every_node_id_starts_with_its_connection_s() {
        let id = format!("pg{SEP}shop{SEP}#Tables");
        assert_eq!(connection_of(&id), "pg");
        assert_eq!(connection_of("pg"), "pg");
    }
}
