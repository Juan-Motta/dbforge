//! A small form in a dialog, as the former web UI's `FormDialog.tsx`: adding a
//! key or an element, setting a TTL, renaming or deleting a key. It stays
//! open and shows the error when its action fails, and closes
//! ([`DismissEvent`]) when it succeeds. Enter submits from a one-line field
//! (⌘↵ / Ctrl+Enter from anywhere); Escape closes.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    App, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, Window, actions, div, prelude::FluentBuilder, px,
};

use super::format::{FieldName, FormValues};
use crate::i18n::Translate;
use crate::sql::Syntax;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::field::{Tooltip, frame, label, text_box};
use crate::ui::{
    CodeEditor, Select, SelectEvent, SelectOption, TextInput, TextInputEvent, button, enabled, glass, icon,
    primary_button,
};

actions!(keyspace_dialog, [Submit, Dismiss]);

const CONTEXT: &str = "KeyspaceDialog";

/// A text area's rows, as the former web UI's `rows={5}`.
const AREA_ROWS: f32 = 5.;

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("enter", Submit, Some(CONTEXT)),
        KeyBinding::new("secondary-enter", Submit, Some(CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(CONTEXT)),
    ]
}

/// How a field is typed into.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Text,
    /// Several lines; Enter starts a new one.
    Area,
    /// A number, typed as text and read by the action.
    Number,
    /// One of these `(value, label)` choices.
    Choice(Vec<(String, SharedString)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormField {
    pub name: FieldName,
    pub label: SharedString,
    pub kind: FieldKind,
    pub placeholder: Option<&'static str>,
    pub initial: String,
}

impl FormField {
    pub fn new(name: FieldName, label: impl Into<SharedString>, kind: FieldKind) -> Self {
        Self { name, label: label.into(), kind, placeholder: None, initial: String::new() }
    }

    pub fn placeholder(mut self, placeholder: &'static str) -> Self {
        self.placeholder = Some(placeholder);
        self
    }

    pub fn initial(mut self, initial: impl Into<String>) -> Self {
        self.initial = initial.into();
        self
    }
}

/// What the form holds, by field.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Values(pub HashMap<FieldName, String>);

impl FormValues for Values {
    fn get(&self, field: FieldName) -> String {
        self.0.get(&field).cloned().unwrap_or_default()
    }
}

/// An action of the dialog: refused at once with a message, or a call
/// whose failure the dialog shows.
pub type Action = Rc<dyn Fn(&Values, &mut App) -> Result<Task<Result<(), String>>, String>>;

/// The text above the fields, a deletion's question: `before` **name** `after`.
pub struct Intro {
    pub before: &'static str,
    pub name: String,
    pub after: String,
}

enum Control {
    Text(Entity<TextInput>),
    Area(Entity<CodeEditor>),
    Choice(Entity<Select<String>>),
}

pub struct FormDialog {
    title: SharedString,
    submit_label: SharedString,
    danger: bool,
    intro: Option<Intro>,
    fields: Vec<(FormField, Control)>,
    /// The fields shown for the value of the `Kind` field; all when `None`.
    layout: Option<fn(&str) -> Vec<FieldName>>,
    submit: Action,
    secondary: Option<(SharedString, Action)>,
    error: Option<String>,
    busy: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for FormDialog {}

impl Focusable for FormDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FormDialog {
    pub fn new(
        title: impl Into<SharedString>,
        submit_label: impl Into<SharedString>,
        fields: Vec<FormField>,
        submit: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        let controls = fields
            .into_iter()
            .map(|field| {
                let control = match &field.kind {
                    FieldKind::Text | FieldKind::Number => {
                        let input = cx.new(|cx| {
                            let mut input = TextInput::new(field.placeholder.unwrap_or_default(), cx);
                            input.set_text(&field.initial, cx);
                            input
                        });
                        subscriptions.push(cx.subscribe_in(
                            &input,
                            window,
                            |this, _, event: &TextInputEvent, _, cx| match event {
                                TextInputEvent::Submitted => this.run_submit(cx),
                                TextInputEvent::Cancelled => cx.emit(DismissEvent),
                                TextInputEvent::Changed | TextInputEvent::Pasted(_) => {}
                            },
                        ));
                        Control::Text(input)
                    }
                    FieldKind::Area => Control::Area(cx.new(|cx| CodeEditor::new(&field.initial, Syntax::Plain, cx))),
                    FieldKind::Choice(options) => {
                        let options = options
                            .iter()
                            .map(|(value, label)| SelectOption::new(value.clone(), label.clone()))
                            .collect();
                        let select = cx.new(|cx| Select::new(options, Some(field.initial.clone()), cx));
                        subscriptions.push(cx.subscribe(&select, |_, _, _: &SelectEvent<String>, cx| cx.notify()));
                        Control::Choice(select)
                    }
                };
                (field, control)
            })
            .collect();
        Self {
            title: title.into(),
            submit_label: submit_label.into(),
            danger: false,
            intro: None,
            fields: controls,
            layout: None,
            submit,
            secondary: None,
            error: None,
            busy: false,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The main action deletes: drawn in the error colour.
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn intro(mut self, intro: Intro) -> Self {
        self.intro = Some(intro);
        self
    }

    /// Shows only the fields `layout` lists for the `Kind` field's value.
    pub fn layout(mut self, layout: fn(&str) -> Vec<FieldName>) -> Self {
        self.layout = Some(layout);
        self
    }

    /// An extra action left of Cancel, such as "Remove TTL".
    pub fn secondary(mut self, label: impl Into<SharedString>, action: Action) -> Self {
        self.secondary = Some((label.into(), action));
        self
    }

    /// The first field takes the keyboard; with none, the dialog does.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        match self.shown(cx).first().map(|(_, control)| control) {
            Some(Control::Text(input)) => input.read(cx).focus(window),
            Some(Control::Area(editor)) => editor.read(cx).focus(window),
            Some(Control::Choice(select)) => select.read(cx).focus(window),
            None => window.focus(&self.focus),
        }
    }

    pub fn values(&self, cx: &App) -> Values {
        Values(
            self.fields
                .iter()
                .map(|(field, control)| {
                    let value = match control {
                        Control::Text(input) => input.read(cx).text().to_owned(),
                        Control::Area(editor) => editor.read(cx).text().to_owned(),
                        Control::Choice(select) => select.read(cx).value().cloned().unwrap_or_default(),
                    };
                    (field.name, value)
                })
                .collect(),
        )
    }

    /// The error shown, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The fields on screen now, in order.
    pub fn shown_fields(&self, cx: &App) -> Vec<FieldName> {
        self.shown(cx).into_iter().map(|(field, _)| field.name).collect()
    }

    /// Types `text` into `field` (a choice's value), as the user would.
    pub fn fill(&self, field: FieldName, text: &str, cx: &mut App) {
        let Some((_, control)) = self.fields.iter().find(|(shown, _)| shown.name == field) else { return };
        match control {
            Control::Text(input) => input.update(cx, |input, cx| input.set_text(text, cx)),
            Control::Area(editor) => editor.update(cx, |editor, cx| editor.set_text(text, cx)),
            Control::Choice(select) => select.update(cx, |select, cx| select.set_value(Some(text.to_owned()), cx)),
        }
    }

    fn shown(&self, cx: &App) -> Vec<&(FormField, Control)> {
        let wanted = self.layout.map(|layout| layout(&self.values(cx).get(FieldName::Kind)));
        self.fields
            .iter()
            .filter(|(field, _)| wanted.as_ref().is_none_or(|wanted| wanted.contains(&field.name)))
            .collect()
    }

    pub fn run_submit(&mut self, cx: &mut Context<Self>) {
        let action = self.submit.clone();
        self.run(action, cx);
    }

    /// The extra action, such as "Remove TTL".
    pub fn run_secondary(&mut self, cx: &mut Context<Self>) {
        if let Some((_, action)) = self.secondary.clone() {
            self.run(action, cx);
        }
    }

    fn run(&mut self, action: Action, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let values = self.values(cx);
        let task = match action(&values, cx) {
            Ok(task) => task,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.busy = true;
        self.error = None;
        cx.spawn(async move |this, cx| {
            let outcome = task.await;
            let _ = this.update(cx, |this, cx| match outcome {
                Ok(()) => cx.emit(DismissEvent),
                Err(error) => {
                    this.busy = false;
                    this.error = Some(error);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn render_field(&self, index: usize, field: &FormField, control: &Control, window: &Window, cx: &App) -> gpui::Div {
        let theme = cx.theme();
        let label = label(theme, ("keyspace-field", index), field.label.clone(), None);
        let control = match control {
            Control::Text(input) => text_box(input, false, false, window, cx).into_any_element(),
            Control::Area(editor) => frame(theme, editor.read(cx).is_focused(window), false, false)
                .h(px(AREA_ROWS * crate::ui::code_editor::LINE_HEIGHT + 20.))
                .p_0()
                .overflow_hidden()
                .font_family(MONO)
                .child(editor.clone())
                .into_any_element(),
            Control::Choice(select) => select.clone().into_any_element(),
        };
        crate::ui::field::field(label, control, None)
    }
}

impl Render for FormDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().keyspace;
        let busy = self.busy;
        let fields: Vec<gpui::Div> = self
            .shown(cx)
            .into_iter()
            .enumerate()
            .map(|(index, (field, control))| self.render_field(index, field, control, window, cx))
            .collect();
        let intro = self.intro.as_ref().map(|intro| {
            div()
                .flex()
                .flex_wrap()
                .items_baseline()
                .gap_x(px(4.))
                .text_size(px(13.))
                .line_height(px(20.))
                .text_color(theme.text2)
                .child(intro.before)
                .child(
                    div()
                        .font_family(MONO)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.text)
                        .child(intro.name.clone()),
                )
                .child(intro.after.trim_start().to_owned())
        });
        let main = if self.danger {
            button(&theme, None, &self.submit_label).text_color(theme.err)
        } else {
            primary_button(&theme, &self.submit_label)
        };
        glass(&theme)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Submit, _, cx| this.run_submit(cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(DismissEvent)))
            .bg(theme.dialog)
            .w(px(440.))
            .max_w_full()
            .rounded(px(22.))
            .p(px(22.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div().flex_1().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(self.title.clone()),
                    )
                    .child(
                        div()
                            .id("keyspace-dialog-close")
                            .size(px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(13.))
                            .cursor_pointer()
                            .hover(|close| close.bg(theme.sel))
                            .tooltip(Tooltip::text(m.close))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
                            .child(icon("close", theme.text2, 13.)),
                    ),
            )
            .children(intro)
            .children(fields)
            // Room for an error is kept, so one appearing never moves the buttons.
            .child(div().min_h(px(20.)).text_size(px(12.5)).text_color(theme.err).children(self.error.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when_some(self.secondary.clone(), |footer, (label, action)| {
                        footer.child(enabled(button(&theme, None, &label), !busy).id("keyspace-dialog-secondary").when(
                            !busy,
                            |secondary| {
                                secondary.on_click(cx.listener(move |this, _, _, cx| this.run(action.clone(), cx)))
                            },
                        ))
                    })
                    .child(div().flex_1())
                    .child(
                        button(&theme, None, m.cancel)
                            .id("keyspace-dialog-cancel")
                            .cursor_pointer()
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        enabled(main, !busy)
                            .id("keyspace-dialog-submit")
                            .when(!busy, |submit| submit.on_click(cx.listener(|this, _, _, cx| this.run_submit(cx)))),
                    ),
            )
    }
}
