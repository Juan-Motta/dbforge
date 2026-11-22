//! Saving a console the first time (⌘S): a small form under the console's
//! toolbar asking for the query's name (required), its folder (one already
//! used, none, or a new one typed in place of the list), where it is offered
//! (this connection, or any connection of its engine) and a description,
//! listing the `:parameters` its SQL asks for. Enter saves; Escape closes.

use dbforge_core::connections::model::Engine;
use dbforge_core::saved_queries::params::parameters;
use dbforge_core::saved_queries::{SavedQuery, SavedQueryInput, SavedScope};
use gpui::{
    App, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, Window, actions, div, prelude::FluentBuilder, px,
};

use super::rules::folder_names;
use super::store::{self, SavedStore};
use crate::i18n::Translate;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::field::{CONTROL_HEIGHT, Tooltip, field, frame, help, label, text_box};
use crate::ui::{
    Select, SelectEvent, SelectOption, TextInput, TextInputEvent, any, button, enabled, glass, pill, primary_button,
    segment_face, segmented, shadow, shortcut,
};

actions!(save_query_form, [Confirm, Dismiss]);

const CONTEXT: &str = "SaveQueryForm";

/// The form's width.
pub const WIDTH: f32 = 380.;

/// Enter saves (a closed folder list leaves it to the form); Escape closes.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("enter", Confirm, Some(CONTEXT)), KeyBinding::new("escape", Dismiss, Some(CONTEXT))]
}

/// What the form tells its console, besides closing ([`DismissEvent`]).
#[derive(Debug, Clone, PartialEq)]
pub enum SaveFormEvent {
    Saved(SavedQuery),
}

/// A choice of the folder list.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FolderChoice {
    None,
    Existing(String),
    New,
}

/// What the console being saved is.
pub struct Draft {
    pub connection_id: String,
    pub connection_name: String,
    pub engine: Engine,
    pub sql: String,
}

pub struct SaveForm {
    draft: Draft,
    params: Vec<String>,
    name: Entity<TextInput>,
    folder: Entity<Select<FolderChoice>>,
    new_folder: Entity<TextInput>,
    /// A new folder's name is typed in place of the list.
    typing_folder: bool,
    any_connection: bool,
    description: Entity<TextInput>,
    /// Save was asked for without a name.
    name_missing: bool,
    saving: bool,
    error: Option<String>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for SaveForm {}
impl EventEmitter<SaveFormEvent> for SaveForm {}

impl Focusable for SaveForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SaveForm {
    pub fn new(draft: Draft, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let m = &cx.messages().saved;
        let params = parameters(&draft.sql, draft.engine);
        let store = SavedStore::global(cx);
        let mut options = vec![SelectOption::new(FolderChoice::None, m.no_folder)];
        options.extend(
            folder_names(store.read(cx).queries())
                .into_iter()
                .map(|name| SelectOption::new(FolderChoice::Existing(name.clone()), name)),
        );
        options.push(SelectOption::new(FolderChoice::New, m.new_folder));
        let folder = cx.new(|cx| Select::new(options, Some(FolderChoice::None), cx));
        let name = cx.new(|cx| TextInput::new(m.name_placeholder, cx));
        let new_folder = cx.new(|cx| TextInput::new(m.new_folder_placeholder, cx));
        let description = cx.new(|cx| TextInput::new(m.description_placeholder, cx));
        let mut subscriptions = vec![cx.subscribe_in(&folder, window, Self::on_folder)];
        for input in [&name, &new_folder, &description] {
            subscriptions.push(cx.subscribe_in(input, window, Self::on_input));
        }
        Self {
            draft,
            params,
            name,
            folder,
            new_folder,
            typing_folder: false,
            any_connection: false,
            description,
            name_missing: false,
            saving: false,
            error: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The name field takes the keyboard.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        self.name.read(cx).focus(window);
    }

    /// Types `name` as the user would, for tests.
    #[cfg(test)]
    pub fn set_name(&mut self, name: &str, cx: &mut Context<Self>) {
        self.name.update(cx, |input, cx| input.set_text(name, cx));
    }

    pub fn set_any_connection(&mut self, any_connection: bool, cx: &mut Context<Self>) {
        self.any_connection = any_connection;
        cx.notify();
    }

    fn on_input(
        &mut self,
        input: &Entity<TextInput>,
        event: &TextInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TextInputEvent::Changed => {
                if *input == self.name {
                    self.name_missing = false;
                }
                self.error = None;
                cx.notify();
            }
            TextInputEvent::Submitted => self.save(window, cx),
            TextInputEvent::Cancelled if *input == self.new_folder => self.choose_folder(window, cx),
            TextInputEvent::Cancelled => cx.emit(DismissEvent),
            TextInputEvent::Pasted(_) => {}
        }
    }

    fn on_folder(
        &mut self,
        _: &Entity<Select<FolderChoice>>,
        SelectEvent(choice): &SelectEvent<FolderChoice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if *choice == FolderChoice::New {
            self.typing_folder = true;
            self.new_folder.read(cx).focus(window);
            cx.notify();
        }
    }

    /// Back from typing a new folder to the list, on no folder.
    fn choose_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.typing_folder = false;
        self.new_folder.update(cx, |input, cx| input.set_text("", cx));
        self.folder.update(cx, |select, cx| select.set_value(Some(FolderChoice::None), cx));
        self.folder.read(cx).focus(window);
        cx.notify();
    }

    fn folder_value(&self, cx: &App) -> Option<String> {
        if self.typing_folder {
            return Some(self.new_folder.read(cx).text().to_owned());
        }
        match self.folder.read(cx).value() {
            Some(FolderChoice::Existing(name)) => Some(name.clone()),
            _ => None,
        }
    }

    /// What is saved, or `None` without a name.
    fn input(&self, cx: &App) -> Option<SavedQueryInput> {
        let name = self.name.read(cx).text().trim().to_owned();
        if name.is_empty() {
            return None;
        }
        let description = Some(self.description.read(cx).text().to_owned());
        let scope = if self.any_connection {
            SavedScope::Engine
        } else {
            SavedScope::Connection { connection_id: self.draft.connection_id.clone() }
        };
        Some(SavedQueryInput {
            name,
            sql: self.draft.sql.clone(),
            description,
            folder: self.folder_value(cx),
            scope,
            engine: self.draft.engine,
        })
    }

    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some(input) = self.input(cx) else {
            self.name_missing = true;
            self.focus(window, cx);
            cx.notify();
            return;
        };
        self.saving = true;
        let saving = store::save(None, input, cx);
        cx.spawn(async move |this, cx| {
            let saved = saving.await;
            let _ = this.update(cx, |this, cx| match saved {
                Ok(query) => {
                    cx.emit(SaveFormEvent::Saved(query));
                    cx.emit(DismissEvent);
                }
                Err(error) => {
                    this.error = Some(error);
                    this.saving = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn folder_control(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        if !self.typing_folder {
            return any(self.folder.clone());
        }
        let theme = *cx.theme();
        let m = &cx.messages().saved;
        let focused = self.new_folder.read(cx).is_focused(window);
        // The same frame and height as the list it replaces, so nothing moves.
        any(frame(&theme, focused, false, false)
            .text_color(theme.text)
            .child(div().flex_1().min_w_0().child(self.new_folder.clone()))
            .child(
                div()
                    .id("choose-folder")
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(10.))
                    .cursor_pointer()
                    .hover(|button| button.bg(theme.sel))
                    .tooltip(Tooltip::text(m.choose_folder))
                    .on_click(cx.listener(|this, _, window, cx| this.choose_folder(window, cx)))
                    .child(crate::ui::icon("close", theme.text3, 11.)),
            ))
    }
}

impl Render for SaveForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().saved;
        let engine = self.draft.engine.label();
        let any_connection = self.any_connection;
        let scope_note =
            if any_connection { (m.any_engine_note)(engine) } else { (m.only_here_note)(&self.draft.connection_name) };
        let parameters: gpui::AnyElement = if self.params.is_empty() {
            help(&theme, m.no_parameters, None).mt(px(0.)).into_any_element()
        } else {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(5.))
                .children(self.params.iter().map(|name| pill(&format!(":{name}"), theme.accent_text).font_family(MONO)))
                .child(div().text_size(px(12.)).text_color(theme.text3).child(m.asked_when_it_runs))
                .into_any_element()
        };
        glass(&theme)
            .id("save-query-form")
            .occlude()
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.save(window, cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(DismissEvent)))
            .bg(theme.dialog)
            .shadow(shadow(30., 14., 0x00000059))
            .w(px(WIDTH))
            .rounded(px(18.))
            .p(px(18.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(m.save_title))
                    .child(div().flex_1())
                    .child(div().text_size(px(12.)).text_color(theme.text3).child(shortcut("s"))),
            )
            .child(field(
                label(&theme, "save-name", m.name, None),
                text_box(&self.name, self.name_missing, false, window, cx),
                None,
            ))
            .child(field(
                label(&theme, "save-folder", m.folder, None),
                div().h(px(CONTROL_HEIGHT)).child(self.folder_control(window, cx)),
                None,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(label(&theme, "save-scope", m.available_on, None))
                    .child(div().flex().child(segmented(
                        &theme,
                        "save-scope-choice",
                        Some(if any_connection { 1 } else { 0 }),
                        vec![
                            any(segment_face(&theme, None, m.this_connection, !any_connection)
                                .id("save-scope-connection")
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| this.set_any_connection(false, cx)))),
                            any(segment_face(&theme, None, &(m.any_engine_connection)(engine), any_connection)
                                .id("save-scope-engine")
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| this.set_any_connection(true, cx)))),
                        ],
                    )))
                    .child(help(&theme, scope_note, None)),
            )
            .child(field(
                label(&theme, "save-description", format!("{} {}", m.description, m.optional), None),
                text_box(&self.description, false, false, window, cx),
                None,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label(&theme, "save-parameters", m.parameters_found, None).mb(px(0.)))
                    .child(parameters),
            )
            // Room for an error is kept, so one appearing never moves the buttons.
            .child(div().min_h(px(18.)).text_size(px(12.5)).text_color(theme.err).children(self.error.clone()))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        button(&theme, None, m.cancel)
                            .id("save-cancel")
                            .cursor_pointer()
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        enabled(primary_button(&theme, if self.saving { m.saving } else { m.save }), !self.saving)
                            .id("save-confirm")
                            .when(!self.saving, |save| {
                                save.on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))
                            }),
                    ),
            )
    }
}
