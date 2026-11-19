//! The window's layout: the explorer on the left; the toolbar, the tab strip
//! and the main area on the right, with the statement console under them
//! while it is open; and a dialog over them when one is open (the command
//! palette, the settings, the connection and admin dialogs, exporting and
//! importing rows, a database's dump and restore, the notice of unreadable
//! settings); over those, while a check found something to say, the update
//! dialog; and over everything, while an agent waits for the user, the
//! dialog approving its change; and at the bottom, over all of it, the
//! notice of what just happened (a copy, a failure), for a moment. Each
//! part is its own view, which its feature grows.
//! ⌘K / Ctrl+K toggles the palette, ⌘B / Ctrl+B the explorer and ⌘J /
//! Ctrl+J the statement console, from anywhere in the window.

pub mod connection_details;
pub mod connection_facts;
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
use crate::dump::{DumpEvent, DumpMode, ExportDatabaseDialog, ImportDatabaseDialog};
use crate::export::{ExportDialog, ExportSource};
use crate::i18n::{Locale, Translate};
use crate::import::{ImportDialog, ImportEvent, ImportTarget};
use crate::mcp::{ApprovalDialog, Approvals};
use crate::palette::{CommandAction, CommandPalette, PaletteEvent, PaletteSource};
use crate::preferences::{Preferences, keys};
use crate::settings::{Section, SettingsDialog, SettingsEvent};
use crate::statement_log::{StatementConsole, StatementConsoleEvent};
use crate::theme::{ActiveTheme, Theme};
use crate::ui::notice::{Notices, notice_layer};
use crate::updates::{UpdateDialog, Updates};
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

/// The request an agent waits on, over everything else, and where the
/// keyboard was before it took it.
struct OpenApproval {
    dialog: Entity<ApprovalDialog>,
    restore: Option<gpui::FocusHandle>,
}

/// The update dialog while a check has something to say, and where the
/// keyboard was before it took it.
struct OpenUpdate {
    dialog: Entity<UpdateDialog>,
    restore: Option<gpui::FocusHandle>,
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
    approval: Option<OpenApproval>,
    update: Option<OpenUpdate>,
    /// The notice at the bottom of the window.
    notices: Entity<Notices>,
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
                if translated {
                    crate::app::set_menus(cx);
                }
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
        subscriptions.push(cx.subscribe_in(
            &workspace,
            window,
            |this, _, event: &WorkspaceEvent, window, cx| match event {
                WorkspaceEvent::Dialog(dialog) => {
                    this.dialog = dialog.clone().map(|view| OpenDialog {
                        view,
                        placement: Placement::Center,
                        _subscriptions: Vec::new(),
                    });
                    cx.notify();
                }
                WorkspaceEvent::Export(source) => this.open_export_dialog(source.clone(), window, cx),
                WorkspaceEvent::Import(target) => this.open_import_dialog((**target).clone(), window, cx),
            },
        ));
        // An agent asked, or stopped waiting: its dialog shows, or the next one, or none.
        subscriptions.push(cx.observe_in(&Approvals::global(cx), window, |this, _, window, cx| {
            this.sync_approval(window, cx);
        }));
        // A check found an update, or the user asked: the dialog shows until it is dismissed.
        subscriptions.push(cx.observe_in(&Updates::global(cx), window, |this, _, window, cx| {
            this.sync_update(window, cx);
        }));
        // A notice came or went.
        let notices = Notices::global(cx);
        subscriptions.push(cx.observe(&notices, |_, _, cx| cx.notify()));
        let mut shell = Self {
            explorer,
            explorer_open: true,
            toolbar,
            workspace,
            statements: None,
            dialog: None,
            approval: None,
            update: None,
            notices,
            _subscriptions: subscriptions,
        };
        // Requests that came while the window opened.
        shell.sync_approval(window, cx);
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
            ExplorerEvent::ExportTable { connection, table } => {
                let source = ExportSource::Table {
                    connection_id: connection.id.clone(),
                    table: table.clone(),
                    filter: Default::default(),
                };
                self.open_export_dialog(source, window, cx);
            }
            ExplorerEvent::ImportRows { connection, table } => {
                // An open tab of the table shows the new rows on its next refresh, as in the former web UI.
                let target = ImportTarget { connection: connection.clone(), table: table.clone(), rows: None };
                self.open_import_dialog(target, window, cx);
            }
            ExplorerEvent::Dump { mode, connection, database, node_id } => {
                self.open_dump_dialog(*mode, connection.clone(), database.clone(), Some(node_id.clone()), window, cx);
            }
        }
    }

    /// Exports `source`'s rows to a file or the clipboard.
    pub fn open_export_dialog(&mut self, source: ExportSource, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| ExportDialog::new(source, window, cx));
        dialog.read(cx).focus(window);
        self.show_dialog(dialog, Vec::new(), window, cx);
    }

    /// Imports a file's rows into `target`'s table; the tab it came from reads its rows again once they are in.
    pub fn open_import_dialog(&mut self, target: ImportTarget, window: &mut Window, cx: &mut Context<Self>) {
        let rows = target.rows.clone();
        let dialog = cx.new(|cx| ImportDialog::new(target, window, cx));
        dialog.read(cx).focus(window);
        let imported = cx.subscribe(&dialog, move |_, _, ImportEvent::Imported, cx| {
            if let Some(rows) = rows.as_ref().and_then(|rows| rows.upgrade()) {
                rows.update(cx, |rows, cx| rows.reload(cx));
            }
        });
        self.show_dialog(dialog, vec![imported], window, cx);
    }

    /// A database's export with pg_dump, or a restore into it with psql; a
    /// restore lists the database's node (`node_id`) again.
    pub fn open_dump_dialog(
        &mut self,
        mode: DumpMode,
        connection: Connection,
        database: String,
        node_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match mode {
            DumpMode::Export => {
                let dialog = cx.new(|cx| ExportDatabaseDialog::new(connection, database, window, cx));
                dialog.read(cx).focus(window);
                self.show_dialog(dialog, Vec::new(), window, cx);
            }
            DumpMode::Import => {
                let dialog = cx.new(|cx| ImportDatabaseDialog::new(connection, database, window, cx));
                dialog.read(cx).focus(window);
                let imported = cx.subscribe(&dialog, move |this, _, DumpEvent::Imported, cx| {
                    this.explorer.update(cx, |explorer, cx| explorer.refresh(node_id.as_deref(), cx));
                });
                self.show_dialog(dialog, vec![imported], window, cx);
            }
        }
    }

    fn on_toolbar(&mut self, _: &Entity<Toolbar>, event: &ToolbarEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            ToolbarEvent::Search => self.open_palette(window, cx),
            ToolbarEvent::ToggleExplorer => self.toggle_explorer(window, cx),
            ToolbarEvent::ToggleStatements => self.toggle_statements(cx),
            ToolbarEvent::OpenSettings => self.open_settings(Section::Appearance, window, cx),
            ToolbarEvent::EditConnection(connection) => {
                self.open_connection_dialog(Some((**connection).clone()), window, cx)
            }
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
        if self.dialog.is_some() || self.approval.is_some() || self.update.is_some() {
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
            // The palette closes first, as the former web UI's did, then what was chosen runs.
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
        // Badges show new names and colours, connections lost a deleted tag,
        // or a backup brought connections in: the explorer reads them again.
        let saved = cx.subscribe(&dialog, |this, _, _: &SettingsEvent, cx| {
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
        if self.approval.is_none() && self.update.is_none() {
            self.focus_explorer(window, cx);
        }
        cx.notify();
    }

    fn focus_explorer(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.explorer_open {
            window.focus(&self.explorer.focus_handle(cx));
        } else {
            window.blur();
        }
    }

    /// Shows the request agents have waited on longest, each in a dialog of
    /// its own with Decline focused; once none waits, the keyboard goes back
    /// where it was. The window is not brought forward: a key meant for the
    /// agent's terminal must not answer.
    fn sync_approval(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = Approvals::global(cx).read(cx).current().cloned();
        let shown = self.approval.as_ref().map(|open| open.dialog.read(cx).id().to_owned());
        if current.as_ref().map(|request| request.id.as_str()) == shown.as_deref() {
            return;
        }
        let restore = match self.approval.take() {
            Some(open) => open.restore,
            None => window.focused(cx),
        };
        match current {
            Some(request) => {
                let engine = Backend::global(cx)
                    .core()
                    .connections()
                    .list()
                    .into_iter()
                    .find(|connection| connection.id == request.connection_id)
                    .map(|connection| connection.engine);
                let dialog = cx.new(|cx| ApprovalDialog::new(request, engine, cx));
                dialog.read(cx).focus(window);
                self.approval = Some(OpenApproval { dialog, restore });
            }
            None => match restore {
                Some(handle) => window.focus(&handle),
                None if self.dialog.is_none() => self.focus_explorer(window, cx),
                None => window.blur(),
            },
        }
        cx.notify();
    }

    /// Shows the update dialog while the updater has something to say, over
    /// any other dialog (Settings › About asks from one); once it closes, the
    /// keyboard goes back where it was.
    fn sync_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown = Updates::global(cx).read(cx).phase().shown();
        match (shown, self.update.is_some()) {
            (true, false) => {
                let restore = window.focused(cx);
                let dialog = cx.new(UpdateDialog::new);
                if self.approval.is_none() {
                    dialog.read(cx).focus(window);
                }
                self.update = Some(OpenUpdate { dialog, restore });
            }
            (false, true) => {
                let restore = self.update.take().and_then(|open| open.restore);
                if self.approval.is_none() {
                    match restore {
                        Some(handle) => window.focus(&handle),
                        None if self.dialog.is_none() => self.focus_explorer(window, cx),
                        None => window.blur(),
                    }
                }
            }
            _ => return,
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
    /// `update` checks for updates as the menu does (point it at a local
    /// manifest with `DBFORGE_UPDATE_ENDPOINT`), and `update#install` installs
    /// what it finds (over `DBFORGE_UPDATE_INSTALL_PATH`, a stand-in);
    /// `update#downloading`, `#installing` and `#failed` show a sample update
    /// in that phase.
    /// `approval` shows a sample of an agent's request to approve, on the
    /// first saved connection (`#plain`: no agent name, tag or database;
    /// `#many`: two waiting); answering it reaches the core, which has
    /// nothing waiting under its id, so nothing runs.
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
            ("approval", _) => return self.show_approval_for_development(seed.as_deref(), cx),
            ("notice", _) => {
                let (text, tone) = match seed.as_deref() {
                    Some("error") => (
                        "connection refused: is the server running on host \"db.internal\" (10.0.0.7) and \
                         accepting TCP/IP connections on port 5432?",
                        crate::ui::notice::Tone::Error,
                    ),
                    _ => (cx.messages().shell.copied, crate::ui::notice::Tone::Info),
                };
                return crate::ui::notice::notify(text, tone, cx);
            }
            (_, Some(("connection-details", id))) => {
                let found = self.explorer.read(cx).connection(id).map(|(c, tag)| (c.clone(), tag.cloned()));
                let Some(found) = found else { return eprintln!("DBFORGE_OPEN: no connection {id}") };
                self.toolbar.update(cx, |toolbar, cx| {
                    toolbar.show(Some(found), cx);
                    toolbar.toggle_details(window, cx);
                });
                return;
            }
            (_, Some((kind @ ("table-menu" | "view-menu"), target))) => {
                let mut parts = target.split('/').map(str::to_owned);
                let id = parts.next().unwrap_or_default();
                let path: Vec<String> = parts.collect();
                self.explorer.update(cx, |explorer, cx| {
                    explorer.open_table_menu_for_development(&id, path, kind == "view-menu", window, cx)
                });
                return;
            }
            ("update", _) => return show_update_for_development(seed.as_deref(), cx),
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
            Some((kind @ ("export" | "import" | "export-database" | "import-database"), target)) => {
                return self.open_transfer_for_development(kind, target, seed.as_deref().unwrap_or(""), window, cx);
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
            if seed.as_deref() == Some("failure") {
                dialog.update(cx, |dialog, cx| dialog.fail_for_development(window, cx));
            }
        }
    }

    /// `DBFORGE_OPEN=approval`: sample requests, as if agents had asked.
    #[cfg(debug_assertions)]
    fn show_approval_for_development(&mut self, seed: Option<&str>, cx: &mut Context<Self>) {
        use dbforge_core::mcp::ApprovalRequest;
        let connection = Backend::global(cx).core().connections().list().into_iter().next();
        let (connection_id, connection_name) =
            connection.map_or(("sample".to_owned(), "Shop".to_owned()), |connection| (connection.id, connection.name));
        let plain = seed == Some("plain");
        let sample = |id: &str, statement: &str| ApprovalRequest {
            id: format!("development-{id}"),
            agent: (!plain).then(|| "claude-code".to_owned()),
            connection_id: connection_id.clone(),
            connection_name: connection_name.clone(),
            tag: (!plain).then(|| "PROD".to_owned()),
            database: (!plain).then(|| "shop".to_owned()),
            statement: statement.to_owned(),
        };
        let mut requests =
            vec![sample("1", "UPDATE orders\nSET status = 'shipped', shipped_at = CURRENT_TIMESTAMP\nWHERE id = 42;")];
        if seed == Some("many") {
            requests.push(sample("2", "DELETE FROM carts WHERE updated_at < '2026-01-01';"));
        }
        Approvals::global(cx).update(cx, |approvals, cx| {
            for request in requests {
                approvals.show_for_development(request, cx);
            }
        });
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

    /// `DBFORGE_OPEN=export:<id>/<path>` or `import:<id>/<path>` (a table, as
    /// `table:` names it), `export-database:<id>/<database>` or
    /// `import-database:<id>/<database>`: that dialog. Native file dialogs
    /// can't be driven, so `DBFORGE_PICK` answers them (`ui::file_prompt`):
    /// an import picks its file at once, and `#run` presses the main button
    /// once, as soon as it can be pressed (on a connection whose tag asks
    /// first, that only shows the warning).
    #[cfg(debug_assertions)]
    fn open_transfer_for_development(
        &mut self,
        kind: &str,
        target: &str,
        section: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut parts = target.split('/').map(str::to_owned);
        let Some(connection_id) = parts.next() else { return };
        let path: Vec<String> = parts.collect();
        let Some(connection) =
            Backend::global(cx).core().connections().list().into_iter().find(|c| c.id == connection_id)
        else {
            return eprintln!("DBFORGE_OPEN: no connection {connection_id}");
        };
        let press = section == "run";
        let picked = std::env::var_os("DBFORGE_PICK").is_some();
        match kind {
            "export-database" | "import-database" => {
                let [database] = &path[..] else { return eprintln!("DBFORGE_OPEN: {kind}:<id>/<database>") };
                let mode = if kind == "export-database" { DumpMode::Export } else { DumpMode::Import };
                self.open_dump_dialog(mode, connection, database.clone(), None, window, cx);
            }
            _ => {
                let Some(table) = crate::sql::snippets::table_ref(&path) else {
                    return eprintln!("DBFORGE_OPEN: no table in {target}");
                };
                if kind == "export" {
                    let source = ExportSource::Table { connection_id, table, filter: Default::default() };
                    self.open_export_dialog(source, window, cx);
                } else {
                    self.open_import_dialog(ImportTarget { connection, table, rows: None }, window, cx);
                }
            }
        }
        let Some(view) = self.dialog.as_ref().map(|open| open.view.clone()) else { return };
        if let Ok(dialog) = view.clone().downcast::<ImportDialog>() {
            if picked {
                dialog.update(cx, |dialog, cx| dialog.pick_for_development(window, cx));
            }
            if press {
                press_when_ready(dialog, ImportDialog::ready, ImportDialog::start, window, cx);
            }
        } else if let Ok(dialog) = view.clone().downcast::<ImportDatabaseDialog>() {
            if picked {
                dialog.update(cx, |dialog, cx| dialog.choose(window, cx));
            }
            if press {
                press_when_ready(
                    dialog,
                    ImportDatabaseDialog::ready_for_development,
                    ImportDatabaseDialog::start,
                    window,
                    cx,
                );
            }
        } else if press && let Ok(dialog) = view.clone().downcast::<ExportDialog>() {
            press_when_ready(dialog, ExportDialog::ready_for_development, ExportDialog::export_to_file, window, cx);
        } else if press && let Ok(dialog) = view.downcast::<ExportDatabaseDialog>() {
            press_when_ready(dialog, |_, _| true, ExportDatabaseDialog::start, window, cx);
        }
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

/// Development only: presses a dialog's main button once, as soon as `ready`
/// says it can be (its file read, its columns known), giving up after 15 s.
#[cfg(debug_assertions)]
fn press_when_ready<V: 'static>(
    dialog: Entity<V>,
    ready: fn(&V, &gpui::App) -> bool,
    press: fn(&mut V, &mut Window, &mut Context<V>),
    window: &mut Window,
    cx: &mut Context<Shell>,
) {
    cx.spawn_in(window, async move |_, cx| {
        for _ in 0..75 {
            cx.background_executor().timer(std::time::Duration::from_millis(200)).await;
            let pressed = dialog.update_in(cx, |dialog, window, cx| {
                let can = ready(dialog, cx);
                if can {
                    press(dialog, window, cx);
                }
                can
            });
            if !matches!(pressed, Ok(false)) {
                return;
            }
        }
    })
    .detach();
}

/// `DBFORGE_OPEN=update`: a check, or a sample update in the phase `seed` names.
#[cfg(debug_assertions)]
fn show_update_for_development(seed: Option<&str>, cx: &mut Context<Shell>) {
    use std::sync::Arc;

    use crate::updates::{Phase, Stage, Update};

    let update = Arc::new(Update {
        version: "0.4.0".into(),
        current_version: dbforge_core::VERSION.into(),
        notes: Some(
            "## Features\n\n- **app:** a native window, drawn with GPUI\n- **updates:** install and restart\n\n\
             ## Fixes\n\n- **explorer:** keep the tree on refresh"
                .into(),
        ),
        url: "http://127.0.0.1:9/never".into(),
        signature: String::new(),
        installer: crate::updates::target::Installer::App,
    });
    let phase = match seed {
        Some("downloading") => Some(Phase::Installing {
            update,
            stage: Stage::Downloading { downloaded: 12 * 1024 * 1024 + 300 * 1024, total: Some(31 * 1024 * 1024) },
        }),
        Some("installing") => Some(Phase::Installing { update, stage: Stage::Installing }),
        Some("failed") => {
            Some(Phase::Failed { error: "The update's signature is not valid".into(), update: Some(update) })
        }
        _ => None,
    };
    Updates::global(cx).update(cx, |updates, cx| match phase {
        Some(phase) => updates.show_for_development(phase, cx),
        None if seed == Some("install") => updates.check_and_install_for_development(cx),
        None => updates.check(true, cx),
    });
}

/// A dialog over the window, on its scrim. Render it as the window's last
/// child, so it paints over the rest. It must not be deferred: a dialog's
/// dropdowns are, and GPUI panics on a deferred draw inside another
/// ("cannot call defer_draw during deferred drawing").
fn dialog_layer(id: &'static str, theme: &Theme, view: AnyView, placement: Placement) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
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
        crate::menu_bar::handle_window_commands(div())
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
            .on_action(cx.listener(|this, _: &crate::menu_bar::About, window, cx| {
                this.open_settings(Section::About, window, cx)
            }))
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
            .children(
                self.dialog
                    .as_ref()
                    .map(|dialog| dialog_layer("dialog-scrim", theme, dialog.view.clone(), dialog.placement)),
            )
            .children(
                self.update
                    .as_ref()
                    .map(|open| dialog_layer("update-scrim", theme, open.dialog.clone().into(), Placement::Center)),
            )
            // Last, so it covers any other dialog.
            .children(
                self.approval
                    .as_ref()
                    .map(|open| dialog_layer("approval-scrim", theme, open.dialog.clone().into(), Placement::Center)),
            )
            // Over everything, taking no clicks.
            .children(self.notices.read(cx).current().map(|notice| notice_layer(theme, notice)))
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
            div().size_full().relative().child(dialog_layer(
                "dialog-scrim",
                &theme,
                self.select.clone().into(),
                Placement::Center,
            ))
        }
    }

    fn approval(id: &str) -> dbforge_core::mcp::ApprovalRequest {
        dbforge_core::mcp::ApprovalRequest {
            id: id.into(),
            agent: Some("codex".into()),
            connection_id: "gone".into(),
            connection_name: "Shop".into(),
            tag: Some("PROD".into()),
            database: None,
            statement: "DELETE FROM orders".into(),
        }
    }

    #[gpui::test]
    fn shows_each_waiting_request_in_turn_over_any_dialog_then_gives_the_keyboard_back(cx: &mut TestAppContext) {
        use dbforge_core::{Event, Events};
        let _dir = crate::test_support::setup(cx);
        let events = cx.update(|cx| {
            let (events, received) = crate::events::channel();
            crate::events::EventHub::start(received, cx);
            Approvals::start(cx);
            cx.bind_keys(crate::mcp::key_bindings());
            cx.bind_keys(crate::settings::key_bindings());
            events
        });
        let (shell, cx) = cx.add_window_view(Shell::new);
        shell.update_in(cx, |shell, window, cx| shell.open_settings(Section::Tags, window, cx));
        let before = cx.update(|window, cx| window.focused(cx));
        let shown = |cx: &mut gpui::VisualTestContext| {
            shell.read_with(cx, |shell, cx| shell.approval.as_ref().map(|open| open.dialog.read(cx).id().to_owned()))
        };

        events.emit(Event::McpApprovalAsked(approval("a")));
        events.emit(Event::McpApprovalAsked(approval("b")));
        cx.run_until_parked();
        crate::test_support::draw(cx);
        assert_eq!(shown(cx).as_deref(), Some("a"));
        assert!(shell.read_with(cx, |shell, _| shell.dialog.is_some()), "the settings stay under it");
        let decline = cx.update(|window, cx| {
            shell.read(cx).approval.as_ref().map(|open| open.dialog.focus_handle(cx).is_focused(window))
        });
        assert_eq!(decline, Some(true), "Decline has the keyboard");

        // The agent left: the next request shows.
        events.emit(Event::McpApprovalWithdrawn { id: "a".into() });
        cx.run_until_parked();
        assert_eq!(shown(cx).as_deref(), Some("b"));
        crate::test_support::draw(cx);

        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert_eq!(shown(cx), None);
        assert_eq!(cx.update(|window, cx| window.focused(cx)), before, "the keyboard goes back to the settings");
        assert!(shell.read_with(cx, |shell, _| shell.dialog.is_some()), "Escape declined; it closed nothing else");
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
