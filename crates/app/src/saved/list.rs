//! The Saved view of a console's Library: the saved queries, searched by
//! name, description or folder, kept to the console's connection ("This
//! connection": its own and those of any connection of its engine) or not,
//! grouped by folder under headers that fold. A row shows a query's name,
//! description, `:parameters`, where it is offered and when it last ran;
//! on hover, Run, Open in a new tab and ⋯ (Rename, Duplicate, Delete) cover
//! the time, so nothing moves. A click loads a query into the console;
//! ⌘-click (Ctrl-click) opens it in a new tab.

use std::collections::HashSet;

use dbforge_core::connections::model::Engine;
use dbforge_core::saved_queries::SavedQuery;
use dbforge_core::saved_queries::params::parameters;
use gpui::{
    AnyElement, AnyView, AppContext, ClickEvent, Context, DismissEvent, Entity, EventEmitter, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render, StatefulInteractiveElement, Styled,
    Subscription, Window, div, px,
};

use super::dialogs::{DeleteSavedDialog, RenameDialog};
use super::rules::{Folder, Place, last_run, library, scope_label};
use super::store::{self, SavedStore};
use crate::backend::Backend;
use crate::history::NEW_TAB_CLICK;
use crate::i18n::{SavedMessages, Translate};
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::field::Tooltip;
use crate::ui::notice::{Tone, notify};
use crate::ui::{
    Ellipsis, MenuItem, PopoverMenu, TextInput, TextInputEvent, any, capsule, icon, icon_button, pill, popover,
    segment_face, segmented, shortcut,
};

/// What the Saved view asks of its console.
#[derive(Debug, Clone)]
pub enum SavedEvent {
    /// Put this query in the console (a click).
    Load(SavedQuery),
    Run(SavedQuery),
    /// Open this query in a new tab.
    Open(SavedQuery),
    /// Show this dialog over the window, or close the one shown.
    Dialog(Option<AnyView>),
}

pub struct SavedPanel {
    /// The console's connection and engine, what "This connection" keeps to.
    connection_id: String,
    engine: Engine,
    search: Entity<TextInput>,
    this_connection: bool,
    /// Folded folders, by name in lower case.
    folded: HashSet<String>,
    /// A query's ⋯ menu, open.
    menu: Option<(Entity<PopoverMenu>, Point<Pixels>, Subscription)>,
    /// What the panel hears from the dialog it showed.
    dialog: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SavedEvent> for SavedPanel {}

impl SavedPanel {
    pub fn new(connection_id: String, engine: Engine, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new(cx.messages().saved.search_placeholder, cx));
        let store = SavedStore::global(cx);
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |_, _, event: &TextInputEvent, _, cx| {
                if *event == TextInputEvent::Changed {
                    cx.notify();
                }
            }),
            cx.observe(&store, |_, _, cx| cx.notify()),
        ];
        Self {
            connection_id,
            engine,
            search,
            this_connection: true,
            folded: HashSet::new(),
            menu: None,
            dialog: None,
            _subscriptions: subscriptions,
        }
    }

    /// The console's connection, for "This connection".
    fn place(&self) -> Place<'_> {
        Place { connection_id: &self.connection_id, engine: self.engine }
    }

    /// The folders shown now, as searched and filtered.
    pub fn folders(&self, cx: &gpui::App) -> Vec<Folder> {
        let store = SavedStore::global_read(cx);
        let place = self.this_connection.then(|| self.place());
        library(store.map_or(&[], |store| store.queries()), self.search.read(cx).text(), place)
    }

    pub fn set_this_connection(&mut self, this_connection: bool, cx: &mut Context<Self>) {
        self.this_connection = this_connection;
        cx.notify();
    }

    fn toggle_folder(&mut self, name: &str, cx: &mut Context<Self>) {
        let key = name.to_lowercase();
        if !self.folded.remove(&key) {
            self.folded.insert(key);
        }
        cx.notify();
    }

    /// Shows `dialog` over the window; once it closes, the panel says so.
    fn show_dialog<V: Render + EventEmitter<DismissEvent>>(&mut self, dialog: Entity<V>, cx: &mut Context<Self>) {
        self.dialog = Some(cx.subscribe(&dialog, |this, _, _: &DismissEvent, cx| {
            this.dialog = None;
            cx.emit(SavedEvent::Dialog(None));
        }));
        cx.emit(SavedEvent::Dialog(Some(dialog.into())));
    }

    fn open_menu(&mut self, query: SavedQuery, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let m = &cx.messages().saved;
        let this = cx.entity().downgrade();
        let action = |label: &'static str,
                      icon_name: &'static str,
                      act: fn(&mut Self, SavedQuery, &mut Window, &mut Context<Self>)| {
            let (this, query) = (this.clone(), query.clone());
            MenuItem::new(label, move |window, cx| {
                let query = query.clone();
                let _ = this.update(cx, |this, cx| act(this, query, window, cx));
            })
            .icon(icon_name)
        };
        let items = vec![
            action(m.rename, "edit", Self::rename),
            action(m.duplicate, "duplicate", Self::duplicate),
            action(m.delete, "trash", Self::delete),
        ];
        let menu = cx.new(|cx| PopoverMenu::new(items, window, cx));
        let closing = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, position, closing));
        cx.notify();
    }

    fn rename(&mut self, query: SavedQuery, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| RenameDialog::new(query, window, cx));
        dialog.read(cx).focus(window, cx);
        self.show_dialog(dialog, cx);
    }

    fn duplicate(&mut self, query: SavedQuery, _: &mut Window, cx: &mut Context<Self>) {
        let duplicating = store::duplicate(query.id, cx);
        cx.spawn(async move |_, cx| {
            if let Err(error) = duplicating.await {
                let _ = cx.update(|cx| notify(error, Tone::Error, cx));
            }
        })
        .detach();
    }

    fn delete(&mut self, query: SavedQuery, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| DeleteSavedDialog::new(query, cx));
        dialog.read(cx).focus(window);
        self.show_dialog(dialog, cx);
    }

    fn render_folder(
        &self,
        theme: &Theme,
        index: usize,
        name: &str,
        count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let folded = self.folded.contains(&name.to_lowercase());
        let toggled = name.to_owned();
        div()
            .id(("saved-folder", index))
            .mx(px(6.))
            .px(px(8.))
            .h(px(28.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(9.))
            .cursor_pointer()
            .hover(|header| header.bg(theme.sel))
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.text2)
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_folder(&toggled, cx)))
            .child(icon(if folded { "right" } else { "down" }, theme.text3, 12.))
            .child(icon("folder", theme.text3, 14.))
            .child(div().flex_1().min_w_0().ellipsis().child(name.to_owned()))
            .child(div().flex_none().font_weight(FontWeight::NORMAL).text_color(theme.text3).child(count.to_string()))
            .into_any_element()
    }

    fn render_query(
        &self,
        theme: &Theme,
        index: usize,
        query: &SavedQuery,
        scope: String,
        now: i64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let m: &SavedMessages = &cx.messages().saved;
        let params = parameters(&query.sql, query.engine);
        let (clicked, run, open, more) = (query.clone(), query.clone(), query.clone(), query.clone());
        div()
            .id(("saved-query", index))
            .group("saved-query")
            .mx(px(6.))
            .px(px(8.))
            .py(px(7.))
            .flex()
            .flex_col()
            .gap(px(3.))
            .rounded(px(10.))
            .cursor_pointer()
            .hover(|row| row.bg(theme.sel))
            .on_click(cx.listener(move |_, event: &ClickEvent, _, cx| {
                cx.emit(if event.modifiers().secondary() {
                    SavedEvent::Open(clicked.clone())
                } else {
                    SavedEvent::Load(clicked.clone())
                });
            }))
            .child(
                div()
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .ellipsis()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(query.name.clone()),
                    )
                    // The time it last ran, and over it on hover the row's actions.
                    .child(
                        div()
                            .w(px(92.))
                            .h_full()
                            .flex_none()
                            .relative()
                            .child(
                                div()
                                    .size_full()
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .text_size(px(11.))
                                    .text_color(theme.text3)
                                    .group_hover("saved-query", |time| time.opacity(0.))
                                    .child(div().min_w_0().ellipsis().child(last_run(query, now, m))),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .gap(px(2.))
                                    .opacity(0.)
                                    .group_hover("saved-query", |actions| actions.opacity(1.))
                                    .child(
                                        icon_button(theme, ("saved-run", index), "play", true, false)
                                            .size(px(24.))
                                            .tooltip(Tooltip::text(m.run))
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                cx.stop_propagation();
                                                cx.emit(SavedEvent::Run(run.clone()));
                                            })),
                                    )
                                    .child(
                                        icon_button(theme, ("saved-open", index), "external", true, false)
                                            .size(px(24.))
                                            .tooltip(Tooltip::text(m.open_in_tab))
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                cx.stop_propagation();
                                                cx.emit(SavedEvent::Open(open.clone()));
                                            })),
                                    )
                                    .child(
                                        div()
                                            .id(("saved-more", index))
                                            .size(px(24.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(12.))
                                            .hover(|button| button.bg(theme.sel))
                                            .text_color(theme.text2)
                                            .font_weight(FontWeight::BOLD)
                                            .tooltip(Tooltip::text(m.more))
                                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                                cx.stop_propagation();
                                                if this.menu.is_none() {
                                                    let at = event.position() + gpui::point(px(0.), px(10.));
                                                    this.open_menu(more.clone(), at, window, cx);
                                                }
                                            }))
                                            .child("⋯"),
                                    ),
                            ),
                    ),
            )
            .children(query.description.clone().map(|description| {
                div().min_w_0().ellipsis().text_size(px(11.5)).text_color(theme.text2).child(description)
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .min_w_0()
                    .overflow_hidden()
                    .children(
                        params
                            .into_iter()
                            .map(|name| pill(&format!(":{name}"), theme.accent_text).flex_none().font_family(MONO)),
                    )
                    .child(div().min_w_0().ellipsis().text_size(px(11.)).text_color(theme.text3).child(scope)),
            )
            .into_any_element()
    }

    fn body(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let m = &cx.messages().saved;
        let folders = self.folders(cx);
        let note = |text: String| div().p(px(14.)).text_size(px(12.5)).text_color(theme.text3).child(text);
        if folders.is_empty() {
            let search = self.search.read(cx).text().trim().to_owned();
            let saved_any = SavedStore::global_read(cx).is_some_and(|store| !store.queries().is_empty());
            let text = if !search.is_empty() {
                (m.no_match)(&search)
            } else if saved_any && self.this_connection {
                m.none_for_connection.to_owned()
            } else {
                (m.empty)(&shortcut("s"))
            };
            return note(text).into_any_element();
        }
        let now = chrono::Utc::now().timestamp_millis();
        let connections = Backend::global(cx).core().connections().list();
        let name_of = |id: &str| connections.iter().find(|c| c.id == id).map(|c| c.name.clone());
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut index = 0;
        for (folder_index, folder) in folders.iter().enumerate() {
            if let Some(name) = &folder.name {
                rows.push(self.render_folder(theme, folder_index, name, folder.queries.len(), cx));
                if self.folded.contains(&name.to_lowercase()) {
                    index += folder.queries.len();
                    continue;
                }
            }
            for query in &folder.queries {
                let scope = scope_label(query, Some(&self.connection_id), name_of, m);
                rows.push(self.render_query(theme, index, query, scope, now, cx));
                index += 1;
            }
        }
        div()
            .id("saved-list")
            .size_full()
            .overflow_y_scroll()
            .pb(px(6.))
            .flex()
            .flex_col()
            .gap(px(1.))
            .children(rows)
            .into_any_element()
    }
}

impl Render for SavedPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().saved;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div().px(px(10.)).flex().child(
                    capsule(&theme)
                        .flex_1()
                        .min_w_0()
                        .h(px(32.))
                        .px(px(12.))
                        .child(icon("search", theme.text3, 14.))
                        .child(self.search.clone()),
                ),
            )
            .child(div().px(px(10.)).pt(px(8.)).flex().child(segmented(
                &theme,
                "saved-scope",
                Some(if self.this_connection { 0 } else { 1 }),
                vec![
                    any(segment_face(&theme, None, m.this_connection, self.this_connection)
                        .id("saved-scope-connection")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.set_this_connection(true, cx)))),
                    any(segment_face(&theme, None, m.all, !self.this_connection)
                        .id("saved-scope-all")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.set_this_connection(false, cx)))),
                ],
            )))
            .child(div().flex_1().min_h_0().pt(px(6.)).child(self.body(&theme, cx)))
            .child(
                div()
                    .flex_none()
                    .px(px(14.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(theme.line)
                    .text_size(px(11.))
                    .text_color(theme.text3)
                    .child((m.hint)(NEW_TAB_CLICK, &shortcut("s"))),
            )
            .children(self.menu.as_ref().map(|(menu, position, _)| popover(menu.clone(), *position)))
    }
}
