//! Running a saved query with `:parameters`: one row per parameter with its
//! type (Text, Number, Date, Date and time, Boolean, NULL) and value,
//! starting from the values it last ran with; the SQL that will run, the
//! values written into it as literals of the connection's dialect and
//! highlighted; why a value can't be used, which keeps Run off; and
//! "Remember values". Each value field lists the last values its parameter
//! ran with, newest first, to pick one again. Enter or ⌘↵ runs; Escape closes.

use std::collections::BTreeMap;

use dbforge_core::saved_queries::params::{ParamError, Substituted, parameters, substitute};
use dbforge_core::saved_queries::{ParamKind, ParamValue, SavedQuery};
use gpui::{
    App, AppContext, ClickEvent, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    HighlightStyle, InteractiveElement, IntoElement, KeyBinding, ParentElement, Pixels, Point, Render,
    StatefulInteractiveElement, Styled, StyledText, Subscription, Window, actions, div, prelude::FluentBuilder, px,
};

use super::rules::{ago, default_kind, param_error, param_of};
use crate::i18n::{SavedMessages, Translate};
use crate::query::shortcuts;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::field::{Tooltip, help, text_box};
use crate::ui::modal::{modal, modal_body, modal_footer, note};
use crate::ui::{
    Checkbox, Ellipsis, MenuItem, PopoverMenu, Select, SelectEvent, SelectOption, TextInput, TextInputEvent, button,
    enabled, icon_button, popover, primary_button,
};

actions!(run_saved_query, [Confirm, Dismiss]);

const CONTEXT: &str = "RunSavedQuery";

/// ⌘↵ (Ctrl+Enter) and Enter on a closed type list run; Escape closes.
pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    vec![
        KeyBinding::new("secondary-enter", Confirm, context),
        KeyBinding::new("enter", Confirm, context),
        KeyBinding::new("escape", Dismiss, context),
    ]
}

/// What the values are asked for: to run the query, or to explain it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunAction {
    #[default]
    Run,
    Explain,
}

/// A saved query to run (or explain), and where.
#[derive(Debug, Clone, PartialEq)]
pub struct RunRequest {
    pub query: SavedQuery,
    /// The SQL whose parameters are asked for: the query's, or its console's text.
    pub sql: String,
    pub connection_id: String,
    pub connection_name: String,
    /// The tab of the console that asked, when one did.
    pub console: Option<String>,
    /// Run it, or explain it (Explain in its console).
    pub action: RunAction,
}

/// What the dialog tells its owner, besides closing ([`DismissEvent`]).
#[derive(Debug, Clone, PartialEq)]
pub enum RunDialogEvent {
    /// Run `sql` (the values written into it); remember `values` when `remember`.
    Run { request: RunRequest, sql: String, values: BTreeMap<String, ParamValue>, remember: bool },
}

/// The types a value can be written as, in the order the list shows them.
const KINDS: [ParamKind; 6] =
    [ParamKind::Text, ParamKind::Number, ParamKind::Date, ParamKind::DateTime, ParamKind::Boolean, ParamKind::Null];

fn kind_label(kind: ParamKind, m: &SavedMessages) -> &'static str {
    match kind {
        ParamKind::Text => m.kind_text,
        ParamKind::Number => m.kind_number,
        ParamKind::Date => m.kind_date,
        ParamKind::DateTime => m.kind_date_time,
        ParamKind::Boolean => m.kind_boolean,
        ParamKind::Null => m.kind_null,
    }
}

/// What a value field shows while empty, for its type.
fn placeholder(kind: ParamKind, m: &SavedMessages) -> &'static str {
    match kind {
        ParamKind::Date => m.date_placeholder,
        ParamKind::DateTime => m.date_time_placeholder,
        ParamKind::Boolean => m.boolean_placeholder,
        ParamKind::Number => "0",
        ParamKind::Text | ParamKind::Null => "",
    }
}

/// How a remembered value is listed: NULL, an empty value, or the value.
fn shown_value<'a>(value: &'a ParamValue, m: &SavedMessages) -> &'a str {
    match value.kind {
        ParamKind::Null => "NULL",
        _ if value.value.is_empty() => m.empty_value,
        _ => value.value.as_str(),
    }
}

/// The size of the recent-values button, and of the room kept for it.
const RECENT_BUTTON: f32 = 22.;

/// One parameter's row.
struct Row {
    name: String,
    kind: Entity<Select<ParamKind>>,
    value: Entity<TextInput>,
    /// "Last used …", when it ran with a value for it before.
    last_used: Option<String>,
    /// The last values it ran with, newest first.
    recent: Vec<ParamValue>,
}

pub struct RunDialog {
    request: RunRequest,
    rows: Vec<Row>,
    remember: bool,
    /// A parameter's recent values, listed.
    recent_menu: Option<(Entity<PopoverMenu>, Point<Pixels>, Subscription)>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for RunDialog {}
impl EventEmitter<RunDialogEvent> for RunDialog {}

impl Focusable for RunDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl RunDialog {
    pub fn new(request: RunRequest, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let m = &cx.messages().saved;
        let now = chrono::Utc::now().timestamp_millis();
        let mut subscriptions = Vec::new();
        let rows: Vec<Row> = parameters(&request.sql, request.query.engine)
            .into_iter()
            .map(|name| {
                let remembered = request.query.last_values.get(&name);
                let kind = default_kind(remembered);
                let options = KINDS.iter().map(|&kind| SelectOption::new(kind, kind_label(kind, m))).collect();
                let select = cx.new(|cx| Select::new(options, Some(kind), cx));
                let value = cx.new(|cx| {
                    let mut input = TextInput::new(placeholder(kind, m), cx);
                    if let Some(remembered) = remembered {
                        input.set_text(&remembered.value, cx);
                    }
                    input
                });
                if kind == ParamKind::Null {
                    value.update(cx, |input, cx| input.set_disabled(true, window, cx));
                }
                subscriptions.push(cx.subscribe_in(&select, window, Self::on_kind));
                subscriptions.push(cx.subscribe_in(&value, window, Self::on_value));
                let last_used = remembered.zip(request.query.last_run_at).map(|(value, then)| {
                    let shown = if value.kind == ParamKind::Null { "NULL" } else { value.value.as_str() };
                    (m.last_used)(shown, &ago(then, now, m))
                });
                let recent = request.query.recent_values.get(&name).cloned().unwrap_or_default();
                Row { name, kind: select, value, last_used, recent }
            })
            .collect();
        Self {
            request,
            rows,
            remember: true,
            recent_menu: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The first value takes the keyboard.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        match self.rows.iter().find(|row| !row.value.read(cx).is_disabled()) {
            Some(row) => row.value.read(cx).focus(window),
            None => window.focus(&self.focus),
        }
    }

    pub fn request(&self) -> &RunRequest {
        &self.request
    }

    /// Sets a parameter's type and value, as the user would.
    pub fn set_value(&mut self, name: &str, kind: ParamKind, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|row| row.name == name) else { return };
        let (select, input) = (row.kind.clone(), row.value.clone());
        select.update(cx, |select, cx| select.set_value(Some(kind), cx));
        input.update(cx, |input, cx| input.set_text(value, cx));
        self.kind_changed(&select, kind, window, cx);
    }

    /// The last values parameter `name` ran with, newest first.
    pub fn recent_values(&self, name: &str) -> &[ParamValue] {
        self.rows.iter().find(|row| row.name == name).map_or(&[], |row| row.recent.as_slice())
    }

    /// The recent-values menu, while open.
    pub fn recent_menu(&self) -> Option<&Entity<PopoverMenu>> {
        self.recent_menu.as_ref().map(|(menu, _, _)| menu)
    }

    /// Lists parameter `name`'s recent values at `position`, each with its
    /// type when it isn't the row's; picking one fills the row with it.
    pub fn open_recent(&mut self, name: &str, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|row| row.name == name) else { return };
        if row.recent.is_empty() {
            return;
        }
        let m = &cx.messages().saved;
        let current = row.kind.read(cx).value().copied().unwrap_or(ParamKind::Text);
        let typed = row.value.read(cx).text().to_owned();
        let this = cx.entity().downgrade();
        let items = row
            .recent
            .iter()
            .map(|value| {
                let (this, name, picked) = (this.clone(), name.to_owned(), value.clone());
                let same = value.kind == current && (value.kind == ParamKind::Null || value.value == typed);
                MenuItem::new(shown_value(value, m).to_owned(), move |window, cx| {
                    let _ = this.update(cx, |this, cx| this.pick_recent(&name, &picked, window, cx));
                })
                .checked(same)
                .detail((value.kind != current).then_some(kind_label(value.kind, m)))
            })
            .collect();
        let menu = cx.new(|cx| PopoverMenu::new(items, window, cx));
        let closing = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.recent_menu = None;
            cx.notify();
        });
        self.recent_menu = Some((menu, position, closing));
        cx.notify();
    }

    /// Fills parameter `name` with a recent `value` and gives its field the keyboard back.
    fn pick_recent(&mut self, name: &str, value: &ParamValue, window: &mut Window, cx: &mut Context<Self>) {
        self.set_value(name, value.kind, &value.value, window, cx);
        if let Some(row) = self.rows.iter().find(|row| row.name == name) {
            if row.value.read(cx).is_disabled() {
                window.focus(&self.focus);
            } else {
                row.value.read(cx).focus(window);
            }
        }
    }

    /// The button at a value field's right edge listing its recent values;
    /// with none, its room is kept so the field doesn't change width.
    fn recent_button(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> gpui::AnyElement {
        let slot = div().size(px(RECENT_BUTTON)).flex_none();
        if row.recent.is_empty() {
            return slot.into_any_element();
        }
        let theme = cx.theme();
        let m = &cx.messages().saved;
        let name = row.name.clone();
        slot.child(
            icon_button(theme, ("run-saved-recent", index), "down", true, self.recent_menu.is_some())
                .size(px(RECENT_BUTTON))
                .tooltip(Tooltip::text(m.recent_values))
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    if this.recent_menu.is_none() {
                        let at = event.position() + gpui::point(px(0.), px(12.));
                        this.open_recent(&name, at, window, cx);
                    }
                })),
        )
        .into_any_element()
    }

    pub fn set_remember(&mut self, remember: bool, cx: &mut Context<Self>) {
        self.remember = remember;
        cx.notify();
    }

    fn on_kind(
        &mut self,
        select: &Entity<Select<ParamKind>>,
        SelectEvent(kind): &SelectEvent<ParamKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.kind_changed(select, *kind, window, cx);
    }

    /// A NULL takes no value; each type says what it expects while empty.
    fn kind_changed(
        &mut self,
        select: &Entity<Select<ParamKind>>,
        kind: ParamKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let m = &cx.messages().saved;
        if let Some(row) = self.rows.iter().find(|row| row.kind == *select) {
            row.value.update(cx, |input, cx| {
                input.set_placeholder(placeholder(kind, m), cx);
                input.set_disabled(kind == ParamKind::Null, window, cx);
            });
        }
        cx.notify();
    }

    fn on_value(&mut self, _: &Entity<TextInput>, event: &TextInputEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Changed => cx.notify(),
            TextInputEvent::Submitted => self.run(cx),
            TextInputEvent::Cancelled => cx.emit(DismissEvent),
            TextInputEvent::Pasted(_) => {}
        }
    }

    /// The values typed, by parameter.
    fn values(&self, cx: &App) -> BTreeMap<String, ParamValue> {
        self.rows
            .iter()
            .map(|row| {
                let kind = row.kind.read(cx).value().copied().unwrap_or(ParamKind::Text);
                let value = if kind == ParamKind::Null { String::new() } else { row.value.read(cx).text().to_owned() };
                (row.name.clone(), ParamValue { kind, value })
            })
            .collect()
    }

    /// The SQL that would run with the values typed, or why it can't.
    pub fn outcome(&self, cx: &App) -> Result<Substituted, ParamError> {
        substitute(&self.request.sql, self.request.query.engine, &self.values(cx))
    }

    pub fn can_run(&self, cx: &App) -> bool {
        self.outcome(cx).is_ok()
    }

    /// Runs with the values typed, when they can be used.
    pub fn run(&mut self, cx: &mut Context<Self>) {
        let Ok(substituted) = self.outcome(cx) else { return };
        cx.emit(RunDialogEvent::Run {
            request: self.request.clone(),
            sql: substituted.sql,
            values: self.values(cx),
            remember: self.remember,
        });
        cx.emit(DismissEvent);
    }

    fn preview(&self, outcome: &Result<Substituted, ParamError>, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let text = match outcome {
            Ok(substituted) => {
                let style = HighlightStyle {
                    color: Some(theme.accent_text),
                    background_color: Some(theme.accent.opacity(0.18)),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..Default::default()
                };
                StyledText::new(substituted.sql.clone())
                    .with_highlights(substituted.spans.iter().map(|span| (span.clone(), style)))
            }
            // The SQL as written while a value can't be used.
            Err(_) => StyledText::new(self.request.sql.clone()),
        };
        div()
            .id("run-preview")
            .h(px(150.))
            .overflow_y_scroll()
            .p(px(12.))
            .rounded(px(12.))
            .border_1()
            .border_color(theme.line)
            .bg(theme.sel)
            .font_family(MONO)
            .text_size(px(12.))
            .line_height(px(18.))
            .text_color(theme.text)
            .child(text)
    }
}

/// The widths of the parameter table's first two columns.
const NAME_WIDTH: f32 = 130.;
const KIND_WIDTH: f32 = 150.;

impl Render for RunDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().saved;
        let outcome = self.outcome(cx);
        let invalid = outcome.as_ref().err().and_then(param_of).map(str::to_owned);
        let ready = outcome.is_ok();
        let explain = self.request.action == RunAction::Explain;
        let query = &self.request.query;
        let mut subtitle: Vec<String> = query.folder.iter().cloned().collect();
        subtitle.push(self.request.connection_name.clone());
        subtitle.push((m.parameters)(self.rows.len()));
        let column = |text: &'static str| {
            div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(theme.text3).child(text)
        };
        let header = div()
            .flex()
            .gap(px(10.))
            .child(column(m.parameter).w(px(NAME_WIDTH)).flex_none())
            .child(column(m.kind).w(px(KIND_WIDTH)).flex_none())
            .child(column(m.value).flex_1());
        let rows: Vec<gpui::AnyElement> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let bad = invalid.as_deref() == Some(row.name.as_str());
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(
                        div()
                            .w(px(NAME_WIDTH))
                            .flex_none()
                            .h(px(32.))
                            .flex()
                            .items_center()
                            .font_family(MONO)
                            .text_size(px(12.5))
                            .text_color(theme.accent_text)
                            .child(div().min_w_0().ellipsis().child(format!(":{}", row.name))),
                    )
                    .child(div().w(px(KIND_WIDTH)).flex_none().child(row.kind.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                text_box(&row.value, bad, true, window, cx)
                                    .pr(px(5.))
                                    .child(self.recent_button(index, row, cx)),
                            )
                            .child(help(&theme, row.last_used.clone().unwrap_or_default(), None).ellipsis()),
                    )
                    .into_any_element()
            })
            .collect();
        let error = outcome.as_ref().err().map(|error| param_error(error, m));
        modal(&theme, 640., (m.run_title)(&query.name))
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.run(cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(DismissEvent)))
            .child(div().px(px(22.)).text_size(px(12.5)).text_color(theme.text3).ellipsis().child(subtitle.join(" · ")))
            .child(
                modal_body("run-saved-body")
                    .child(div().flex().flex_col().gap(px(8.)).child(header).children(rows))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(column(m.preview))
                            .child(self.preview(&outcome, cx))
                            .child(note(&theme, m.preview_note))
                            // Room for why a value can't be used is kept, so nothing moves.
                            .child(div().min_h(px(18.)).text_size(px(12.5)).text_color(theme.err).children(error)),
                    ),
            )
            .child(
                modal_footer(&theme)
                    .child(
                        div().flex_1().child(
                            Checkbox::new("remember-values", m.remember_values, self.remember).on_change(
                                cx.listener(|this, remember: &bool, _, cx| this.set_remember(*remember, cx)),
                            ),
                        ),
                    )
                    .child(
                        button(&theme, None, m.cancel)
                            .id("run-saved-cancel")
                            .cursor_pointer()
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        enabled(primary_button(&theme, if explain { m.explain } else { m.run }), ready)
                            .id("run-saved-run")
                            .when(!explain, |run| {
                                run.child(div().font_weight(FontWeight::MEDIUM).opacity(0.8).child(shortcuts::RUN))
                            })
                            .when(ready, |run| run.on_click(cx.listener(|this, _, _, cx| this.run(cx)))),
                    ),
            )
            .children(self.recent_menu.as_ref().map(|(menu, position, _)| popover(menu.clone(), *position)))
    }
}
