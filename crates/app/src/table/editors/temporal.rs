//! A date or time being edited: its text, typed as before, and under it the
//! parts as controls (day and month dropdowns, a year field, hour : minute :
//! second fields), kept in step. The cell's editor and the inspector both
//! use it, each with its own buttons around it.

use gpui::{
    AppContext, Context, Entity, EventEmitter, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::i18n::Translate;
use crate::table::temporal::{Temporal, TemporalDraft, TemporalKind, days_in_month};
use crate::theme::{ActiveTheme, MONO};
use crate::ui::field::{Tooltip, text_box};
use crate::ui::{Select, SelectEvent, SelectOption, TextInput, TextInputEvent};

/// What the fields tell their owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemporalEvent {
    /// The text or a part changed.
    Changed,
    /// Enter in the text or a part.
    Submitted,
    /// Escape.
    Cancelled,
}

/// A part shown as a two-digit (or four-digit) field: which, and its range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Year,
    Hour,
    Minute,
    Second,
}

impl Part {
    fn of(self, parts: &Temporal) -> u32 {
        match self {
            Part::Year => parts.year,
            Part::Hour => parts.hour,
            Part::Minute => parts.minute,
            Part::Second => parts.second,
        }
    }

    fn text(self, parts: &Temporal) -> String {
        match self {
            Part::Year => self.of(parts).to_string(),
            _ => format!("{:02}", self.of(parts)),
        }
    }

    /// Sets the part from typed digits: a year within 1–9999, a time part
    /// wrapping past its end (60 minutes is 00). `false` when it isn't a number.
    fn set(self, parts: &mut Temporal, typed: &str) -> bool {
        let Ok(number) = typed.trim().parse::<u32>() else { return false };
        let wrap = |max: u32| number % (max + 1);
        match self {
            Part::Year if (1..=9999).contains(&number) => parts.year = number,
            Part::Year => return false,
            Part::Hour => parts.hour = wrap(23),
            Part::Minute => parts.minute = wrap(59),
            Part::Second => parts.second = wrap(59),
        }
        true
    }
}

pub struct TemporalFields {
    draft: TemporalDraft,
    text: Entity<TextInput>,
    day: Entity<Select<u32>>,
    month: Entity<Select<u32>>,
    parts: Vec<(Part, Entity<TextInput>)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TemporalEvent> for TemporalFields {}

impl TemporalFields {
    pub fn new(
        initial: Option<&str>,
        kind: TemporalKind,
        offset_space: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = TemporalDraft::new(initial, kind, offset_space);
        let months = cx.messages().table.months;
        let text = cx.new(|cx| {
            let mut input = TextInput::new(if initial.is_none() { "NULL" } else { "" }, cx);
            input.set_text(&draft.text, cx);
            input.select_all(cx);
            input
        });
        let day = cx.new(|cx| Select::new(day_options(&draft.parts), Some(draft.parts.day), cx));
        let month_options =
            months.iter().enumerate().map(|(index, name)| SelectOption::new(index as u32 + 1, *name)).collect();
        let month = cx.new(|cx| Select::new(month_options, Some(draft.parts.month), cx));
        let shown: &[Part] = match kind {
            TemporalKind::Date => &[Part::Year],
            TemporalKind::Time => &[Part::Hour, Part::Minute, Part::Second],
            _ => &[Part::Year, Part::Hour, Part::Minute, Part::Second],
        };
        let parts: Vec<(Part, Entity<TextInput>)> = shown
            .iter()
            .map(|part| {
                let input = cx.new(|cx| {
                    let mut input = TextInput::new("", cx);
                    input.set_text(&part.text(&draft.parts), cx);
                    input
                });
                (*part, input)
            })
            .collect();
        let mut subscriptions = vec![
            cx.subscribe_in(&text, window, |this, input, event: &TextInputEvent, window, cx| match event {
                TextInputEvent::Changed => {
                    let typed = input.read(cx).text().to_owned();
                    this.draft.type_text(&typed);
                    this.sync(window, cx);
                    cx.emit(TemporalEvent::Changed);
                }
                other => this.forward(other, cx),
            }),
            cx.subscribe_in(&day, window, |this, _, SelectEvent(day): &SelectEvent<u32>, window, cx| {
                let day = *day;
                this.change(|parts| parts.day = day, window, cx);
            }),
            cx.subscribe_in(&month, window, |this, _, SelectEvent(month): &SelectEvent<u32>, window, cx| {
                let month = *month;
                this.change(|parts| parts.month = month, window, cx);
            }),
        ];
        for (part, input) in &parts {
            let part = *part;
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |this, input, event: &TextInputEvent, window, cx| match event {
                    TextInputEvent::Changed => {
                        let typed = input.read(cx).text().to_owned();
                        let mut parts = this.draft.parts.clone();
                        if part.set(&mut parts, &typed) {
                            this.change(|current| *current = parts, window, cx);
                        }
                    }
                    other => this.forward(other, cx),
                },
            ));
        }
        let fields = Self { draft, text, day, month, parts, _subscriptions: subscriptions };
        fields.text.read(cx).focus(window);
        fields
    }

    pub fn draft(&self) -> &TemporalDraft {
        &self.draft
    }

    /// A dropdown of the fields is showing its list.
    pub fn is_choosing(&self, cx: &gpui::App) -> bool {
        self.day.read(cx).is_open() || self.month.read(cx).is_open()
    }

    pub fn focus(&self, window: &mut Window, cx: &gpui::App) {
        self.text.read(cx).focus(window);
    }

    /// This computer's clock (Now, Today).
    pub fn now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.draft.now();
        self.sync(window, cx);
        cx.emit(TemporalEvent::Changed);
    }

    /// Back to `text` (Revert).
    pub fn reset(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.draft.type_text(text);
        self.sync(window, cx);
        cx.emit(TemporalEvent::Changed);
    }

    fn forward(&mut self, event: &TextInputEvent, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Submitted => cx.emit(TemporalEvent::Submitted),
            TextInputEvent::Cancelled => cx.emit(TemporalEvent::Cancelled),
            _ => {}
        }
    }

    fn change(&mut self, change: impl FnOnce(&mut Temporal), window: &mut Window, cx: &mut Context<Self>) {
        self.draft.change(change);
        self.sync(window, cx);
        cx.emit(TemporalEvent::Changed);
    }

    /// Shows the draft in every control but the one being typed in.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parts = self.draft.parts.clone();
        if !self.text.read(cx).is_focused(window) {
            let text = self.draft.text.clone();
            self.text.update(cx, |input, cx| input.set_text(&text, cx));
        }
        self.day.update(cx, |day, cx| {
            day.set_options(day_options(&parts), cx);
            day.set_value(Some(parts.day), cx);
        });
        self.month.update(cx, |month, cx| month.set_value(Some(parts.month), cx));
        for (part, input) in &self.parts {
            if !input.read(cx).is_focused(window) {
                let text = part.text(&parts);
                input.update(cx, |input, cx| input.set_text(&text, cx));
            }
        }
        cx.notify();
    }
}

fn day_options(parts: &Temporal) -> Vec<SelectOption<u32>> {
    (1..=days_in_month(parts.year, parts.month)).map(|day| SelectOption::new(day, day.to_string())).collect()
}

/// `+05:30` as the controls show an offset, with a true minus sign.
fn offset_text(offset: i32) -> String {
    let sign = if offset < 0 { '−' } else { '+' };
    format!("UTC{sign}{:02}:{:02}", offset.abs() / 60, offset.abs() % 60)
}

impl Render for TemporalFields {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().table;
        let kind = self.draft.kind;
        let part = |part: Part, width: f32, window: &Window, cx: &Context<Self>| {
            self.parts
                .iter()
                .find(|(shown, _)| *shown == part)
                .map(|(_, input)| text_box(input, false, true, window, cx).w(px(width)).flex_none())
        };
        let row = || div().flex().items_center().gap(px(6.));
        let mut fields = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(text_box(&self.text, !self.draft.is_valid(), true, window, cx).w_full());
        if kind != TemporalKind::Time {
            fields = fields.child(
                row()
                    .child(div().w(px(64.)).flex_none().child(self.day.clone()))
                    .child(div().flex_1().min_w_0().child(self.month.clone()))
                    .children(part(Part::Year, 72., window, cx)),
            );
        }
        if kind != TemporalKind::Date {
            let colon = || div().text_color(theme.text3).child(":");
            let mut time = row()
                .children(part(Part::Hour, 46., window, cx))
                .child(colon())
                .children(part(Part::Minute, 46., window, cx))
                .child(colon())
                .children(part(Part::Second, 46., window, cx));
            let note = |text: String| div().text_size(px(11.5)).font_family(MONO).text_color(theme.text3).child(text);
            if !self.draft.parts.fraction.is_empty() {
                time = time.child(note(format!(".{}", self.draft.parts.fraction)));
            }
            if kind == TemporalKind::TimestampTz {
                time = time.child(
                    div()
                        .id("temporal-offset")
                        .tooltip(Tooltip::text(m.offset_kept))
                        .child(note(offset_text(self.draft.parts.offset.unwrap_or(0)))),
                );
            }
            fields = fields.child(time);
        }
        fields
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinners_wrap_time_parts_and_keep_years_in_range() {
        let mut parts = TemporalDraft::new(Some("2026-01-01 10:00:00"), TemporalKind::Timestamp, false).parts;
        assert!(Part::Minute.set(&mut parts, "60"));
        assert_eq!(parts.minute, 0, "60 minutes is 00");
        assert!(Part::Hour.set(&mut parts, "25"));
        assert_eq!(parts.hour, 1);
        assert!(!Part::Year.set(&mut parts, "0"));
        assert!(!Part::Year.set(&mut parts, "x"));
        assert!(Part::Year.set(&mut parts, "2028"));
        assert_eq!((parts.year, Part::Second.text(&parts)), (2028, "00".to_owned()));
        assert_eq!(offset_text(-180), "UTC−03:00");
    }
}
