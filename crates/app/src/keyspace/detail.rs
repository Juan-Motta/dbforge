//! One key, as the former web UI's `KeyDetail.tsx`: its type, TTL and size, its
//! value a page at a time (fields, items, members or entries, filtered by
//! the server where the type allows), the picked element in a viewer that
//! shows it as text, JSON, hex or MessagePack and edits it, and the key's
//! dialogs (TTL, rename, delete, add an element). A read keeps the value
//! shown until it arrives; a reply to an older read is dropped.

use std::rc::Rc;

use dbforge_core::drivers::redis::{Entry, KeyEdit, KeyValue, Text};
use dbforge_core::keyspace::{redis_edit_key, redis_read_key};
use gpui::{
    AnyElement, App, AppContext, ClipboardItem, Context, DismissEvent, Entity, EventEmitter, FontWeight,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, WeakEntity, Window, div, prelude::FluentBuilder, px,
    relative,
};

use super::dialog::{Action, FieldKind, FormDialog, FormField, Intro, Values};
use super::format::{
    FieldName, Shape, ValueMode, add_edit, add_fields, as_json, editable, first_mode, format_bytes, format_ttl,
    kind_label, remove_edit, save_edit, shape, shown_value, ttl_ms, ttl_seconds,
};
use super::list::KeyAction;
use super::msgpack::as_msgpack;
use super::{badge, kind_badge};
use crate::backend::Backend;
use crate::i18n::{KeyspaceMessages, Translate, thousands};
use crate::sql::Syntax;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::code_editor::LINE_HEIGHT;
use crate::ui::field::{Tooltip, frame, text_box};
use crate::ui::{
    CodeEditor, MenuItem, PopoverMenu, TextInput, TextInputEvent, button, enabled, ghost_button, group, icon,
    icon_button, popover, primary_button, segment,
};

/// Elements per page of a value.
const PAGE: usize = 200;
/// The tallest a value is shown before it scrolls.
const VALUE_MAX: f32 = 320.;

/// What the detail asks of the browser.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyDetailEvent {
    /// Keys were added, renamed or removed: the list scans again, and this
    /// key is selected (none after a deletion).
    KeysChanged(Option<String>),
    /// Show one of the key's dialogs (see [`KeyDetail::dialog`]).
    OpenDialog(DialogKind),
}

/// The picked element's viewer: how it shows it, and its edit in course.
struct Viewer {
    /// Which element it shows; a different one starts over.
    identity: String,
    mode: ValueMode,
    json: Option<String>,
    msgpack: Option<String>,
    code: Entity<CodeEditor>,
    draft: Option<Draft>,
    busy: bool,
    error: Option<String>,
}

/// A value or a score being edited.
enum Draft {
    Value(Entity<CodeEditor>),
    Score { input: Entity<TextInput>, _events: Subscription },
}

pub struct KeyDetail {
    connection_id: String,
    db: i64,
    key: Text,
    value: Option<KeyValue>,
    error: Option<String>,
    filter: Entity<TextInput>,
    loading_more: bool,
    picked: usize,
    /// The newest read asked for; an older one arriving late is dropped.
    read: u64,
    /// A dialog the key list's menu asked for, opened once the key is read.
    pending: Option<KeyAction>,
    viewer: Option<Viewer>,
    menu: Option<(Entity<PopoverMenu>, Point<Pixels>, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<KeyDetailEvent> for KeyDetail {}

impl KeyDetail {
    pub fn new(
        connection_id: String,
        db: i64,
        key: Text,
        pending: Option<KeyAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| TextInput::new("", cx));
        let subscriptions = vec![cx.subscribe_in(&filter, window, |this, _, event: &TextInputEvent, _, cx| {
            if matches!(event, TextInputEvent::Changed | TextInputEvent::Submitted) {
                this.load(cx);
            }
        })];
        let mut detail = Self {
            connection_id,
            db,
            key,
            value: None,
            error: None,
            filter,
            loading_more: false,
            picked: 0,
            read: 0,
            pending,
            viewer: None,
            menu: None,
            _subscriptions: subscriptions,
        };
        detail.load(cx);
        detail
    }

    pub fn key(&self) -> &Text {
        &self.key
    }

    pub fn value(&self) -> Option<&KeyValue> {
        self.value.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Reads the key's first page again (the CLI may have changed it).
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.read += 1;
        let read = self.read;
        let filter = self.filter.read(cx).text().to_owned();
        let reading = self.read_page(None, filter, cx);
        cx.spawn(async move |this, cx| {
            let page = reading.await;
            let _ = this.update(cx, |this, cx| {
                if this.read != read {
                    return;
                }
                match page {
                    Ok(value) => {
                        this.picked = this.picked.min(value.entries.len().saturating_sub(1));
                        this.value = Some(value);
                        this.error = None;
                        this.loading_more = false;
                        this.sync_viewer(cx);
                        this.sync_filter_placeholder(cx);
                        this.open_pending(cx);
                    }
                    Err(error) => {
                        this.error = Some(error);
                        this.value = None;
                        this.viewer = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn read_page(&self, cursor: Option<String>, filter: String, cx: &App) -> Task<Result<KeyValue, String>> {
        let (connection, db, key) = (self.connection_id.clone(), self.db, self.key.clone());
        Backend::global(cx).spawn(cx, move |core| async move {
            redis_read_key(core.connections(), core.statement_log(), connection, db, key, cursor, filter, PAGE)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        let Some(next) = self.value.as_ref().and_then(|value| value.next.clone()) else { return };
        if self.loading_more {
            return;
        }
        self.loading_more = true;
        let read = self.read;
        let filter = self.filter.read(cx).text().to_owned();
        let reading = self.read_page(Some(next), filter, cx);
        cx.spawn(async move |this, cx| {
            let page = reading.await;
            let _ = this.update(cx, |this, cx| {
                if this.read != read {
                    return;
                }
                this.loading_more = false;
                match page {
                    Ok(more) => {
                        if let Some(value) = this.value.as_mut() {
                            let mut entries = std::mem::take(&mut value.entries);
                            entries.extend(more.entries.iter().cloned());
                            *value = KeyValue { entries, ..more };
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Changes the key while it still has the type shown.
    fn edit_task(&self, change: KeyEdit, cx: &App) -> Task<Result<(), String>> {
        let kind = self.value.as_ref().map(|value| value.kind.clone()).unwrap_or_default();
        let (connection, db, key) = (self.connection_id.clone(), self.db, self.key.clone());
        Backend::global(cx).spawn(cx, move |core| async move {
            redis_edit_key(core.connections(), core.statement_log(), connection, db, key, kind, change)
                .await
                .map_err(|error| error.to_string())
        })
    }

    /// An edit, then the key read again once it succeeds.
    fn edit_then_reload(this: WeakEntity<Self>, change: KeyEdit, cx: &mut App) -> Task<Result<(), String>> {
        let Some(editing) = this.upgrade().map(|detail| detail.read(cx).edit_task(change, cx)) else {
            return Task::ready(Ok(()));
        };
        cx.spawn(async move |cx| {
            editing.await?;
            let _ = this.update(cx, |this, cx| this.load(cx));
            Ok(())
        })
    }

    /// An edit that changes which keys exist, then `selected` chosen.
    fn edit_then_list(
        this: WeakEntity<Self>,
        change: KeyEdit,
        selected: Option<String>,
        cx: &mut App,
    ) -> Task<Result<(), String>> {
        let Some(editing) = this.upgrade().map(|detail| detail.read(cx).edit_task(change, cx)) else {
            return Task::ready(Ok(()));
        };
        cx.spawn(async move |cx| {
            editing.await?;
            let _ = this.update(cx, |_, cx| cx.emit(KeyDetailEvent::KeysChanged(selected)));
            Ok(())
        })
    }

    /// Runs an edit from the viewer, its error shown there.
    fn act(&mut self, change: Result<KeyEdit, String>, cx: &mut Context<Self>) {
        let Some(viewer) = self.viewer.as_mut() else { return };
        if viewer.busy {
            return;
        }
        let change = match change {
            Ok(change) => change,
            Err(error) => {
                viewer.error = Some(error);
                cx.notify();
                return;
            }
        };
        viewer.busy = true;
        viewer.error = None;
        let editing = Self::edit_then_reload(cx.entity().downgrade(), change, cx);
        cx.spawn(async move |this, cx| {
            let outcome = editing.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(viewer) = this.viewer.as_mut() {
                    viewer.busy = false;
                    match outcome {
                        Ok(()) => viewer.draft = None,
                        Err(error) => viewer.error = Some(error),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn picked_entry(&self) -> Option<&Entry> {
        self.value.as_ref()?.entries.get(self.picked)
    }

    fn pick(&mut self, index: usize, cx: &mut Context<Self>) {
        self.picked = index;
        self.sync_viewer(cx);
        cx.notify();
    }

    /// The viewer for the picked element: a new one for another element,
    /// the same one (its mode and edit kept) with the element's text again.
    fn sync_viewer(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.picked_entry().cloned() else {
            self.viewer = None;
            return;
        };
        let identity = format!(
            "{}:{}:{}",
            self.key.text,
            self.picked,
            entry.key.as_ref().map_or(&entry.value.text, |key| &key.text)
        );
        let json = (!entry.value.binary).then(|| as_json(&entry.value.text)).flatten();
        let msgpack = as_msgpack(&entry.value);
        let same = self.viewer.as_ref().is_some_and(|viewer| viewer.identity == identity);
        if same && let Some(viewer) = self.viewer.as_mut() {
            viewer.json = json;
            viewer.msgpack = msgpack;
        } else {
            let mode = first_mode(entry.value.binary, json.is_some(), msgpack.is_some());
            let code = cx.new(|cx| {
                let mut editor = CodeEditor::new("", Syntax::Plain, cx);
                editor.set_read_only(true, cx);
                editor
            });
            self.viewer = Some(Viewer { identity, mode, json, msgpack, code, draft: None, busy: false, error: None });
        }
        self.show_value(cx);
    }

    /// The viewer's text for its mode.
    fn show_value(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.picked_entry().cloned() else { return };
        let Some(viewer) = self.viewer.as_ref() else { return };
        let text = shown_value(&entry.value, viewer.mode, viewer.json.as_deref(), viewer.msgpack.as_deref());
        let syntax = match viewer.mode {
            ValueMode::Json | ValueMode::Msgpack if !text.is_empty() => Syntax::Json,
            _ => Syntax::Plain,
        };
        viewer.code.update(cx, |code, cx| {
            if code.syntax() != syntax {
                *code = CodeEditor::new("", syntax, cx);
                code.set_read_only(true, cx);
            }
            code.show_text(&text, cx);
        });
    }

    fn set_mode(&mut self, mode: ValueMode, cx: &mut Context<Self>) {
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.mode = mode;
        }
        self.show_value(cx);
        cx.notify();
    }

    fn start_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(value) = self.value.as_ref() else { return };
        let score = value.kind == "zset";
        let Some(entry) = self.picked_entry().cloned() else { return };
        let Some(viewer) = self.viewer.as_mut() else { return };
        viewer.error = None;
        viewer.draft = Some(if score {
            let input = cx.new(|cx| {
                let mut input = TextInput::new("", cx);
                input.set_text(&entry.score.map(js_score).unwrap_or_default(), cx);
                input
            });
            input.read(cx).focus(window);
            let events = cx.subscribe_in(&input, window, |this, input, event: &TextInputEvent, _, cx| match event {
                TextInputEvent::Submitted => {
                    let text = input.read(cx).text().to_owned();
                    this.save(text, cx);
                }
                TextInputEvent::Cancelled => this.cancel_edit(cx),
                TextInputEvent::Changed | TextInputEvent::Pasted(_) => {}
            });
            Draft::Score { input, _events: events }
        } else {
            let editor = cx.new(|cx| CodeEditor::new(&entry.value.text, Syntax::Plain, cx));
            editor.read(cx).focus(window);
            Draft::Value(editor)
        });
        cx.notify();
    }

    fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.draft = None;
            viewer.error = None;
        }
        cx.notify();
    }

    fn save_draft(&mut self, cx: &mut Context<Self>) {
        let text = match self.viewer.as_ref().and_then(|viewer| viewer.draft.as_ref()) {
            Some(Draft::Value(editor)) => editor.read(cx).text().to_owned(),
            Some(Draft::Score { input, .. }) => input.read(cx).text().to_owned(),
            None => return,
        };
        self.save(text, cx);
    }

    fn save(&mut self, text: String, cx: &mut Context<Self>) {
        let (Some(value), Some(entry)) = (self.value.as_ref(), self.picked_entry()) else { return };
        let change = save_edit(&value.kind, entry, &text, &cx.messages().keyspace);
        self.act(change, cx);
    }

    fn remove(&mut self, entry: Entry, cx: &mut Context<Self>) {
        let Some(kind) = self.value.as_ref().map(|value| value.kind.clone()) else { return };
        if self.viewer.is_none() {
            return;
        }
        self.act(Ok(remove_edit(&kind, &entry)), cx);
    }

    // Dialogs

    fn open_pending(&mut self, cx: &mut Context<Self>) {
        if let Some(action) = self.pending.take() {
            self.request_dialog(DialogKind::from(action), cx);
        }
    }

    /// Asks the browser to show one of the key's dialogs.
    pub fn request_dialog(&mut self, kind: DialogKind, cx: &mut Context<Self>) {
        cx.emit(KeyDetailEvent::OpenDialog(kind));
    }

    /// Builds one of the key's dialogs; the browser shows it.
    pub fn dialog(&self, kind: DialogKind, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<FormDialog>> {
        let value = self.value.as_ref()?;
        let m = &cx.messages().keyspace;
        let this = cx.entity().downgrade();
        let dialog = match kind {
            DialogKind::Ttl => {
                let fields = vec![
                    FormField::new(FieldName::Ttl, m.expires_in, FieldKind::Number).initial(ttl_seconds(value.ttl_ms)),
                ];
                let set: Action = {
                    let this = this.clone();
                    Rc::new(move |values: &Values, cx: &mut App| {
                        let ms = ttl_ms(
                            &values.0.get(&FieldName::Ttl).cloned().unwrap_or_default(),
                            &cx.messages().keyspace,
                        )?;
                        Ok(Self::edit_then_reload(this.clone(), KeyEdit::Expire { ttl_ms: Some(ms) }, cx))
                    })
                };
                let has_ttl = value.ttl_ms.is_some();
                cx.new(|cx| {
                    let dialog = FormDialog::new(m.set_ttl, m.set_ttl, fields, set, window, cx);
                    if has_ttl {
                        let remove: Action = Rc::new(move |_: &Values, cx: &mut App| {
                            Ok(Self::edit_then_reload(this.clone(), KeyEdit::Expire { ttl_ms: None }, cx))
                        });
                        dialog.secondary(m.remove_ttl, remove)
                    } else {
                        dialog
                    }
                })
            }
            DialogKind::Rename => {
                let fields = vec![FormField::new(FieldName::Name, m.new_name, FieldKind::Text).initial(&self.key.text)];
                let rename: Action = Rc::new(move |values: &Values, cx: &mut App| {
                    let to = values.0.get(&FieldName::Name).cloned().unwrap_or_default();
                    Ok(Self::edit_then_list(this.clone(), KeyEdit::Rename { to: to.clone() }, Some(to), cx))
                });
                cx.new(|cx| FormDialog::new(m.rename_key, m.rename, fields, rename, window, cx))
            }
            DialogKind::Delete => {
                let noun = shape(&value.kind, m).map_or(m.noun_elements, |shape| shape.noun);
                let intro = Intro {
                    before: m.delete_before,
                    name: self.key.text.clone(),
                    after: (m.delete_after)(&thousands(value.length.max(0) as u64), noun),
                };
                let delete: Action = Rc::new(move |_: &Values, cx: &mut App| {
                    Ok(Self::edit_then_list(this.clone(), KeyEdit::Delete, None, cx))
                });
                cx.new(|cx| {
                    FormDialog::new(m.delete_key, m.delete, Vec::new(), delete, window, cx).danger().intro(intro)
                })
            }
            DialogKind::Add => {
                let shape = shape(&value.kind, m)?;
                let title = shape.add?;
                let kind = value.kind.clone();
                let fields = add_fields(&kind).iter().map(|name| add_field(*name, m)).collect();
                let add: Action = Rc::new(move |values: &Values, cx: &mut App| {
                    let change = add_edit(&kind, values, &cx.messages().keyspace)?;
                    Ok(Self::edit_then_reload(this.clone(), change, cx))
                });
                cx.new(|cx| FormDialog::new(title, m.add, fields, add, window, cx))
            }
        };
        Some(dialog)
    }

    fn open_entry_menu(&mut self, index: usize, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(value) = self.value.as_ref() else { return };
        let m = &cx.messages().keyspace;
        let Some(shape) = shape(&value.kind, m) else { return };
        let Some(entry) = value.entries.get(index).cloned() else { return };
        let zset = value.kind == "zset";
        self.pick(index, cx);
        let copy = |label: &str, text: String| {
            MenuItem::new((m.copy_field_of)(label), move |_, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
            })
        };
        let mut items = Vec::new();
        if let (Some(key), Some(label)) = (&entry.key, shape.key) {
            items.push(copy(label, key.text.clone()));
        }
        items.push(copy(shape.value, entry.value.text.clone()));
        if zset && let Some(score) = entry.score {
            items.push(copy(m.score, js_score(score)));
        }
        let this = cx.entity().downgrade();
        items.push(
            MenuItem::new(m.remove, move |_, cx| {
                let entry = entry.clone();
                let _ = this.update(cx, |this, cx| this.remove(entry, cx));
            })
            .icon("trash"),
        );
        let menu = cx.new(|cx| PopoverMenu::new(items, window, cx));
        let closing = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, position, closing));
        cx.notify();
    }
}

/// One of a key's dialogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    Ttl,
    Rename,
    Delete,
    Add,
}

impl From<KeyAction> for DialogKind {
    fn from(action: KeyAction) -> Self {
        match action {
            KeyAction::Rename => DialogKind::Rename,
            KeyAction::Ttl => DialogKind::Ttl,
            KeyAction::Delete => DialogKind::Delete,
        }
    }
}

/// A field of the dialog adding an element, as the former web UI's `addFields`.
fn add_field(name: FieldName, m: &KeyspaceMessages) -> FormField {
    match name {
        FieldName::Field => FormField::new(name, m.field, FieldKind::Text),
        FieldName::Value => FormField::new(name, m.value, FieldKind::Area),
        FieldName::Member => FormField::new(name, m.member, FieldKind::Text),
        FieldName::Score => FormField::new(name, m.score, FieldKind::Number),
        FieldName::End => FormField::new(
            name,
            m.add_at,
            FieldKind::Choice(vec![("tail".into(), m.the_end.into()), ("head".into(), m.the_start.into())]),
        )
        .initial("tail"),
        FieldName::Fields => FormField::new(name, m.fields_per_line, FieldKind::Area).placeholder("event=login"),
        FieldName::Name | FieldName::Kind | FieldName::Ttl => FormField::new(name, "", FieldKind::Text),
    }
}

/// A score as JavaScript's `String()` writes it: `4`, not `4.0`.
fn js_score(score: f64) -> String {
    if score.fract() == 0. && score.abs() < 1e21 { format!("{score:.0}") } else { score.to_string() }
}

// Drawing

/// The widths of the elements' table: `#`, the size, and a sorted set's score.
const NUMBER_COLUMN: f32 = 44.;
const SIZE_COLUMN: f32 = 80.;
const SCORE_COLUMN: f32 = 90.;

impl KeyDetail {
    fn render_header(&self, value: &KeyValue, shape: Option<Shape>, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().keyspace;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .py(px(12.))
            .border_b_1()
            .border_color(theme.line)
            .child(kind_badge(&theme, &value.kind, &kind_label(&value.kind)).h(px(22.)).px(px(8.)).text_size(px(11.)))
            .child(
                div()
                    .min_w_0()
                    .flex_shrink()
                    .truncate()
                    .font_family(MONO)
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(value.name.clone()),
            )
            .child(
                icon_button(&theme, "rename-key", "edit", true, false)
                    .tooltip(Tooltip::text(m.rename_key))
                    .on_click(cx.listener(|this, _, _, cx| this.request_dialog(DialogKind::Rename, cx))),
            )
            .child(div().flex_1())
            .when_some(value.ttl_ms, |header, ttl| {
                header.child(
                    badge(&theme, Some(theme.warn))
                        .child(icon("clock", theme.warn, 12.))
                        .child(format!("TTL {}", format_ttl(Some(ttl), m, true))),
                )
            })
            .when_some(shape, |header, shape| {
                header.child(badge(&theme, None).child(format!(
                    "{} {}",
                    thousands(value.length.max(0) as u64),
                    shape.noun
                )))
            })
            .when_some(value.bytes, |header, bytes| header.child(badge(&theme, None).child(format_bytes(bytes))))
            .when_some(value.encoding.clone(), |header, encoding| header.child(badge(&theme, None).child(encoding)))
            .child(
                button(&theme, None, m.set_ttl)
                    .id("set-ttl")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.request_dialog(DialogKind::Ttl, cx))),
            )
            .child(
                button(&theme, None, m.delete)
                    .text_color(theme.err)
                    .id("delete-key")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.request_dialog(DialogKind::Delete, cx))),
            )
            .into_any_element()
    }

    fn render_entries(&self, value: &KeyValue, shape: Shape, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().keyspace;
        let zset = value.kind == "zset";
        let cell = |text: String| div().min_w_0().truncate().child(text);
        let columns = |row: gpui::Div,
                       number: AnyElement,
                       key: Option<AnyElement>,
                       item: AnyElement,
                       score: Option<AnyElement>,
                       size: AnyElement| {
            row.flex()
                .items_center()
                .gap(px(12.))
                .px(px(12.))
                .child(div().w(px(NUMBER_COLUMN)).flex_none().child(number))
                .when_some(key, |row, key| row.child(div().w(relative(0.3)).min_w(px(120.)).flex_none().child(key)))
                .child(div().flex_1().min_w(px(120.)).child(item))
                .when_some(score, |row, score| row.child(div().w(px(SCORE_COLUMN)).flex_none().child(score)))
                .child(div().w(px(SIZE_COLUMN)).flex_none().flex().justify_end().child(size))
        };
        let head = columns(
            div()
                .h(px(30.))
                .bg(theme.sel.opacity(0.5))
                .text_color(theme.text3)
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM),
            "#".into_any_element(),
            shape.key.map(|key| key.into_any_element()),
            shape.value.into_any_element(),
            zset.then(|| m.score.into_any_element()),
            m.size.into_any_element(),
        );
        let rows = value.entries.iter().enumerate().map(|(index, entry)| {
            let picked = index == self.picked;
            columns(
                div().h(px(30.)).border_t_1().border_color(theme.line).text_size(px(12.5)).font_family(MONO),
                div().text_color(theme.text3).child((index + 1).to_string()).into_any_element(),
                shape.key.map(|_| {
                    cell(entry.key.as_ref().map(|key| key.text.clone()).unwrap_or_default())
                        .text_color(theme.kw)
                        .into_any_element()
                }),
                cell(entry.value.text.clone()).text_color(theme.text).into_any_element(),
                zset.then(|| {
                    div().text_color(theme.num).child(entry.score.map(js_score).unwrap_or_default()).into_any_element()
                }),
                div().text_color(theme.text3).child(format_bytes(entry.size as i64)).into_any_element(),
            )
            .id(("entry", index))
            .cursor_pointer()
            .when(picked, |row| row.bg(theme.accent.opacity(0.2)))
            .when(!picked, |row| row.hover(|row| row.bg(theme.sel)))
            .on_click(cx.listener(move |this, _, _, cx| this.pick(index, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.open_entry_menu(index, event.position, window, cx)
                }),
            )
        });
        let rows: Vec<_> = rows.collect();
        let empty = value.entries.is_empty().then(|| {
            div()
                .border_t_1()
                .border_color(theme.line)
                .px(px(12.))
                .py(px(10.))
                .text_size(px(12.5))
                .text_color(theme.text3)
                .child(m.nothing_matches)
        });
        div()
            .flex()
            .flex_col()
            .flex_none()
            .rounded(px(10.))
            .border_1()
            .border_color(theme.line)
            .overflow_hidden()
            .child(head)
            .children(rows)
            .children(empty)
            .into_any_element()
    }

    fn render_viewer(
        &self,
        value: &KeyValue,
        entry: &Entry,
        label: String,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let viewer = self.viewer.as_ref()?;
        let theme = *cx.theme();
        let m = &cx.messages().keyspace;
        let binary = entry.value.binary;
        let score = value.kind == "zset";
        let can_edit = editable(&value.kind, value.length, entry);
        let removable = value.kind != "string";
        let mode_button = |mode: ValueMode, label: &'static str, on: bool, id: &'static str| {
            let shown =
                segment(&theme, None, label, viewer.mode == mode).h(px(22.)).px(px(9.)).text_size(px(11.5)).id(id);
            if on {
                shown
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, cx)))
                    .into_any_element()
            } else {
                shown.opacity(0.4).into_any_element()
            }
        };
        let modes = group(
            &theme,
            vec![
                mode_button(ValueMode::Text, m.text, !binary, "mode-text"),
                mode_button(ValueMode::Json, "JSON", viewer.json.is_some(), "mode-json"),
                mode_button(ValueMode::Hex, "Hex", true, "mode-hex"),
                mode_button(ValueMode::Msgpack, "MsgPack", viewer.msgpack.is_some(), "mode-msgpack"),
            ],
        )
        .h(px(28.))
        .rounded(px(14.))
        .id("value-modes")
        .when(!binary, |modes| modes.tooltip(Tooltip::text(m.msgpack_binary)));
        let editing = viewer.draft.is_some();
        let busy = viewer.busy;
        let bar = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .border_b_1()
            .border_color(theme.line)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.5))
                    .text_color(theme.text3)
                    .font_weight(FontWeight::MEDIUM)
                    .child(label),
            )
            .child(modes)
            .when(can_edit && !editing, |bar| {
                bar.child(
                    button(&theme, None, if score { m.edit_score } else { m.edit })
                        .id("value-edit")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| this.start_edit(window, cx))),
                )
            })
            .when(removable && !editing, |bar| {
                let entry = entry.clone();
                bar.child(
                    enabled(button(&theme, None, m.remove).text_color(theme.err), !busy)
                        .id("value-remove")
                        .when(!busy, |remove| {
                            remove.on_click(cx.listener(move |this, _, _, cx| this.remove(entry.clone(), cx)))
                        }),
                )
            });
        let lines = viewer.code.read(cx).text().lines().count().max(1) as f32;
        let height = (lines * LINE_HEIGHT + 20.).min(VALUE_MAX);
        let body = match &viewer.draft {
            None => div().h(px(height)).flex().child(viewer.code.clone()).into_any_element(),
            Some(draft) => {
                let control = match draft {
                    Draft::Value(editor) => frame(&theme, editor.read(cx).is_focused(window), false, false)
                        .h(px(6. * LINE_HEIGHT + 20.))
                        .p_0()
                        .overflow_hidden()
                        .font_family(MONO)
                        .child(editor.clone())
                        .into_any_element(),
                    Draft::Score { input, .. } => text_box(input, false, true, window, cx).into_any_element(),
                };
                div()
                    .p(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(control)
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                button(&theme, None, m.cancel)
                                    .id("value-cancel")
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_edit(cx))),
                            )
                            .child(
                                enabled(primary_button(&theme, m.save), !busy).id("value-save").when(!busy, |save| {
                                    save.on_click(cx.listener(|this, _, _, cx| this.save_draft(cx)))
                                }),
                            ),
                    )
                    .into_any_element()
            }
        };
        Some(
            div()
                .mt(px(18.))
                .flex()
                .flex_col()
                .flex_none()
                .rounded(px(10.))
                .border_1()
                .border_color(theme.line)
                .overflow_hidden()
                .child(bar)
                .child(body)
                .when_some(viewer.error.clone(), |viewer, error| {
                    viewer.child(div().px(px(12.)).py(px(8.)).text_size(px(12.5)).text_color(theme.err).child(error))
                })
                .into_any_element(),
        )
    }
}

impl Render for KeyDetail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().keyspace;
        let note = |text: String, tint| {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .px(px(24.))
                .text_size(px(13.))
                .text_color(tint)
                .child(text)
        };
        if let Some(error) = &self.error
            && self.value.is_none()
        {
            return note(error.clone(), theme.err).into_any_element();
        }
        let Some(value) = self.value.clone() else {
            return note((m.loading_key)(&self.key.text), theme.text3).into_any_element();
        };
        let shape = shape(&value.kind, m);
        let header = self.render_header(&value, shape, cx);
        let entry = self.picked_entry().cloned();
        let body = match shape {
            None => note((m.module_keys)(&value.kind), theme.text3).into_any_element(),
            Some(_) if value.kind == "string" => {
                let viewer = entry.as_ref().and_then(|entry| {
                    let label = (m.string_value)(&format_bytes(value.length), value.length > entry.size as i64);
                    self.render_viewer(&value, entry, label, window, cx)
                });
                div()
                    .id("key-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(16.))
                    .pb(px(16.))
                    .flex()
                    .flex_col()
                    .children(viewer)
                    .into_any_element()
            }
            Some(shape) => {
                let filter_focused = self.filter.read(cx).is_focused(window);
                let bar = div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .py(px(8.))
                    .when(shape.filter, |bar| {
                        bar.child(
                            frame(&theme, filter_focused, false, false)
                                .w(px(260.))
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_color(theme.text)
                                .child(icon("search", theme.text3, 14.))
                                .child(div().flex_1().min_w_0().child(self.filter.clone())),
                        )
                    })
                    .child(div().flex_1())
                    .when_some(shape.add, |bar, add| {
                        bar.child(
                            button(&theme, Some("plus"), add)
                                .id("add-element")
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| this.request_dialog(DialogKind::Add, cx))),
                        )
                    });
                let entries = self.render_entries(&value, shape, cx);
                let more = value.next.as_ref().map(|_| {
                    let label = if self.loading_more {
                        m.loading.to_owned()
                    } else {
                        (m.load_more_of)(value.entries.len(), value.length)
                    };
                    ghost_button(&theme, &label).id("entries-more").mt(px(6.)).when(!self.loading_more, |more| {
                        more.cursor_pointer()
                            .hover(|more| more.bg(theme.sel))
                            .on_click(cx.listener(|this, _, _, cx| this.load_more(cx)))
                    })
                });
                let viewer = entry.as_ref().and_then(|entry| {
                    let name = entry.key.as_ref().map_or(&entry.value.text, |key| &key.text).clone();
                    self.render_viewer(&value, entry, (m.entry_value)(&name), window, cx)
                });
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(bar)
                    .child(
                        div()
                            .id("key-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px(px(16.))
                            .pb(px(16.))
                            .flex()
                            .flex_col()
                            .child(entries)
                            .children(more)
                            .children(viewer),
                    )
                    .into_any_element()
            }
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .children(self.menu.as_ref().map(|(menu, position, _)| popover(menu.clone(), *position)))
            .into_any_element()
    }
}

/// Where a label for the filter comes from: the type's elements.
pub(super) fn filter_placeholder(shape: Shape, m: &KeyspaceMessages) -> SharedString {
    (m.filter_noun)(shape.noun).into()
}

impl KeyDetail {
    /// The filter's placeholder follows the type once the key is read.
    pub(super) fn sync_filter_placeholder(&self, cx: &mut Context<Self>) {
        let Some(shape) = self.value.as_ref().and_then(|value| shape(&value.kind, &cx.messages().keyspace)) else {
            return;
        };
        let placeholder = filter_placeholder(shape, &cx.messages().keyspace);
        self.filter.update(cx, |filter, cx| filter.set_placeholder(placeholder, cx));
    }
}
