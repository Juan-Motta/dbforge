//! The window's layout: the explorer on the left; the toolbar, the tab strip
//! and the main area on the right, with the statement console under them
//! while it is open; and a dialog over them when one is open (the command
//! palette, the settings, the connection and admin dialogs, the notice of
//! unreadable settings). Each part is its own view, which its feature grows.
//! ⌘K / Ctrl+K toggles the palette, ⌘B / Ctrl+B the explorer and ⌘J /
//! Ctrl+J the statement console, from anywhere in the window.

pub mod explorer;
pub mod set_aside;
mod toolbar;

use dbforge_core::connections::model::Connection;
use gpui::{
    AnyView, AppContext, BorrowAppContext, Context, DismissEvent, Entity, Focusable, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, Styled, Subscription, Window, actions, div, linear_color_stop, linear_gradient,
    prelude::FluentBuilder, px,
};

pub use crate::workspace::Workspace;
pub use explorer::{Explorer, ExplorerEvent, OpenObject};
pub use set_aside::SetAsideDialog;
pub use toolbar::{Toolbar, ToolbarEvent};

use crate::admin::{AdminEvent, DropDatabaseDialog, NewObjectDialog, NewObjectKind};
use crate::backend::Backend;
use crate::connections::{ConnectionDialog, ConnectionDialogEvent, DeleteConnectionDialog, DeleteConnectionEvent};
use crate::i18n::{Locale, Translate};
use crate::palette::{CommandAction, CommandPalette, PaletteEvent, PaletteSource};
use crate::preferences::{Preferences, keys};
use crate::settings::{Section, SettingsDialog, SettingsEvent};
use crate::statement_log::{StatementConsole, StatementConsoleEvent};
use crate::theme::{ActiveTheme, Theme};
use crate::workspace::WorkspaceEvent;

actions!(shell, [TogglePalette, ToggleExplorer, ToggleStatements]);

/// The window's own shortcuts, answered wherever the keyboard is (the SQL editor included).
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("secondary-k", TogglePalette, None),
        KeyBinding::new("secondary-b", ToggleExplorer, None),
        KeyBinding::new("secondary-j", ToggleStatements, None),
    ]
}

/// Space between the window's panels and around them.
const GAP: f32 = 10.;

/// Where a dialog sits over the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    Center,
    /// Near the top, as a palette drops from the toolbar.
    Top,
}

/// The dialog over the window, and what listens to it until it closes.
struct OpenDialog {
    view: AnyView,
    placement: Placement,
    _subscriptions: Vec<Subscription>,
}

/// The statement console while it is open, and what the window hears from it.
struct OpenStatements {
    console: Entity<StatementConsole>,
    _events: Subscription,
}

/// The window's root view.
pub struct Shell {
    explorer: Entity<Explorer>,
    /// Hidden with ⌘B for more room; it stays alive so its open nodes survive.
    explorer_open: bool,
    toolbar: Entity<Toolbar>,
    workspace: Entity<Workspace>,
    statements: Option<OpenStatements>,
    dialog: Option<OpenDialog>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            // The OS switched between light and dark: "system" follows it.
            cx.observe_window_appearance(window, |_, window, cx| {
                if Theme::sync(window.appearance().into(), cx) {
                    window.refresh();
                }
            }),
            // A preference changed: the theme or the language may have.
            cx.observe_global_in::<Preferences>(window, |_, window, cx| {
                let themed = Theme::sync(window.appearance().into(), cx);
                let translated = Locale::sync(cx);
                if themed || translated {
                    window.refresh();
                }
            }),
        ];
        Theme::sync(window.appearance().into(), cx);
        let explorer = cx.new(|cx| Explorer::new(window, cx));
        let statements_open = crate::statement_log::is_open(Preferences::global(cx).get(keys::STATEMENTS_OPEN));
        let toolbar = cx.new(|_| Toolbar::new(true, statements_open));
        let mut subscriptions = subscriptions;
        subscriptions.push(cx.subscribe_in(&explorer, window, Self::on_explorer));
        subscriptions.push(cx.subscribe_in(&toolbar, window, Self::on_toolbar));
        let workspace = cx.new(|_| Workspace::default());
        // A tab's dialog (a table's PROD question): its owner closes it and takes the keyboard back.
        subscriptions.push(cx.subscribe(&workspace, |this, _, WorkspaceEvent::Dialog(dialog), cx| {
            this.dialog = dialog.clone().map(|view| OpenDialog {
                view,
                placement: Placement::Center,
                _subscriptions: Vec::new(),
            });
            cx.notify();
        }));
        let mut shell = Self {
            explorer,
            explorer_open: true,
            toolbar,
            workspace,
            statements: None,
            dialog: None,
            _subscriptions: subscriptions,
        };
        if statements_open {
            shell.set_statements(true, cx);
        }
        // A selection made before this subscription (development builds open a tree at start).
        shell.show_selected_connection(cx);
        shell.tell_of_set_aside_files(window, cx);
        #[cfg(debug_assertions)]
        shell.open_for_development(window, cx);
        shell
    }

    fn on_explorer(
        &mut self,
        explorer: &Entity<Explorer>,
        event: &ExplorerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // The toolbar shows the connection selected in the explorer.
            ExplorerEvent::Selected { connection_id, path } => {
                let connection = explorer
                    .read(cx)
                    .connection(connection_id)
                    .map(|(connection, tag)| (connection.clone(), tag.cloned()));
                let target = connection.as_ref().map(|(connection, _)| (connection.clone(), path.clone()));
                self.workspace.update(cx, |workspace, cx| workspace.set_target(target, cx));
                self.toolbar.update(cx, |toolbar, cx| toolbar.show(connection, cx));
            }
            ExplorerEvent::ConnectionsChanged => self.show_selected_connection(cx),
            ExplorerEvent::NewConnection => self.open_connection_dialog(None, window, cx),
            ExplorerEvent::EditConnection(connection) => {
                self.open_connection_dialog(Some(connection.clone()), window, cx)
            }
            ExplorerEvent::DeleteConnection(connection) => self.open_delete_dialog(connection.clone(), window, cx),
            ExplorerEvent::NewQuery { connection_id, scope, sql } => {
                self.workspace.update(cx, |workspace, cx| {
                    workspace.new_query(connection_id, scope.clone(), sql.clone(), window, cx);
                });
            }
            ExplorerEvent::OpenDiagram { connection_id, database, schema } => {
                self.workspace.update(cx, |workspace, cx| {
                    workspace.open_diagram(connection_id, database, schema, window, cx);
                });
            }
            ExplorerEvent::NewObject { connection, kind } => {
                self.open_new_object_dialog(connection.clone(), *kind, window, cx)
            }
            ExplorerEvent::DropDatabase { connection, database } => {
                self.open_drop_database_dialog(connection.clone(), database.clone(), window, cx)
            }
        }
    }

    fn on_toolbar(&mut self, _: &Entity<Toolbar>, event: &ToolbarEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            ToolbarEvent::Search => self.open_palette(window, cx),
            ToolbarEvent::ToggleExplorer => self.toggle_explorer(window, cx),
            ToolbarEvent::ToggleStatements => self.toggle_statements(cx),
            ToolbarEvent::OpenSettings => self.open_settings(Section::Appearance, window, cx),
        }
    }

    /// Hides the explorer for more room, or shows it again as it was.
    fn toggle_explorer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.explorer_open = !self.explorer_open;
        if !self.explorer_open && self.explorer.focus_handle(cx).contains_focused(window, cx) {
            window.blur();
        }
        self.sync_toolbar(cx);
    }

    fn toggle_statements(&mut self, cx: &mut Context<Self>) {
        let open = self.statements.is_none();
        self.set_statements(open, cx);
        let value = Some(open.to_string());
        cx.update_global::<Preferences, _>(|preferences, _| preferences.set(keys::STATEMENTS_OPEN, value));
    }

    /// Opens or closes the statement console. Closed, nothing listens for statements.
    fn set_statements(&mut self, open: bool, cx: &mut Context<Self>) {
        self.statements = open.then(|| {
            let console = cx.new(StatementConsole::new);
            let events = cx.subscribe(&console, |this, _, StatementConsoleEvent::Close, cx| {
                this.set_statements(false, cx);
                cx.update_global::<Preferences, _>(|preferences, _| {
                    preferences.set(keys::STATEMENTS_OPEN, Some(false.to_string()))
                });
            });
            OpenStatements { console, _events: events }
        });
        self.sync_toolbar(cx);
    }

    fn sync_toolbar(&mut self, cx: &mut Context<Self>) {
        let (explorer, statements) = (self.explorer_open, self.statements.is_some());
        self.toolbar.update(cx, |toolbar, cx| toolbar.set_panels(explorer, statements, cx));
        cx.notify();
    }

    /// ⌘K: opens the palette, or closes it when it is open. Another dialog keeps the keyboard.
    fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.dialog {
            Some(open) if open.view.clone().downcast::<CommandPalette>().is_ok() => self.close_dialog(window, cx),
            Some(_) => {}
            None => self.open_palette(window, cx),
        }
    }

    /// Opens the command palette on the selected connection's tables, with the commands that apply now.
    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            return;
        }
        let explorer = self.explorer.read(cx);
        let selected = explorer.selected_connection().map(|(connection, _)| connection);
        let path = explorer.selected_path();
        let source = selected.as_ref().map(|connection| PaletteSource {
            connection_id: connection.id.clone(),
            connection_name: connection.name.clone(),
            database: path.first().cloned(),
        });
        let commands = crate::palette::commands::commands(
            selected.as_ref().map(|connection| (connection, path.as_slice())),
            &cx.messages().palette,
        );
        let palette = cx.new(|cx| CommandPalette::new(source, commands, window, cx));
        palette.read(cx).focus(window, cx);
        let chosen = cx.subscribe_in(&palette, window, |this, _, event: &PaletteEvent, window, cx| {
            // The palette closes first, as the web UI's did, then what was chosen runs.
            this.close_dialog(window, cx);
            this.run_palette(event.clone(), window, cx);
        });
        self.show_dialog_at(palette, Placement::Top, vec![chosen], window, cx);
    }

    fn run_palette(&mut self, event: PaletteEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PaletteEvent::OpenTable { connection_id, table } => {
                self.workspace.update(cx, |workspace, cx| {
                    workspace.open_table(&connection_id, table, None, None, window, cx);
                });
            }
            PaletteEvent::Run(CommandAction::NewQuery) => {
                self.workspace.update(cx, |workspace, cx| workspace.new_query_on_target(window, cx));
            }
            PaletteEvent::Run(CommandAction::ShowDiagram { database, schema }) => {
                let Some((connection, _)) = self.explorer.read(cx).selected_connection() else { return };
                self.workspace.update(cx, |workspace, cx| {
                    workspace.open_diagram(&connection.id, &database, &schema, window, cx);
                });
            }
            PaletteEvent::Run(CommandAction::NewConnection) => self.open_connection_dialog(None, window, cx),
            PaletteEvent::Run(CommandAction::Theme(preference)) => preference.save(cx),
            PaletteEvent::Run(CommandAction::OpenSettings) => self.open_settings(Section::Appearance, window, cx),
        }
    }

    /// Opens the settings on `section`.
    pub fn open_settings(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| SettingsDialog::new(section, window, cx));
        dialog.read(cx).focus(window);
        let saved = cx.subscribe(&dialog, |this, _, SettingsEvent::TagsSaved { .. }: &SettingsEvent, cx| {
            // Badges show the new names and colours; connections lost a deleted tag.
            this.explorer.update(cx, |explorer, cx| explorer.reload_connections(None, None, cx));
            this.show_selected_connection(cx);
        });
        self.show_dialog(dialog, vec![saved], window, cx);
    }

    /// Asks for what creating `kind` on `connection`'s server needs, then creates it.
    pub fn open_new_object_dialog(
        &mut self,
        connection: Connection,
        kind: NewObjectKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = cx.new(|cx| NewObjectDialog::new(connection, kind, window, cx));
        dialog.read(cx).focus(window, cx);
        let done = cx.subscribe_in(&dialog, window, Self::on_admin);
        self.show_dialog(dialog, vec![done], window, cx);
    }

    /// Asks for `database`'s name again, then drops it from its server.
    pub fn open_drop_database_dialog(
        &mut self,
        connection: Connection,
        database: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = cx.new(|cx| DropDatabaseDialog::new(connection, database, window, cx));
        dialog.read(cx).focus(window, cx);
        let done = cx.subscribe_in(&dialog, window, Self::on_admin);
        self.show_dialog(dialog, vec![done], window, cx);
    }

    fn on_admin<V>(&mut self, _: &Entity<V>, event: &AdminEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            // A new database shows in the explorer.
            AdminEvent::Created { connection_id } => {
                self.explorer.update(cx, |explorer, cx| explorer.refresh(Some(connection_id), cx));
            }
            AdminEvent::Dropped { connection_id, database } => {
                self.explorer.update(cx, |explorer, cx| explorer.forget_database(connection_id, database, cx));
                self.workspace
                    .update(cx, |workspace, cx| workspace.close_database(connection_id, database, window, cx));
                self.show_selected_connection(cx);
            }
        }
    }

    /// The toolbar and "New query" follow the explorer's selection, which a
    /// save or delete may have changed.
    fn show_selected_connection(&mut self, cx: &mut Context<Self>) {
        let explorer = self.explorer.read(cx);
        let selected = explorer.selected_connection();
        let target = selected.as_ref().map(|(connection, _)| (connection.clone(), explorer.selected_path()));
        self.workspace.update(cx, |workspace, cx| workspace.set_target(target, cx));
        self.toolbar.update(cx, |toolbar, cx| toolbar.show(selected, cx));
    }

    /// Opens the connection dialog on `connection`, or for a new one.
    pub fn open_connection_dialog(
        &mut self,
        connection: Option<Connection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = cx.new(|cx| ConnectionDialog::new(connection, window, cx));
        dialog.read(cx).focus(window, cx);
        let saved = cx.subscribe(&dialog, |this, _, event: &ConnectionDialogEvent, cx| {
            let ConnectionDialogEvent::Saved(connection) = event;
            let id = connection.id.clone();
            let is_new = this.explorer.read(cx).connection(&id).is_none();
            this.explorer
                .update(cx, |explorer, cx| explorer.reload_connections(Some(&id), is_new.then_some(id.as_str()), cx));
            this.show_selected_connection(cx);
            this.refresh_statement_names(cx);
        });
        self.show_dialog(dialog, vec![saved], window, cx);
    }

    /// Asks before deleting `connection`.
    pub fn open_delete_dialog(&mut self, connection: Connection, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| DeleteConnectionDialog::new(connection, cx));
        dialog.read(cx).focus(window);
        let deleted =
            cx.subscribe(&dialog, |this, _, DeleteConnectionEvent::Deleted(id): &DeleteConnectionEvent, cx| {
                this.explorer.update(cx, |explorer, cx| explorer.reload_connections(None, None, cx));
                this.workspace.update(cx, |workspace, cx| workspace.close_connection(id, cx));
                this.show_selected_connection(cx);
                this.refresh_statement_names(cx);
            });
        self.show_dialog(dialog, vec![deleted], window, cx);
    }

    /// The statement console names connections; one was saved or deleted.
    fn refresh_statement_names(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = &self.statements {
            open.console.update(cx, |console, cx| console.refresh_connections(cx));
        }
    }

    /// Shows `dialog` over the window until it closes; the explorer then takes the keyboard back.
    fn show_dialog<V: Render + gpui::EventEmitter<DismissEvent>>(
        &mut self,
        dialog: Entity<V>,
        subscriptions: Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_dialog_at(dialog, Placement::Center, subscriptions, window, cx);
    }

    fn show_dialog_at<V: Render + gpui::EventEmitter<DismissEvent>>(
        &mut self,
        dialog: Entity<V>,
        placement: Placement,
        mut subscriptions: Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        subscriptions.push(cx.subscribe_in(&dialog, window, |this, _, _: &DismissEvent, window, cx| {
            this.close_dialog(window, cx);
        }));
        self.dialog = Some(OpenDialog { view: dialog.into(), placement, _subscriptions: subscriptions });
        cx.notify();
    }

    /// Closes the dialog shown; the explorer (or, hidden, the window) takes the keyboard back.
    fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = None;
        if self.explorer_open {
            window.focus(&self.explorer.focus_handle(cx));
        } else {
            window.blur();
        }
        cx.notify();
    }

    /// `DBFORGE_OPEN`, in development builds: a dialog to open at start, for
    /// screenshots without clicking: `new-connection`, `edit:<id>` or
    /// `delete:<id>`, and for the connection dialog `#ssl` or `#ssh` to show
    /// that section (`edit:<id>#ssh`), or `#engines` to open its engine list.
    /// `query:<id>` (or `query:<id>/<database>`, and for Cosmos DB
    /// `query:<id>/<database>/<container>`) opens a query console on that connection, with `DBFORGE_SQL`'s script in it, run at once when
    /// there is one; `#history` opens the console's history too.
    /// `table:<id>/<path>` opens a table's rows, its path as the explorer
    /// names it (`table:<id>/orders` for SQLite, `<database>/<schema>/<table>`
    /// elsewhere). For screenshots, `#edits` adds pending edits once the rows
    /// load, `#review` shows their SQL, `#prod` applies them (asking first on
    /// a connection tagged PROD), `#inspect` opens the cell inspector,
    /// `#menu` a cell's menu, `#edit`/`#date` a cell's editor (text, date);
    /// `#columns`, `#indexes`, `#foreign-keys`, `#triggers`, `#ddl` or
    /// `#diagram` open the table's tab on that view instead of its rows.
    /// On MongoDB and Cosmos DB, `#documents` shows the rows as documents,
    /// `#document-edits` with pending edits, `#document-edit` with one open
    /// in its editor, and `#document-menu` with a document's menu.
    /// `diagram:<id>/<database>/<schema>` opens a schema's ER diagram.
    /// `query:<id>#explain` explains `DBFORGE_SQL`'s statement instead of
    /// running it, and `#analyze` explains it with Analyze on.
    /// `keyspace:<id>/<database>` opens a Redis database's key browser
    /// (`keyspace:cache/db0`): `DBFORGE_KEY` selects a key, `#ttl`,
    /// `#rename` or `#delete` then opens that dialog on it, `#add-key` the
    /// dialog adding a key, `#list` shows the flat list, and `DBFORGE_SQL`
    /// runs a command in the CLI.
    /// `palette` opens the command palette (`palette#ord` with `ord`
    /// typed), `settings` the settings (`settings#tags`, `#passwords`…),
    /// `statement-log` the statement console, `new-object:<id>` the New
    /// database dialog (`#role`, `#user` for the others) and
    /// `drop-database:<id>/<database>` the Drop database dialog.
    #[cfg(debug_assertions)]
    fn open_for_development(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::connections::form::Section;
        let Ok(wanted) = std::env::var("DBFORGE_OPEN") else { return };
        let (wanted, section) = wanted.split_once('#').unwrap_or((wanted.as_str(), ""));
        let wanted_menu = section == "engines";
        let wanted_history = section == "history";
        let wanted_explain = match section {
            "explain" => Some(false),
            "analyze" => Some(true),
            _ => None,
        };
        let section = match section {
            "ssl" => Section::Ssl,
            "ssh" => Section::Ssh,
            _ => Section::General,
        };
        let saved = |id: &str| cx.global::<Backend>().core().connections().list().into_iter().find(|c| c.id == id);
        let seed = std::env::var("DBFORGE_OPEN").ok().and_then(|open| open.split_once('#').map(|(_, s)| s.to_owned()));
        match (wanted, wanted.split_once(':')) {
            ("palette", _) => {
                self.open_palette(window, cx);
                if let (Some(query), Some(open)) = (seed, &self.dialog)
                    && let Ok(palette) = open.view.clone().downcast::<CommandPalette>()
                {
                    palette.update(cx, |palette, cx| palette.set_query(&query, cx));
                }
                return;
            }
            ("settings", _) => {
                let section = seed.as_deref().and_then(crate::settings::Section::parse);
                let section = section.unwrap_or(crate::settings::Section::Appearance);
                return self.open_settings(section, window, cx);
            }
            ("statement-log", _) => {
                if self.statements.is_none() {
                    self.set_statements(true, cx);
                }
                return;
            }
            (_, Some(("new-object", id))) => {
                let kind = match seed.as_deref() {
                    Some("role") => NewObjectKind::Role,
                    Some("user") => NewObjectKind::User,
                    _ => NewObjectKind::Database,
                };
                match saved(id) {
                    Some(connection) => return self.open_new_object_dialog(connection, kind, window, cx),
                    None => return eprintln!("DBFORGE_OPEN: no connection {id}"),
                }
            }
            (_, Some(("drop-database", target))) => {
                let Some((id, database)) = target.split_once('/') else {
                    return eprintln!("DBFORGE_OPEN: drop-database:<id>/<database>, not {target}");
                };
                match saved(id) {
                    Some(connection) => {
                        return self.open_drop_database_dialog(connection, database.to_owned(), window, cx);
                    }
                    None => return eprintln!("DBFORGE_OPEN: no connection {id}"),
                }
            }
            _ => {}
        }
        match wanted.split_once(':') {
            Some(("table", target)) => {
                let section =
                    std::env::var("DBFORGE_OPEN").ok().and_then(|open| open.split_once('#').map(|(_, s)| s.to_owned()));
                return self.open_table_for_development(target, section, window, cx);
            }
            Some(("keyspace", target)) => {
                let seed =
                    std::env::var("DBFORGE_OPEN").ok().and_then(|open| open.split_once('#').map(|(_, s)| s.to_owned()));
                return self.open_keyspace_for_development(target, seed.as_deref(), window, cx);
            }
            Some(("query", target)) => {
                return self.open_query_for_development(target, wanted_history, wanted_explain, window, cx);
            }
            Some(("diagram", target)) => {
                let parts: Vec<&str> = target.split('/').collect();
                let [connection_id, database, schema] = parts[..] else {
                    return eprintln!("DBFORGE_OPEN: diagram:<id>/<database>/<schema>, not {target}");
                };
                let opened = self
                    .workspace
                    .update(cx, |workspace, cx| workspace.open_diagram(connection_id, database, schema, window, cx));
                if opened.is_none() {
                    eprintln!("DBFORGE_OPEN: no connection {connection_id}");
                }
                return;
            }
            None if wanted == "new-connection" => self.open_connection_dialog(None, window, cx),
            Some(("edit", id)) => match saved(id) {
                Some(connection) => self.open_connection_dialog(Some(connection), window, cx),
                None => return eprintln!("DBFORGE_OPEN: no connection {id}"),
            },
            Some(("delete", id)) => match saved(id) {
                Some(connection) => return self.open_delete_dialog(connection, window, cx),
                None => return eprintln!("DBFORGE_OPEN: no connection {id}"),
            },
            _ => return eprintln!("DBFORGE_OPEN: unknown dialog {wanted}"),
        }
        if let Some(open) = &self.dialog
            && let Ok(dialog) = open.view.clone().downcast::<ConnectionDialog>()
        {
            dialog.update(cx, |dialog, cx| dialog.show_section(section, cx));
            if wanted_menu {
                dialog.update(cx, |dialog, cx| dialog.open_engines(window, cx));
            }
        }
    }

    /// `DBFORGE_OPEN=query:…`: a console, its script run, its history shown.
    #[cfg(debug_assertions)]
    fn open_query_for_development(
        &mut self,
        target: &str,
        history: bool,
        explain: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // `query:<id>[/<database>[/<container>]]`
        let mut parts = target.split('/');
        let connection_id = parts.next().unwrap_or_default();
        let scope = dbforge_core::query::QueryScope {
            database: parts.next().map(str::to_owned),
            container: parts.next().map(str::to_owned),
        };
        let sql = std::env::var("DBFORGE_SQL").ok().filter(|sql| !sql.trim().is_empty());
        let console = self
            .workspace
            .update(cx, |workspace, cx| workspace.new_query(connection_id, scope, sql.clone(), window, cx));
        let Some(console) = console else {
            return eprintln!("DBFORGE_OPEN: no connection with a console {connection_id}");
        };
        console.update(cx, |console, cx| {
            match (sql, explain) {
                (Some(sql), Some(analyze)) => {
                    console.set_analyze(analyze, cx);
                    console.explain(sql, cx);
                }
                (Some(sql), None) => console.run(sql, cx),
                (None, _) => {}
            }
            if history {
                console.toggle_history(window, cx);
            }
        });
    }

    /// `DBFORGE_OPEN=keyspace:…`: a key browser, a key in it, a dialog, a CLI command.
    #[cfg(debug_assertions)]
    fn open_keyspace_for_development(
        &mut self,
        target: &str,
        seed: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::keyspace::{KeyAction, KeyView};
        let Some((connection_id, database)) = target.split_once('/') else {
            return eprintln!("DBFORGE_OPEN: keyspace:<id>/<database>, not {target}");
        };
        let browser =
            self.workspace.update(cx, |workspace, cx| workspace.open_keyspace(connection_id, database, window, cx));
        let Some(browser) = browser else { return eprintln!("DBFORGE_OPEN: no Redis connection {connection_id}") };
        let action = match seed {
            Some("ttl") => Some(KeyAction::Ttl),
            Some("rename") => Some(KeyAction::Rename),
            Some("delete") => Some(KeyAction::Delete),
            _ => None,
        };
        browser.update(cx, |browser, cx| {
            if let Ok(key) = std::env::var("DBFORGE_KEY") {
                let key = dbforge_core::drivers::redis::Text::from(key.as_str());
                browser.select(Some(key), action, window, cx);
            }
            match seed {
                Some("add-key") => browser.open_add_key(window, cx),
                Some("list") => browser.list().update(cx, |list, cx| list.set_view(KeyView::List, cx)),
                _ => {}
            }
            if let Ok(command) = std::env::var("DBFORGE_SQL") {
                browser.cli().update(cx, |cli, cx| cli.run(command, cx));
            }
        });
    }

    /// `DBFORGE_OPEN=table:…`: a table's rows, with what `seed` asks for once they load.
    #[cfg(debug_assertions)]
    fn open_table_for_development(
        &mut self,
        target: &str,
        seed: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut parts = target.split('/').map(str::to_owned);
        let Some(connection_id) = parts.next() else { return };
        let path: Vec<String> = parts.collect();
        let Some(table) = crate::sql::snippets::table_ref(&path) else {
            return eprintln!("DBFORGE_OPEN: no table in {target}");
        };
        use crate::structure::sections::Section;
        use crate::table::TableTabView;
        let view = match seed.as_deref() {
            Some("columns") => Some(TableTabView::Section(Section::Columns)),
            Some("indexes") => Some(TableTabView::Section(Section::Indexes)),
            Some("foreign-keys") => Some(TableTabView::Section(Section::ForeignKeys)),
            Some("triggers") => Some(TableTabView::Section(Section::Triggers)),
            Some("ddl") => Some(TableTabView::Section(Section::Ddl)),
            Some("diagram") => Some(TableTabView::Diagram),
            _ => None,
        };
        let page = self
            .workspace
            .update(cx, |workspace, cx| workspace.open_table(&connection_id, table, None, view, window, cx));
        let Some(page) = page else { return eprintln!("DBFORGE_OPEN: no connection {connection_id}") };
        if let Some(seed) = seed.filter(|_| view.is_none()) {
            let data = page.read(cx).data().clone();
            data.update(cx, |data, _| data.seed_for_development(seed));
        }
    }

    /// Opens the notice of settings files that couldn't be read, once, when there are any.
    fn tell_of_set_aside_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut files = Backend::global(cx).core().set_aside().0.clone();
        files.extend(Preferences::global(cx).set_aside().map(|path| path.display().to_string()));
        if files.is_empty() {
            return;
        }
        let dialog = cx.new(|cx| SetAsideDialog::new(files, cx));
        dialog.read(cx).focus(window);
        self.show_dialog(dialog, Vec::new(), window, cx);
    }
}

/// A dialog over the window, on its scrim. Render it as the window's last
/// child, so it paints over the rest. It must not be deferred: a dialog's
/// dropdowns are, and GPUI panics on a deferred draw inside another
/// ("cannot call defer_draw during deferred drawing").
fn dialog_layer(theme: &Theme, view: AnyView, placement: Placement) -> gpui::Stateful<gpui::Div> {
    div()
        .id("dialog-scrim")
        .absolute()
        .inset_0()
        .occlude()
        .bg(theme.scrim)
        .flex()
        .justify_center()
        .p(px(16.))
        .map(|layer| match placement {
            Placement::Center => layer.items_center(),
            Placement::Top => layer.items_start().pt(px(80.)),
        })
        .child(view)
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .size_full()
            .relative()
            .flex()
            .gap(px(GAP))
            .p(px(GAP))
            .bg(linear_gradient(180., linear_color_stop(theme.wall_top, 0.), linear_color_stop(theme.wall_bottom, 1.)))
            .text_color(theme.text)
            .on_action(cx.listener(|this, action: &OpenObject, window, cx| {
                this.workspace.update(cx, |workspace, cx| workspace.open(action, window, cx));
            }))
            .on_action(cx.listener(|this, _: &TogglePalette, window, cx| this.toggle_palette(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleExplorer, window, cx| this.toggle_explorer(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleStatements, _, cx| this.toggle_statements(cx)))
            .when(self.explorer_open, |shell| shell.child(self.explorer.clone()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(GAP))
                    .child(self.toolbar.clone())
                    .child(self.workspace.clone())
                    .children(self.statements.as_ref().map(|open| open.console.clone())),
            )
            .children(self.dialog.as_ref().map(|dialog| dialog_layer(theme, dialog.view.clone(), dialog.placement)))
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::theme::Appearance;
    use crate::ui::{Select, SelectOption};

    /// A window holding a dropdown in the dialog layer, as a dialog's would be.
    struct DialogHost {
        select: Entity<Select<u8>>,
    }

    impl Render for DialogHost {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let theme = *cx.theme();
            div().size_full().relative().child(dialog_layer(&theme, self.select.clone().into(), Placement::Center))
        }
    }

    #[gpui::test]
    fn a_dropdown_opens_inside_a_dialog(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Theme::new(Appearance::Dark));
        });
        let (host, cx) = cx.add_window_view(|_, cx| DialogHost {
            select: cx
                .new(|cx| Select::new(vec![SelectOption::new(1, "One"), SelectOption::new(2, "Two")], Some(1), cx)),
        });
        cx.run_until_parked();
        host.update_in(cx, |host, window, cx| host.select.update(cx, |select, cx| select.open(window, cx)));
        // Drawing the open list inside the dialog layer must not panic.
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(host.read_with(cx, |host, cx| host.select.read(cx).is_open()));
    }
}
