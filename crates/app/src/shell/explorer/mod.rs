//! The explorer pane: the saved connections as a tree of their databases,
//! schemas, tables and views. Each level is listed from the core the first
//! time it opens; a refresh lists it again while the rows stay on screen.

mod menu;
pub mod tree;
mod view;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use dbforge_core::connections::model::{Connection, Engine};
use dbforge_core::connections::tags::Tag;
use dbforge_core::explorer;
use gpui::{
    App, AppContext, BorrowAppContext, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    KeyBinding, Pixels, Point, ScrollStrategy, Subscription, UniformListScrollHandle, Window, actions,
};

use self::menu::{NodeCommand, commands, diagram_target};
use self::tree::{ChildState, Node, NodeKind, TreeNode, TreeRow};
use crate::backend::Backend;
use crate::connections::url::connection_url;
use crate::i18n::Translate;
use crate::preferences::{Preferences, keys};
use crate::structure::sections::Section;
use crate::table::TableTabView;
use crate::ui::{MenuItem, PopoverMenu, TextInput, TextInputEvent};

actions!(explorer, [SelectPrevious, SelectNext, Expand, Collapse, Activate]);

/// A table's or view's primary action (double-click or Enter), or a Redis
/// database's: open it. Dispatched for the workspace to handle.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = explorer, no_json)]
pub struct OpenObject {
    pub connection_id: String,
    pub kind: NodeKind,
    pub path: Vec<String>,
    /// What a table's tab shows: its rows when `None`.
    pub view: Option<TableTabView>,
}

/// What the explorer tells the window.
#[derive(Debug, Clone, PartialEq)]
pub enum ExplorerEvent {
    /// A node was selected: the connection (and where in it) now in use.
    Selected { connection_id: String, path: Vec<String> },
    /// The connection list changed (a duplicate was added): the selected connection may have.
    ConnectionsChanged,
    /// The "+" button: open the dialog for a new connection.
    NewConnection,
    /// Open the dialog on this connection.
    EditConnection(Connection),
    /// Ask before deleting this connection.
    DeleteConnection(Connection),
    /// Open a query console on this connection and database, starting with `sql`.
    NewQuery { connection_id: String, database: Option<String>, sql: Option<String> },
    /// Open the ER diagram of this schema.
    OpenDiagram { connection_id: String, database: String, schema: String },
}

const CONTEXT: &str = "Explorer";

/// The explorer's width when none was kept, and the range it may be dragged within.
const WIDTH: f32 = 280.;
const MIN_WIDTH: f32 = 200.;
const MAX_WIDTH: f32 = 560.;

/// ↑ ↓ move the selection, → ← open and close, Enter runs the node's primary action.
pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    vec![
        KeyBinding::new("up", SelectPrevious, context),
        KeyBinding::new("down", SelectNext, context),
        KeyBinding::new("right", Expand, context),
        KeyBinding::new("left", Collapse, context),
        KeyBinding::new("enter", Activate, context),
    ]
}

/// The width kept under [`keys::EXPLORER_WIDTH`], within its range; the default when none was.
pub fn width(saved: Option<&str>) -> f32 {
    match saved.and_then(|saved| saved.parse::<f32>().ok()) {
        Some(width) if width > 0. => width.clamp(MIN_WIDTH, MAX_WIDTH),
        _ => WIDTH,
    }
}

/// An open right-click menu: the menu, where, and the subscription that closes it.
struct OpenMenu {
    menu: Entity<PopoverMenu>,
    position: Point<Pixels>,
    _closing: Subscription,
}

pub struct Explorer {
    connections: Vec<Connection>,
    tags: Vec<Tag>,
    expanded: HashSet<String>,
    /// What each node listed, by node id.
    children: HashMap<String, ChildState>,
    /// The newest listing started for each node, so an older one finishing late is dropped.
    loads: HashMap<String, u64>,
    next_load: u64,
    /// The tree and the rows shown, rebuilt after each change.
    tree: Vec<TreeNode>,
    rows: Rc<Vec<TreeRow>>,
    selected: Option<String>,
    filter: Entity<TextInput>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    menu: Option<OpenMenu>,
    /// Development only: nodes whose children open by themselves once listed.
    auto_expand: HashSet<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ExplorerEvent> for Explorer {}

impl Focusable for Explorer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Explorer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let m = cx.messages();
        let filter = cx.new(|cx| TextInput::new(m.filter_placeholder, cx));
        let subscriptions = vec![
            cx.subscribe_in(&filter, window, Self::on_filter),
            // The language may have changed: folder names and messages follow it.
            cx.observe_global::<Preferences>(|this, cx| {
                let placeholder = cx.messages().filter_placeholder;
                this.filter.update(cx, |filter, cx| filter.set_placeholder(placeholder, cx));
                this.rebuild(cx);
            }),
        ];
        // The stores keep their lists in memory: read at once, so the first frame already shows them.
        let core = Backend::global(cx).core();
        let (connections, tags) = (core.connections().list(), core.tags().list());
        let mut explorer = Self {
            connections,
            tags,
            expanded: HashSet::new(),
            children: HashMap::new(),
            loads: HashMap::new(),
            next_load: 0,
            tree: Vec::new(),
            rows: Rc::default(),
            selected: None,
            filter,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            menu: None,
            auto_expand: HashSet::new(),
            _subscriptions: subscriptions,
        };
        explorer.rebuild(cx);
        #[cfg(debug_assertions)]
        explorer.expand_for_development(cx);
        explorer
    }

    /// The connection `id` and its tag, if it has one that still exists.
    pub fn connection(&self, id: &str) -> Option<(&Connection, Option<&Tag>)> {
        let connection = self.connections.iter().find(|connection| connection.id == id)?;
        let tag = connection.environment.as_ref().and_then(|tag| self.tags.iter().find(|t| &t.id == tag));
        Some((connection, tag))
    }

    /// The connection of the selected node, and its tag.
    pub fn selected_connection(&self) -> Option<(Connection, Option<Tag>)> {
        let id = self.selected.as_deref()?;
        let connection_id = tree::row_of(&self.rows, id).and_then(|row| self.node_at(row))?.connection_id;
        self.connection(&connection_id).map(|(connection, tag)| (connection.clone(), tag.cloned()))
    }

    /// Where in its connection the selected node is.
    pub fn selected_path(&self) -> Vec<String> {
        self.selected
            .as_deref()
            .and_then(|id| tree::row_of(&self.rows, id))
            .and_then(|row| self.node_at(row))
            .map(|node| node.path)
            .unwrap_or_default()
    }

    /// What a "New query" on `node` opens: its database, and for a table its first rows.
    fn new_query(&self, node: &Node) -> Option<ExplorerEvent> {
        let (connection, _) = self.connection(&node.connection_id)?;
        let dialect = crate::sql::Dialect::of(connection.engine)?;
        let (database, sql) = match node.kind {
            NodeKind::Connection if connection.engine == Engine::Sqlite => (Some("main".to_owned()), None),
            NodeKind::Connection => (Some(connection.database.clone()).filter(|database| !database.is_empty()), None),
            NodeKind::Table | NodeKind::View => {
                let table = crate::sql::snippets::table_ref(&node.path)?;
                let sql = crate::sql::snippets::preview_query(dialect, &table);
                (Some(table.database), Some(sql))
            }
            _ => (node.path.first().cloned(), None),
        };
        Some(ExplorerEvent::NewQuery { connection_id: node.connection_id.clone(), database, sql })
    }

    fn query(&self, cx: &App) -> String {
        self.filter.read(cx).text().trim().to_owned()
    }

    /// Rebuilds the tree and the rows shown from what was listed.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let m = cx.messages();
        self.tree = tree::build_tree(&self.connections, &self.children, m);
        let query = self.query(cx);
        let rows = if query.is_empty() {
            tree::visible_rows(&self.tree, &|id| self.expanded.contains(id), m)
        } else {
            // While filtering, every listed match is shown open so it is visible.
            tree::visible_rows(&tree::filter_tree(self.tree.clone(), &query), &|_| true, m)
        };
        self.rows = Rc::new(rows);
        cx.notify();
    }

    /// Lists `nodes`' children again; what is on screen stays until each list arrives.
    fn load(&mut self, nodes: Vec<Node>, cx: &mut Context<Self>) {
        if nodes.is_empty() {
            return;
        }
        for node in nodes {
            let state = tree::start_loading(self.children.get(&node.id));
            self.children.insert(node.id.clone(), state);
            self.next_load += 1;
            let load = self.next_load;
            self.loads.insert(node.id.clone(), load);
            let (connection_id, path) = (node.connection_id, node.path);
            let listing = Backend::global(cx).spawn(cx, move |core| async move {
                explorer::list_objects(core.connections(), core.statement_log(), connection_id, path)
                    .await
                    .map_err(|error| error.to_string())
            });
            let id = node.id;
            cx.spawn(async move |this, cx| {
                let listed = listing.await;
                // Fails only when the explorer is gone.
                let _ = this.update(cx, |this, cx| this.listed(id, load, listed, cx));
            })
            .detach();
        }
        self.rebuild(cx);
    }

    fn listed(
        &mut self,
        id: String,
        load: u64,
        listed: Result<Vec<dbforge_core::drivers::DbObject>, String>,
        cx: &mut Context<Self>,
    ) {
        if self.loads.get(&id) != Some(&load) {
            return;
        }
        self.loads.remove(&id);
        let state = match listed {
            Ok(objects) => ChildState::Loaded(objects),
            Err(message) => ChildState::Error(message),
        };
        self.children.insert(id.clone(), state);
        self.rebuild(cx);
        if self.auto_expand.contains(&id) {
            self.expand_children(&id, cx);
        }
        #[cfg(debug_assertions)]
        if !self.auto_expand.is_empty() {
            self.print_rows();
        }
    }

    fn find(&self, id: &str) -> Option<&TreeNode> {
        fn search<'a>(nodes: &'a [TreeNode], id: &str) -> Option<&'a TreeNode> {
            nodes.iter().find_map(|tree| {
                if tree.node.id == id { Some(tree) } else { search(tree.children.as_deref().unwrap_or_default(), id) }
            })
        }
        search(&self.tree, id)
    }

    /// Opens or closes `node`; opening lists it the first time, and again after an error.
    fn toggle(&mut self, node: &Node, cx: &mut Context<Self>) {
        if !node.expandable {
            return;
        }
        if self.expanded.remove(&node.id) {
            self.rebuild(cx);
            return;
        }
        self.expanded.insert(node.id.clone());
        let listed = matches!(self.children.get(&node.id), Some(ChildState::Loaded(_) | ChildState::Loading(_)));
        if node.loads_children() && !listed {
            self.load(vec![node.clone()], cx);
        } else {
            self.rebuild(cx);
        }
    }

    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.expanded.clear();
        self.rebuild(cx);
    }

    /// Reads the saved connections and tags again, after one was saved,
    /// duplicated or deleted. Open connections stay open and keep their
    /// listings; a deleted one's are dropped. `changed` (a saved connection)
    /// is listed again where it is open, as its server may have; `select` is
    /// selected (a new connection).
    pub fn reload_connections(&mut self, changed: Option<&str>, select: Option<&str>, cx: &mut Context<Self>) {
        let core = Backend::global(cx).core();
        self.connections = core.connections().list();
        self.tags = core.tags().list();
        let present: HashSet<String> = self.connections.iter().map(|connection| connection.id.clone()).collect();
        let kept = |id: &String| present.contains(tree::connection_of(id));
        self.expanded.retain(kept);
        self.children.retain(|id, _| kept(id));
        self.loads.retain(|id, _| kept(id));
        self.auto_expand.retain(kept);
        if self.selected.as_ref().is_some_and(|id| !kept(id)) {
            self.selected = None;
        }
        self.rebuild(cx);
        if let Some(id) = changed.filter(|id| self.expanded.contains(*id)) {
            self.refresh(Some(id), cx);
        }
        if let Some(node) = select.and_then(|id| self.find(id)).map(|tree| tree.node.clone()) {
            self.select(&node, cx);
        }
    }

    /// Saves a copy of `connection`, its secrets included, then selects it.
    fn duplicate(&mut self, connection: &Connection, cx: &mut Context<Self>) {
        let (id, name) = (connection.id.clone(), (cx.messages().duplicate_name)(&connection.name));
        // The core copies the secrets, which may wait on the keychain.
        let duplicating = Backend::global(cx).spawn_blocking(cx, move |core| {
            dbforge_core::connections::commands::duplicate_connection(core.connections(), id, name)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let duplicated = duplicating.await;
            let _ = this.update(cx, |this, cx| match duplicated {
                Ok(copy) => {
                    this.reload_connections(None, Some(&copy.id), cx);
                    cx.emit(ExplorerEvent::ConnectionsChanged);
                }
                // Until the app has notifications, the reason goes to the log.
                Err(message) => eprintln!("DBForge could not duplicate the connection: {message}"),
            });
        })
        .detach();
    }

    /// Lists again every open node, or with `scope` (a node's id) only that node's part of the tree.
    pub fn refresh(&mut self, scope: Option<&str>, cx: &mut Context<Self>) {
        let nodes = tree::nodes_to_reload(&self.tree, &|id| self.expanded.contains(id), scope);
        self.load(nodes, cx);
    }

    fn select(&mut self, node: &Node, cx: &mut Context<Self>) {
        if let Some(index) = tree::row_of(&self.rows, &node.id) {
            self.scroll.scroll_to_item(index, ScrollStrategy::Center);
        }
        if self.selected.as_deref() == Some(node.id.as_str()) {
            return;
        }
        self.selected = Some(node.id.clone());
        cx.emit(ExplorerEvent::Selected { connection_id: node.connection_id.clone(), path: node.path.clone() });
        cx.notify();
    }

    fn selected_row(&self) -> Option<usize> {
        self.selected.as_deref().and_then(|id| tree::row_of(&self.rows, id))
    }

    fn node_at(&self, index: usize) -> Option<Node> {
        self.rows.get(index).and_then(TreeRow::node).cloned()
    }

    /// Moves the selection to the next node row up or down.
    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let nodes: Vec<usize> = (0..self.rows.len()).filter(|&index| self.rows[index].node().is_some()).collect();
        let next = match (self.selected_row().and_then(|row| nodes.iter().position(|&n| n == row)), forward) {
            (Some(at), true) => nodes.get(at + 1).or(nodes.last()),
            (Some(at), false) => nodes.get(at.saturating_sub(1)),
            (None, true) => nodes.first(),
            (None, false) => nodes.last(),
        };
        if let Some(node) = next.and_then(|&index| self.node_at(index)) {
            self.select(&node, cx);
        }
    }

    /// → opens the selected node, or moves into it when it is open.
    fn expand(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row() else { return self.step(true, cx) };
        let Some(TreeRow::Node { node, expanded, .. }) = self.rows.get(row).cloned() else { return };
        if !node.expandable {
            return;
        }
        if expanded || self.expanded.contains(&node.id) {
            self.step(true, cx);
        } else {
            self.toggle(&node, cx);
        }
    }

    /// ← closes the selected node, or moves to its parent when it is closed.
    fn collapse(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row() else { return };
        let Some(node) = self.node_at(row) else { return };
        if node.expandable && self.expanded.contains(&node.id) {
            self.toggle(&node, cx);
        } else if let Some(parent) = tree::parent_row(&self.rows, row).and_then(|index| self.node_at(index)) {
            self.select(&parent, cx);
        }
    }

    /// Double-click or Enter: opens a level of the tree, or a table, view or Redis database.
    fn activate(&mut self, node: &Node, window: &mut Window, cx: &mut Context<Self>) {
        if node.expandable {
            self.toggle(node, cx);
            return;
        }
        let opens = matches!(node.kind, NodeKind::Table | NodeKind::View)
            || (node.kind == NodeKind::Database && node.engine == Engine::Redis);
        if opens {
            window.dispatch_action(
                Box::new(OpenObject {
                    connection_id: node.connection_id.clone(),
                    kind: node.kind,
                    path: node.path.clone(),
                    view: None,
                }),
                cx,
            );
        }
    }

    fn on_filter(
        &mut self,
        _: &Entity<TextInput>,
        event: &TextInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TextInputEvent::Changed => self.rebuild(cx),
            // Enter goes to the first match, ready for the arrows.
            TextInputEvent::Submitted => {
                window.focus(&self.focus);
                if let Some(node) = self.rows.iter().find_map(TreeRow::node).cloned() {
                    self.select(&node, cx);
                }
            }
            TextInputEvent::Cancelled => {
                if self.filter.read(cx).text().is_empty() {
                    window.focus(&self.focus);
                } else {
                    self.filter.update(cx, |filter, cx| filter.set_text("", cx));
                    self.rebuild(cx);
                }
            }
            // The filter inserts its pastes itself.
            TextInputEvent::Pasted(_) => {}
        }
    }

    /// Opens `node`'s menu at `position`.
    fn open_menu(&mut self, node: &Node, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let m = cx.messages();
        let this = cx.entity().downgrade();
        let items = commands(node)
            .into_iter()
            .map(|command| {
                let (this, node) = (this.clone(), node.clone());
                MenuItem::new(command.label(m), move |window, cx| match command {
                    NodeCommand::OpenData | NodeCommand::ViewStructure => {
                        let view =
                            (command == NodeCommand::ViewStructure).then_some(TableTabView::Section(Section::Columns));
                        let open = OpenObject {
                            connection_id: node.connection_id.clone(),
                            kind: node.kind,
                            path: node.path.clone(),
                            view,
                        };
                        window.dispatch_action(Box::new(open), cx);
                    }
                    NodeCommand::ShowDiagram => {
                        if let Some((database, schema)) = diagram_target(&node) {
                            let _ = this.update(cx, |_, cx| {
                                let connection_id = node.connection_id.clone();
                                cx.emit(ExplorerEvent::OpenDiagram { connection_id, database, schema });
                            });
                        }
                    }
                    NodeCommand::NewQuery => {
                        let _ = this.update(cx, |this, cx| {
                            if let Some(event) = this.new_query(&node) {
                                cx.emit(event);
                            }
                        });
                    }
                    NodeCommand::Refresh => {
                        let _ = this.update(cx, |this, cx| this.refresh(Some(&node.id), cx));
                    }
                    NodeCommand::RefreshConnection => {
                        let _ = this.update(cx, |this, cx| this.refresh(Some(&node.connection_id), cx));
                    }
                    NodeCommand::CopyName => cx.write_to_clipboard(ClipboardItem::new_string(node.name.clone())),
                    NodeCommand::CopyConnectionUrl => {
                        let _ = this.update(cx, |this, cx| {
                            if let Some((connection, _)) = this.connection(&node.connection_id) {
                                cx.write_to_clipboard(ClipboardItem::new_string(connection_url(connection)));
                            }
                        });
                    }
                    NodeCommand::DuplicateConnection => {
                        let _ = this.update(cx, |this, cx| {
                            if let Some((connection, _)) = this.connection(&node.connection_id) {
                                let connection = connection.clone();
                                this.duplicate(&connection, cx);
                            }
                        });
                    }
                    NodeCommand::EditConnection | NodeCommand::DeleteConnection => {
                        let _ = this.update(cx, |this, cx| {
                            let Some((connection, _)) = this.connection(&node.connection_id) else { return };
                            let connection = connection.clone();
                            cx.emit(if command == NodeCommand::EditConnection {
                                ExplorerEvent::EditConnection(connection)
                            } else {
                                ExplorerEvent::DeleteConnection(connection)
                            });
                        });
                    }
                })
                .icon(command.icon())
            })
            .collect();
        let menu = cx.new(|cx| PopoverMenu::new(items, window, cx));
        let closing = cx.subscribe_in(&menu, window, |this, _, _: &gpui::DismissEvent, window, cx| {
            this.menu = None;
            window.focus(&this.focus);
            cx.notify();
        });
        self.menu = Some(OpenMenu { menu, position, _closing: closing });
        cx.notify();
    }

    /// Keeps the width the user dragged the explorer's edge to.
    fn resize(&mut self, width: f32, cx: &mut Context<Self>) {
        let width = width.clamp(MIN_WIDTH, MAX_WIDTH).round().to_string();
        if Preferences::global(cx).get(keys::EXPLORER_WIDTH) != Some(width.as_str()) {
            cx.update_global::<Preferences, _>(|preferences, _| preferences.set(keys::EXPLORER_WIDTH, Some(width)));
            cx.notify();
        }
    }

    /// Back to the default width (double-click on the edge).
    fn reset_width(&mut self, cx: &mut Context<Self>) {
        cx.update_global::<Preferences, _>(|preferences, _| preferences.set(keys::EXPLORER_WIDTH, None));
        cx.notify();
    }

    /// Opens the expandable children of `id` that loaded by themselves (development).
    fn expand_children(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(children) = self.find(id).and_then(|tree| tree.children.clone()) else { return };
        let mut load = Vec::new();
        for child in children.iter().filter(|child| child.node.expandable) {
            self.expanded.insert(child.node.id.clone());
            self.auto_expand.insert(child.node.id.clone());
            if child.node.loads_children() {
                load.push(child.node.clone());
            }
        }
        self.load(load, cx);
        self.rebuild(cx);
    }

    /// Development: the rows as text on stderr, after each listing the
    /// `DBFORGE_EXPAND` tree makes, to check the tree without looking at it.
    #[cfg(debug_assertions)]
    fn print_rows(&self) {
        let lines: Vec<String> = self
            .rows
            .iter()
            .map(|row| match row {
                TreeRow::Node { node, depth, expanded } => {
                    let state = if node.loading {
                        " (loading)"
                    } else if *expanded {
                        " (open)"
                    } else {
                        ""
                    };
                    let meta = node.meta.as_deref().map(|meta| format!(" [{meta}]")).unwrap_or_default();
                    format!("{}{}{meta}{state}", "  ".repeat(*depth), node.name)
                }
                TreeRow::Message { depth, text, .. } => format!("{}({text})", "  ".repeat(*depth)),
            })
            .collect();
        eprintln!("explorer:\n{}\n", lines.join("\n"));
    }

    /// `DBFORGE_EXPAND`, in development builds: a comma-separated list of
    /// connection ids (or `all`) whose whole tree opens at start, the first
    /// one selected, so a screenshot shows a listed tree without clicking.
    #[cfg(debug_assertions)]
    fn expand_for_development(&mut self, cx: &mut Context<Self>) {
        let Ok(wanted) = std::env::var("DBFORGE_EXPAND") else { return };
        let wanted: Vec<&str> = wanted.split(',').map(str::trim).collect();
        let roots: Vec<Node> = self
            .tree
            .iter()
            .map(|tree| tree.node.clone())
            .filter(|node| wanted.contains(&"all") || wanted.contains(&node.id.as_str()))
            .collect();
        if let Some(first) = roots.first() {
            self.select(first, cx);
        }
        for node in &roots {
            self.auto_expand.insert(node.id.clone());
            self.expanded.insert(node.id.clone());
        }
        self.load(roots, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_saved_width_within_its_range() {
        assert_eq!(width(None), 280.);
        assert_eq!(width(Some("320")), 320.);
        assert_eq!(width(Some("90")), 200.);
        assert_eq!(width(Some("4000")), 560.);
        assert_eq!(width(Some("0")), 280.);
        assert_eq!(width(Some("wide")), 280.);
    }
}
