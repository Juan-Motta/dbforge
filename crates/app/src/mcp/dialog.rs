//! An agent wants to change data on a connection whose tag asks first, as
//! the former web UI's `ApprovalDialog.tsx`: what it will run, where, and who
//! asks. Or it wants to create, update or delete a saved query: what the
//! query is, or what changes (each field before and after, and the SQL).
//! Decline has the focus, so Enter or Space pressed by mistake can't
//! approve; Tab moves between the two buttons, and Enter or Space presses
//! the one focused. Escape and the close button decline. Approve and run is
//! the only way an agent's change runs from here.

use dbforge_core::connections::model::Engine;
use dbforge_core::mcp::{
    ApprovalRequest, ApprovalSubject, DraftScope, SavedQueryChange, SavedQueryDraft, SavedQueryField,
};
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, actions, div, px,
};

use super::Approvals;
use crate::i18n::{Messages, Translate};
use crate::sql::Syntax;
use crate::theme::{ActiveTheme, Theme};
use crate::ui::code_editor::LINE_HEIGHT;
use crate::ui::field::Tooltip;
use crate::ui::{CodeEditor, button, glass, icon, icon_button, pill, primary_button};

actions!(approval_dialog, [Decline, Press, SwitchButton]);

const CONTEXT: &str = "ApprovalDialog";

/// The statement's box grows with it up to this height, then scrolls.
const STATEMENT_MAX_HEIGHT: f32 = 280.;

pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    vec![
        KeyBinding::new("escape", Decline, context),
        KeyBinding::new("enter", Press, context),
        KeyBinding::new("space", Press, context),
        KeyBinding::new("tab", SwitchButton, context),
        KeyBinding::new("shift-tab", SwitchButton, context),
    ]
}

pub struct ApprovalDialog {
    request: ApprovalRequest,
    /// The statement; for a saved query, its SQL (the new SQL of an update).
    statement: Entity<CodeEditor>,
    /// An update's SQL as it is now, when the update changes it.
    statement_before: Option<Entity<CodeEditor>>,
    decline: FocusHandle,
    approve: FocusHandle,
}

impl Focusable for ApprovalDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.decline.clone()
    }
}

impl ApprovalDialog {
    /// `request` on a connection of `engine` (its statement highlighted in
    /// that engine's language; PostgreSQL's when the connection is gone). A
    /// saved query's SQL is highlighted in the language it was written for.
    pub fn new(request: ApprovalRequest, engine: Option<Engine>, cx: &mut Context<Self>) -> Self {
        let (engine, text, before) = match &request.subject {
            ApprovalSubject::Statement { statement, .. } => (engine.unwrap_or(Engine::Postgres), statement, None),
            ApprovalSubject::SavedQuery(change) => match change.as_ref() {
                SavedQueryChange::Create { query } | SavedQueryChange::Delete { query } => {
                    (query.engine, &query.sql, None)
                }
                SavedQueryChange::Update { before, after } => {
                    (after.engine, &after.sql, change.changes_sql().then_some(before))
                }
            },
        };
        let statement = read_only_editor(text, engine, cx);
        let statement_before = before.map(|before| read_only_editor(&before.sql, before.engine, cx));
        Self { request, statement, statement_before, decline: cx.focus_handle(), approve: cx.focus_handle() }
    }

    pub fn id(&self) -> &str {
        &self.request.id
    }

    /// Gives Decline the keyboard.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.decline);
    }

    fn answer(&self, approved: bool, cx: &mut Context<Self>) {
        let id = self.request.id.clone();
        Approvals::global(cx).update(cx, |approvals, cx| approvals.answer(&id, approved, cx));
    }

    pub fn decline(&mut self, cx: &mut Context<Self>) {
        self.answer(false, cx);
    }

    /// Runs the agent's change: Approve and run, pressed.
    pub fn approve(&mut self, cx: &mut Context<Self>) {
        self.answer(true, cx);
    }

    /// Enter or Space: the focused button. Approves only while Approve has
    /// the keyboard; anything else declines, as Decline has it by default.
    fn press(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let approved = self.approve.is_focused(window);
        self.answer(approved, cx);
    }

    fn switch_button(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = if self.decline.is_focused(window) { &self.approve } else { &self.decline };
        window.focus(next);
        cx.notify();
    }

    fn render_who(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().mcp;
        let agent = self.request.agent.as_deref().unwrap_or(m.an_agent);
        let (who, tag) = match &self.request.subject {
            ApprovalSubject::Statement { connection_name, tag, .. } => {
                ((m.approval_who)(agent, connection_name), tag.as_deref())
            }
            ApprovalSubject::SavedQuery(change) => {
                let who = match change.as_ref() {
                    SavedQueryChange::Create { .. } => m.saved_create_who,
                    SavedQueryChange::Update { .. } => m.saved_update_who,
                    SavedQueryChange::Delete { .. } => m.saved_delete_who,
                };
                (who(agent, change.name()), None)
            }
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(13.))
            .child(icon("warning", theme.warn, 15.))
            .child(div().min_w_0().child(who))
            .children(tag.map(|tag| pill(tag, theme.err).flex_none()))
    }

    /// Where a statement runs, or what a saved query is or what changes in it.
    fn render_details(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let messages = cx.messages();
        let small = || div().text_size(px(12.)).text_color(theme.text3);
        match &self.request.subject {
            ApprovalSubject::Statement { database, .. } => div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .children(database.as_deref().map(|database| small().child((messages.mcp.in_database)(database))))
                .child(sql_box(&self.statement, theme, cx)),
            ApprovalSubject::SavedQuery(change) => {
                let rows = detail_rows(change, messages).into_iter().map(|row| {
                    let value = match row.before {
                        Some(before) => format!("{before}  →  {}", row.value),
                        None => row.value,
                    };
                    div()
                        .debug_selector(move || row.selector.to_owned())
                        .flex()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .child(div().w(px(110.)).flex_none().text_color(theme.text3).child(row.label))
                        .child(div().min_w_0().flex_1().child(value))
                });
                let sql = match (change.as_ref(), &self.statement_before) {
                    (SavedQueryChange::Update { .. }, Some(before)) => div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(small().child(messages.mcp.sql_before))
                        .child(sql_box(before, theme, cx).debug_selector(|| "approval-sql-before".into()))
                        .child(small().child(messages.mcp.sql_after))
                        .child(sql_box(&self.statement, theme, cx).debug_selector(|| "approval-sql-after".into())),
                    (SavedQueryChange::Update { .. }, None) => small().child(messages.mcp.sql_unchanged),
                    _ => div().child(sql_box(&self.statement, theme, cx)),
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(div().flex().flex_col().gap(px(4.)).children(rows))
                    .child(sql)
            }
        }
    }
}

/// A read-only editor showing `text` in `engine`'s language.
fn read_only_editor(text: &str, engine: Engine, cx: &mut Context<ApprovalDialog>) -> Entity<CodeEditor> {
    cx.new(|cx| {
        let mut editor = CodeEditor::new("", Syntax::console(engine), cx);
        editor.show_text(text, cx);
        editor.set_read_only(true, cx);
        editor
    })
}

/// `editor`'s box, growing with its text up to [`STATEMENT_MAX_HEIGHT`], then scrolling.
fn sql_box(editor: &Entity<CodeEditor>, theme: &Theme, cx: &App) -> gpui::Div {
    div()
        .h(px(sql_height(editor.read(cx).text())))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.line)
        .overflow_hidden()
        .flex()
        .child(editor.clone())
}

fn sql_height(text: &str) -> f32 {
    let lines = text.lines().count().max(1) as f32;
    (lines * LINE_HEIGHT + 2. * crate::ui::code_editor::PADDING_TOP).min(STATEMENT_MAX_HEIGHT)
}

/// A line of a saved query's details: a field, and its value (before and after, for an update).
#[derive(Debug, PartialEq)]
struct DetailRow {
    /// What tests find it by.
    selector: &'static str,
    label: &'static str,
    before: Option<String>,
    value: String,
}

/// What the dialog lists about `change`: the new or deleted query's scope,
/// folder and description; the fields an update changes, before and after.
fn detail_rows(change: &SavedQueryChange, m: &Messages) -> Vec<DetailRow> {
    let row = |field, draft: &SavedQueryDraft, before: Option<&SavedQueryDraft>| DetailRow {
        selector: selector(field),
        label: label(field, m),
        before: before.map(|before| field_value(field, before, m)),
        value: field_value(field, draft, m),
    };
    match change {
        SavedQueryChange::Create { query } | SavedQueryChange::Delete { query } => {
            [SavedQueryField::Scope, SavedQueryField::Folder, SavedQueryField::Description]
                .into_iter()
                .filter(|field| *field != SavedQueryField::Description || query.description.is_some())
                .map(|field| row(field, query, None))
                .collect()
        }
        SavedQueryChange::Update { before, after } => {
            change.changed_fields().into_iter().map(|field| row(field, after, Some(before))).collect()
        }
    }
}

fn selector(field: SavedQueryField) -> &'static str {
    match field {
        SavedQueryField::Name => "approval-field-name",
        SavedQueryField::Folder => "approval-field-folder",
        SavedQueryField::Scope => "approval-field-scope",
        SavedQueryField::Description => "approval-field-description",
    }
}

fn label(field: SavedQueryField, m: &Messages) -> &'static str {
    match field {
        SavedQueryField::Name => m.saved.name,
        SavedQueryField::Folder => m.saved.folder,
        SavedQueryField::Scope => m.saved.available_on,
        SavedQueryField::Description => m.saved.description,
    }
}

fn field_value(field: SavedQueryField, draft: &SavedQueryDraft, m: &Messages) -> String {
    match field {
        SavedQueryField::Name => draft.name.clone(),
        SavedQueryField::Folder => draft.folder.clone().unwrap_or_else(|| m.saved.no_folder.to_owned()),
        SavedQueryField::Description => draft.description.clone().unwrap_or_else(|| m.mcp.no_description.to_owned()),
        SavedQueryField::Scope => match &draft.scope {
            DraftScope::Connection { connection_name, .. } => connection_name.clone(),
            DraftScope::Engine => (m.saved.any_engine_connection)(draft.engine.label()),
        },
    }
}

/// The ring around a button that shows it has the keyboard; its room is
/// kept while it doesn't, so moving the focus moves nothing.
fn focus_ring(button: impl IntoElement, theme: &Theme, focused: bool) -> gpui::Div {
    div()
        .p(px(1.))
        .rounded(px(19.))
        .border_2()
        .border_color(if focused { theme.accent } else { gpui::transparent_black() })
        .child(button)
}

impl Render for ApprovalDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().mcp;
        let saved = matches!(self.request.subject, ApprovalSubject::SavedQuery(_));
        let (title, note, approve) = match saved {
            true => (m.saved_approval_title, m.saved_approval_note, m.approve_change),
            false => (m.approval_title, m.approval_note, m.approve),
        };
        let decline_focused = self.decline.is_focused(window);
        let approve_focused = self.approve.is_focused(window);
        glass(&theme)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Decline, _, cx| this.decline(cx)))
            .on_action(cx.listener(|this, _: &Press, window, cx| this.press(window, cx)))
            .on_action(cx.listener(|this, _: &SwitchButton, window, cx| this.switch_button(window, cx)))
            .bg(theme.dialog)
            .w(px(640.))
            .max_w_full()
            .rounded(px(22.))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(22.))
                    .pt(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(
                        icon_button(&theme, "approval-close", "close", true, false)
                            .tooltip(Tooltip::text(m.close))
                            .on_click(cx.listener(|this, _, _, cx| this.decline(cx))),
                    ),
            )
            .child(
                div()
                    .px(px(22.))
                    .pt(px(14.))
                    .pb(px(4.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(self.render_who(&theme, cx))
                    .child(self.render_details(&theme, cx))
                    .child(div().text_size(px(12.)).text_color(theme.text3).child(note)),
            )
            .child(
                div()
                    .px(px(22.))
                    .pt(px(12.))
                    .pb(px(18.))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(focus_ring(
                        button(&theme, None, m.decline)
                            .id("approval-decline")
                            .track_focus(&self.decline)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.decline(cx))),
                        &theme,
                        decline_focused,
                    ))
                    .child(focus_ring(
                        primary_button(&theme, approve)
                            .id("approval-approve")
                            .track_focus(&self.approve)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.approve(cx))),
                        &theme,
                        approve_focused,
                    )),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui::{TestAppContext, VisualTestContext};
    use tokio::task::JoinHandle;

    use super::*;
    use crate::backend::{Backend, runtime};
    use crate::events::{ChannelEvents, EventHub, channel};
    use crate::test_support::{draw, setup};

    fn request(id: &str) -> ApprovalRequest {
        ApprovalRequest {
            id: id.into(),
            agent: None,
            subject: ApprovalSubject::Statement {
                connection_id: "gone".into(),
                connection_name: "Shop".into(),
                tag: Some("PROD".into()),
                database: Some("shop".into()),
                statement: "DELETE FROM orders\nWHERE id = 1".into(),
            },
        }
    }

    /// The core asks, as the MCP server does for an agent's change, and the
    /// dialog shows the request with Decline focused. The task ends with the answer the agent gets.
    fn asked(cx: &mut TestAppContext) -> (JoinHandle<bool>, Entity<ApprovalDialog>, &mut VisualTestContext) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let events: Arc<ChannelEvents> = cx.update(|cx| {
            let (events, received) = channel();
            EventHub::start(received, cx);
            Approvals::start(cx);
            Arc::new(events)
        });
        let core = cx.update(|cx| Backend::global(cx).core().clone());
        let asking = runtime().spawn(async move { core.mcp_approvals().ask(events.as_ref(), request("a")).await });
        let shown = loop {
            cx.run_until_parked();
            if let Some(shown) = cx.update(|cx| Approvals::global(cx).read(cx).current().cloned()) {
                break shown;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let (dialog, cx) = cx.add_window_view(|_, cx| ApprovalDialog::new(shown, None, cx));
        dialog.update_in(cx, |dialog, window, _| dialog.focus(window));
        draw(cx);
        (asking, dialog, cx)
    }

    fn answer(asking: JoinHandle<bool>, cx: &mut VisualTestContext) -> bool {
        cx.run_until_parked();
        assert!(cx.update(|_, cx| Approvals::global(cx).read(cx).current().is_none()), "answered requests leave");
        runtime().block_on(asking).expect("the request's answer")
    }

    #[gpui::test]
    fn enter_pressed_by_mistake_declines(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (asking, _dialog, cx) = asked(cx);
        assert!(!asking.is_finished(), "nothing runs before the user answers");
        cx.simulate_keystrokes("enter");
        assert!(!answer(asking, cx));
    }

    #[gpui::test]
    fn space_on_decline_declines(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (asking, _dialog, cx) = asked(cx);
        cx.simulate_keystrokes("space");
        assert!(!answer(asking, cx));
    }

    #[gpui::test]
    fn escape_declines(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (asking, _dialog, cx) = asked(cx);
        cx.simulate_keystrokes("escape");
        assert!(!answer(asking, cx));
    }

    #[gpui::test]
    fn enter_approves_only_once_tab_moved_the_keyboard_to_approve(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (asking, dialog, cx) = asked(cx);
        cx.simulate_keystrokes("tab");
        assert!(dialog.update_in(cx, |dialog, window, _| dialog.approve.is_focused(window)));
        cx.simulate_keystrokes("tab");
        assert!(dialog.update_in(cx, |dialog, window, _| dialog.decline.is_focused(window)), "back to Decline");
        cx.simulate_keystrokes("shift-tab enter");
        assert!(answer(asking, cx));
    }

    fn draft(name: &str, sql: &str) -> SavedQueryDraft {
        SavedQueryDraft {
            name: name.into(),
            sql: sql.into(),
            description: Some("By status".into()),
            folder: Some("Ops".into()),
            scope: DraftScope::Connection { connection_id: "shop".into(), connection_name: "Shop".into() },
            engine: Engine::Postgres,
        }
    }

    fn update() -> SavedQueryChange {
        let before = draft("Orders", "select 1");
        let after = SavedQueryDraft {
            name: "Paid orders".into(),
            sql: "select *\nfrom orders\nwhere status = :status".into(),
            folder: None,
            scope: DraftScope::Engine,
            ..before.clone()
        };
        SavedQueryChange::Update { before, after }
    }

    #[test]
    fn an_update_lists_each_changed_field_before_and_after() {
        let m = crate::i18n::Language::En.messages();
        let rows = detail_rows(&update(), m);
        let shown: Vec<(&str, Option<&str>, &str)> =
            rows.iter().map(|row| (row.label, row.before.as_deref(), row.value.as_str())).collect();
        assert_eq!(
            shown,
            [
                ("Name", Some("Orders"), "Paid orders"),
                ("Folder", Some("Ops"), "No folder"),
                ("Available on", Some("Shop"), "Any PostgreSQL connection"),
            ]
        );
    }

    #[test]
    fn a_new_or_deleted_query_lists_where_it_is_and_what_it_says() {
        let m = crate::i18n::Language::En.messages();
        let rows = detail_rows(&SavedQueryChange::Create { query: draft("Orders", "select 1") }, m);
        let shown: Vec<(&str, Option<&str>, &str)> =
            rows.iter().map(|row| (row.label, row.before.as_deref(), row.value.as_str())).collect();
        assert_eq!(
            shown,
            [("Available on", None, "Shop"), ("Folder", None, "Ops"), ("Description", None, "By status")]
        );
        let plain = SavedQueryDraft { description: None, folder: None, ..draft("Orders", "select 1") };
        let rows = detail_rows(&SavedQueryChange::Delete { query: plain }, m);
        let labels: Vec<&str> = rows.iter().map(|row| row.label).collect();
        assert_eq!(labels, ["Available on", "Folder"], "no description, no line for it");
    }

    #[gpui::test]
    fn shows_a_saved_query_update_with_its_sql_before_and_after(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let request = ApprovalRequest {
            id: "a".into(),
            agent: Some("codex".into()),
            subject: ApprovalSubject::SavedQuery(Box::new(update())),
        };
        let (dialog, cx) = cx.add_window_view(|_, cx| ApprovalDialog::new(request, None, cx));
        draw(cx);

        for shown in ["approval-field-name", "approval-field-folder", "approval-field-scope"] {
            assert!(cx.debug_bounds(shown).is_some(), "{shown} shows");
        }
        assert!(cx.debug_bounds("approval-field-description").is_none(), "an unchanged field isn't listed");
        assert!(cx.debug_bounds("approval-sql-before").is_some() && cx.debug_bounds("approval-sql-after").is_some());
        dialog.update(cx, |dialog, cx| {
            assert_eq!(dialog.statement.read(cx).text(), "select *\nfrom orders\nwhere status = :status");
            let before = dialog.statement_before.as_ref().expect("the SQL before");
            assert_eq!(before.read(cx).text(), "select 1");
        });
    }

    #[gpui::test]
    fn an_update_that_keeps_the_sql_shows_it_unchanged(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let before = draft("Orders", "select 1");
        let after = SavedQueryDraft { name: "Renamed".into(), ..before.clone() };
        let request = ApprovalRequest {
            id: "a".into(),
            agent: None,
            subject: ApprovalSubject::SavedQuery(Box::new(SavedQueryChange::Update { before, after })),
        };
        let (dialog, cx) = cx.add_window_view(|_, cx| ApprovalDialog::new(request, None, cx));
        draw(cx);
        assert!(cx.debug_bounds("approval-field-name").is_some());
        assert!(cx.debug_bounds("approval-sql-before").is_none() && cx.debug_bounds("approval-sql-after").is_none());
        dialog.update(cx, |dialog, _| assert!(dialog.statement_before.is_none()));
    }
}
