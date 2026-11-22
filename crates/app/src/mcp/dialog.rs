//! An agent wants to change data on a connection whose tag asks first, as
//! the former web UI's `ApprovalDialog.tsx`: what it will run, where, and who asks.
//! Decline has the focus, so Enter or Space pressed by mistake can't
//! approve; Tab moves between the two buttons, and Enter or Space presses
//! the one focused. Escape and the close button decline. Approve and run is
//! the only way an agent's change runs from here.

use dbforge_core::connections::model::Engine;
use dbforge_core::mcp::ApprovalRequest;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, actions, div, px,
};

use super::Approvals;
use crate::i18n::Translate;
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
    statement: Entity<CodeEditor>,
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
    /// that engine's language; PostgreSQL's when the connection is gone).
    pub fn new(request: ApprovalRequest, engine: Option<Engine>, cx: &mut Context<Self>) -> Self {
        let syntax = Syntax::console(engine.unwrap_or(Engine::Postgres));
        let statement = cx.new(|cx| {
            let mut editor = CodeEditor::new("", syntax, cx);
            editor.show_text(&request.statement, cx);
            editor.set_read_only(true, cx);
            editor
        });
        Self { request, statement, decline: cx.focus_handle(), approve: cx.focus_handle() }
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
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(13.))
            .child(icon("warning", theme.warn, 15.))
            .child(div().min_w_0().child((m.approval_who)(agent, &self.request.connection_name)))
            .children(self.request.tag.as_deref().map(|tag| pill(tag, theme.err).flex_none()))
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
        let lines = self.request.statement.lines().count().max(1) as f32;
        let statement_height =
            (lines * LINE_HEIGHT + 2. * crate::ui::code_editor::PADDING_TOP).min(STATEMENT_MAX_HEIGHT);
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
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(m.approval_title),
                    )
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
                    .children(self.request.database.as_deref().map(|database| {
                        div().text_size(px(12.)).text_color(theme.text3).child((m.in_database)(database))
                    }))
                    .child(
                        div()
                            .h(px(statement_height))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(theme.line)
                            .overflow_hidden()
                            .flex()
                            .child(self.statement.clone()),
                    )
                    .child(div().text_size(px(12.)).text_color(theme.text3).child(m.approval_note)),
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
                        primary_button(&theme, m.approve)
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
            connection_id: "gone".into(),
            connection_name: "Shop".into(),
            tag: Some("PROD".into()),
            database: Some("shop".into()),
            statement: "DELETE FROM orders\nWHERE id = 1".into(),
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
}
