//! The window's layout: the explorer on the left; the toolbar, the tab strip
//! and the main area on the right; and a dialog over them when one is open.
//! Each part is its own view, which its feature grows.

pub mod explorer;
pub mod set_aside;
mod toolbar;
mod workspace;

use gpui::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, Styled, Subscription, Window,
    deferred, div, linear_color_stop, linear_gradient, px,
};

pub use explorer::{Explorer, ExplorerEvent, OpenObject};
pub use set_aside::SetAsideDialog;
pub use toolbar::Toolbar;
pub use workspace::Workspace;

use crate::backend::Backend;
use crate::i18n::Locale;
use crate::preferences::Preferences;
use crate::theme::{ActiveTheme, Theme};

/// Space between the window's panels and around them.
const GAP: f32 = 10.;

/// The window's root view.
pub struct Shell {
    explorer: Entity<Explorer>,
    toolbar: Entity<Toolbar>,
    workspace: Entity<Workspace>,
    dialog: Option<Entity<SetAsideDialog>>,
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
        // The toolbar shows the connection selected in the explorer.
        subscriptions.push(cx.subscribe(&explorer, |this, explorer, event: &ExplorerEvent, cx| {
            let ExplorerEvent::Selected { connection_id, .. } = event;
            let connection =
                explorer.read(cx).connection(connection_id).map(|(connection, tag)| (connection.clone(), tag.cloned()));
            this.toolbar.update(cx, |toolbar, cx| toolbar.show(connection, cx));
        }));
        // A selection made before this subscription (development builds open a tree at start).
        let selected = explorer.read(cx).selected_connection();
        toolbar.update(cx, |toolbar, cx| toolbar.show(selected, cx));
        let mut shell =
            Self { explorer, toolbar, workspace: cx.new(|_| Workspace), dialog: None, _subscriptions: subscriptions };
        shell.tell_of_set_aside_files(window, cx);
        shell
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
        self._subscriptions.push(cx.subscribe(&dialog, |this, _, _: &gpui::DismissEvent, cx| {
            this.dialog = None;
            cx.notify();
        }));
        self.dialog = Some(dialog);
    }
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
            // Deferred, so the dialog and its scrim paint over everything else.
            .children(self.dialog.clone().map(|dialog| {
                deferred(
                    div()
                        .id("dialog-scrim")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .bg(theme.scrim)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(dialog),
                )
                .with_priority(1)
            }))
    }
}
