//! The window's layout: the explorer on the left; the toolbar, the tab strip
//! and the main area on the right; and a dialog over them when one is open
//! (the connection dialogs, the notice of unreadable settings). Each part is
//! its own view, which its feature grows.

pub mod explorer;
pub mod set_aside;
mod toolbar;

use dbforge_core::connections::model::Connection;
use gpui::{
    AnyView, AppContext, Context, DismissEvent, Entity, Focusable, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Subscription, Window, div, linear_color_stop, linear_gradient, px,
};

pub use crate::workspace::Workspace;
pub use explorer::{Explorer, ExplorerEvent, OpenObject};
pub use set_aside::SetAsideDialog;
pub use toolbar::Toolbar;

use crate::backend::Backend;
use crate::connections::{ConnectionDialog, ConnectionDialogEvent, DeleteConnectionDialog, DeleteConnectionEvent};
use crate::i18n::Locale;
use crate::preferences::Preferences;
use crate::theme::{ActiveTheme, Theme};
use crate::workspace::WorkspaceEvent;

/// Space between the window's panels and around them.
const GAP: f32 = 10.;

/// The dialog over the window, and what listens to it until it closes.
struct OpenDialog {
    view: AnyView,
    _subscriptions: Vec<Subscription>,
}

/// The window's root view.
pub struct Shell {
    explorer: Entity<Explorer>,
    toolbar: Entity<Toolbar>,
    workspace: Entity<Workspace>,
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
        let toolbar = cx.new(|_| Toolbar::default());
        let mut subscriptions = subscriptions;
        subscriptions.push(cx.subscribe_in(&explorer, window, Self::on_explorer));
        let workspace = cx.new(|_| Workspace::default());
        // A tab's dialog (a table's PROD question): its owner closes it and takes the keyboard back.
        subscriptions.push(cx.subscribe(&workspace, |this, _, WorkspaceEvent::Dialog(dialog), cx| {
            this.dialog = dialog.clone().map(|view| OpenDialog { view, _subscriptions: Vec::new() });
            cx.notify();
        }));
        let mut shell = Self { explorer, toolbar, workspace, dialog: None, _subscriptions: subscriptions };
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
            ExplorerEvent::NewQuery { connection_id, database, sql } => {
                self.workspace.update(cx, |workspace, cx| {
                    workspace.new_query(connection_id, database.clone(), sql.clone(), window, cx);
                });
            }
            ExplorerEvent::OpenDiagram { connection_id, database, schema } => {
                self.workspace.update(cx, |workspace, cx| {
                    workspace.open_diagram(connection_id, database, schema, window, cx);
                });
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
            });
        self.show_dialog(dialog, vec![deleted], window, cx);
    }

    /// Shows `dialog` over the window until it closes; the explorer then takes the keyboard back.
    fn show_dialog<V: Render + gpui::EventEmitter<DismissEvent>>(
        &mut self,
        dialog: Entity<V>,
        mut subscriptions: Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        subscriptions.push(cx.subscribe_in(&dialog, window, |this, _, _: &DismissEvent, window, cx| {
            this.dialog = None;
            window.focus(&this.explorer.focus_handle(cx));
            cx.notify();
        }));
        self.dialog = Some(OpenDialog { view: dialog.into(), _subscriptions: subscriptions });
        cx.notify();
    }

    /// `DBFORGE_OPEN`, in development builds: a dialog to open at start, for
    /// screenshots without clicking: `new-connection`, `edit:<id>` or
    /// `delete:<id>`, and for the connection dialog `#ssl` or `#ssh` to show
    /// that section (`edit:<id>#ssh`), or `#engines` to open its engine list.
    /// `query:<id>` (or `query:<id>/<database>`) opens a query console on
    /// that connection, with `DBFORGE_SQL`'s script in it, run at once when
    /// there is one; `#history` opens the console's history too.
    /// `table:<id>/<path>` opens a table's rows, its path as the explorer
    /// names it (`table:<id>/orders` for SQLite, `<database>/<schema>/<table>`
    /// elsewhere). For screenshots, `#edits` adds pending edits once the rows
    /// load, `#review` shows their SQL, `#prod` applies them (asking first on
    /// a connection tagged PROD), `#inspect` opens the cell inspector,
    /// `#menu` a cell's menu, `#edit`/`#date` a cell's editor (text, date);
    /// `#columns`, `#indexes`, `#foreign-keys`, `#triggers`, `#ddl` or
    /// `#diagram` open the table's tab on that view instead of its rows.
    /// `diagram:<id>/<database>/<schema>` opens a schema's ER diagram.
    /// `query:<id>#explain` explains `DBFORGE_SQL`'s statement instead of
    /// running it, and `#analyze` explains it with Analyze on.
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
        match wanted.split_once(':') {
            Some(("table", target)) => {
                let section =
                    std::env::var("DBFORGE_OPEN").ok().and_then(|open| open.split_once('#').map(|(_, s)| s.to_owned()));
                return self.open_table_for_development(target, section, window, cx);
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
        let (connection_id, database) = match target.split_once('/') {
            Some((id, database)) => (id, Some(database.to_owned())),
            None => (target, None),
        };
        let sql = std::env::var("DBFORGE_SQL").ok().filter(|sql| !sql.trim().is_empty());
        let console = self
            .workspace
            .update(cx, |workspace, cx| workspace.new_query(connection_id, database, sql.clone(), window, cx));
        let Some(console) = console else {
            return eprintln!("DBFORGE_OPEN: no SQL connection {connection_id}");
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
fn dialog_layer(theme: &Theme, view: AnyView) -> gpui::Stateful<gpui::Div> {
    div()
        .id("dialog-scrim")
        .absolute()
        .inset_0()
        .occlude()
        .bg(theme.scrim)
        .flex()
        .items_center()
        .justify_center()
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
            .child(self.explorer.clone())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(GAP))
                    .child(self.toolbar.clone())
                    .child(self.workspace.clone()),
            )
            .children(self.dialog.as_ref().map(|dialog| dialog_layer(theme, dialog.view.clone())))
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
            div().size_full().relative().child(dialog_layer(&theme, self.select.clone().into()))
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
