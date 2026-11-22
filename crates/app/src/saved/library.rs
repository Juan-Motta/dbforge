//! The Library beside a console (⌘Y): its runs (History) and its saved
//! queries (Saved), one at a time under a pill choice that remembers the
//! last one chosen. It passes on what either asks of the console.

use dbforge_core::connections::model::Engine;
use dbforge_core::history::HistoryEntry;
use dbforge_core::saved_queries::SavedQuery;
use gpui::{
    AnyView, App, AppContext, BorrowAppContext, Context, Entity, EventEmitter, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px,
};

use super::list::{SavedEvent, SavedPanel};
use super::store::SavedStore;
use crate::history::{HistoryEvent, HistoryPanel};
use crate::i18n::Translate;
use crate::preferences::{Preferences, keys};
use crate::theme::ActiveTheme;
use crate::ui::field::Tooltip;
use crate::ui::{any, icon, segment_face, segmented, spinner};

/// The panel's width.
pub const WIDTH: f32 = 340.;

/// What the Library shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryView {
    History,
    Saved,
}

impl LibraryView {
    /// The view saved under [`keys::LIBRARY_VIEW`]; History the first time.
    pub fn parse(saved: Option<&str>) -> Self {
        match saved {
            Some("saved") => Self::Saved,
            _ => Self::History,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::History => "history",
            Self::Saved => "saved",
        }
    }
}

/// What the Library asks of its console.
#[derive(Debug, Clone)]
pub enum LibraryEvent {
    /// Put this run's SQL in the console, without running it.
    LoadRun(HistoryEntry),
    /// Open this run's SQL in a new console on its own connection.
    OpenRun(HistoryEntry),
    /// Put this saved query in the console.
    LoadSaved(SavedQuery),
    RunSaved(SavedQuery),
    /// Open this saved query in a new tab.
    OpenSaved(SavedQuery),
    /// Show this dialog over the window, or close the one shown.
    Dialog(Option<AnyView>),
    Close,
}

pub struct LibraryPanel {
    view: LibraryView,
    history: Entity<HistoryPanel>,
    saved: Entity<SavedPanel>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LibraryEvent> for LibraryPanel {}

impl LibraryPanel {
    pub fn new(connection_id: String, engine: Engine, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let view = LibraryView::parse(cx.try_global::<Preferences>().and_then(|p| p.get(keys::LIBRARY_VIEW)));
        let history = cx.new(|cx| HistoryPanel::new(connection_id.clone(), window, cx));
        let saved = cx.new(|cx| SavedPanel::new(connection_id, engine, window, cx));
        let store = SavedStore::global(cx);
        let subscriptions = vec![
            cx.subscribe(&history, |_, _, event: &HistoryEvent, cx| {
                cx.emit(match event {
                    HistoryEvent::Load(entry) => LibraryEvent::LoadRun(entry.clone()),
                    HistoryEvent::OpenNew(entry) => LibraryEvent::OpenRun(entry.clone()),
                });
            }),
            cx.subscribe(&saved, |_, _, event: &SavedEvent, cx| {
                cx.emit(match event {
                    SavedEvent::Load(query) => LibraryEvent::LoadSaved(query.clone()),
                    SavedEvent::Run(query) => LibraryEvent::RunSaved(query.clone()),
                    SavedEvent::Open(query) => LibraryEvent::OpenSaved(query.clone()),
                    SavedEvent::Dialog(dialog) => LibraryEvent::Dialog(dialog.clone()),
                });
            }),
            // The header's spinner and count follow the views.
            cx.observe(&history, |_, _, cx| cx.notify()),
            cx.observe(&store, |_, _, cx| cx.notify()),
        ];
        Self { view, history, saved, _subscriptions: subscriptions }
    }

    pub fn view(&self) -> LibraryView {
        self.view
    }

    pub fn history(&self) -> &Entity<HistoryPanel> {
        &self.history
    }

    pub fn saved(&self) -> &Entity<SavedPanel> {
        &self.saved
    }

    /// Shows `view`, and remembers it for the next Library opened.
    pub fn show(&mut self, view: LibraryView, cx: &mut Context<Self>) {
        if self.view == view {
            return;
        }
        self.view = view;
        if cx.has_global::<Preferences>() {
            cx.update_global::<Preferences, _>(|preferences, _| {
                preferences.set(keys::LIBRARY_VIEW, Some(view.as_str().into()))
            });
        }
        cx.notify();
    }

    /// Reads the runs again, after one.
    pub fn reload_history(&self, cx: &mut App) {
        self.history.update(cx, |history, cx| history.reload(cx));
    }
}

impl Render for LibraryPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = cx.messages();
        let history = self.view == LibraryView::History;
        let loading = history && self.history.read(cx).is_loading();
        let count = SavedStore::global_read(cx).map_or(0, |store| store.queries().len());
        let saved_label = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(m.saved.saved)
            .child(div().font_weight(FontWeight::NORMAL).text_color(theme.text3).child(count.to_string()));
        div()
            .w(px(WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(48.))
                    .flex_none()
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(segmented(
                        &theme,
                        "library-view",
                        Some(if history { 0 } else { 1 }),
                        vec![
                            any(segment_face(&theme, Some("clock"), m.query.history_panel.title, history)
                                .id("library-history")
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| this.show(LibraryView::History, cx)))),
                            any(segment_face(&theme, None, "", !history)
                                .px(px(12.))
                                .id("library-saved")
                                .cursor_pointer()
                                .child(saved_label)
                                .on_click(cx.listener(|this, _, _, cx| this.show(LibraryView::Saved, cx)))),
                        ],
                    ))
                    // The spinner keeps its room, so the header never moves while a list loads.
                    .child(
                        div()
                            .size(px(14.))
                            .when(loading, |slot| slot.child(spinner("library-loading", theme.text3, 14.))),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("close-library")
                            .size(px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(13.))
                            .cursor_pointer()
                            .hover(|button| button.bg(theme.sel))
                            .tooltip(Tooltip::text(m.saved.close_library))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(LibraryEvent::Close)))
                            .child(icon("close", theme.text2, 13.)),
                    ),
            )
            .child(
                div().flex_1().min_h_0().map(|body| {
                    if history { body.child(self.history.clone()) } else { body.child(self.saved.clone()) }
                }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_on_history_until_saved_is_chosen() {
        assert_eq!(LibraryView::parse(None), LibraryView::History);
        assert_eq!(LibraryView::parse(Some("nonsense")), LibraryView::History);
        for view in [LibraryView::History, LibraryView::Saved] {
            assert_eq!(LibraryView::parse(Some(view.as_str())), view);
        }
    }
}
