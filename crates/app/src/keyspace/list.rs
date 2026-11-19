//! The keys of a database found with `SCAN`, as a `:` tree or a flat list,
//! as the former web UI's `KeyList.tsx`: a pattern (applied with Enter) and a
//! type to narrow them, a page at a time with "Load more", and each key's
//! menu. A new scan keeps the keys shown until its page arrives, and a page
//! of a scan that was replaced is dropped.

use std::collections::HashSet;
use std::ops::Range;

use dbforge_core::drivers::redis::{KeyInfo, ScanPage};
use dbforge_core::keyspace::redis_scan;
use gpui::{
    AnyElement, AppContext, ClipboardItem, Context, DismissEvent, Entity, EventEmitter, FontWeight, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px, uniform_list,
};

use super::format::{format_ttl, kind_label, read_command};
use super::keys::{KINDS, KeyNode, join_page, key_tree, last_segment, visible_key_rows};
use super::kind_badge;
use crate::backend::Backend;
use crate::i18n::{Translate, thousands};
use crate::theme::{ActiveTheme, MONO};
use crate::ui::field::{Tooltip, frame};
use crate::ui::{
    MenuItem, PopoverMenu, Select, SelectEvent, SelectOption, TextInput, TextInputEvent, ghost_button, group, icon,
    icon_button, popover, segment,
};

/// Keys per page.
const PAGE: usize = 500;
/// A row's height: only the rows in sight are drawn.
const ROW: f32 = 28.;
/// How far a nested row moves right per level.
const INDENT: f32 = 14.;

/// What a key's menu opens in its detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Rename,
    Ttl,
    Delete,
}

/// What the list tells the browser.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyListEvent {
    Select(KeyInfo),
    AddKey,
    /// Open the key with one of its dialogs.
    Action(KeyInfo, KeyAction),
}

/// The list shown: the `:` tree or every key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyView {
    Tree,
    List,
}

/// One row on screen.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    Folder { prefix: String, count: usize, depth: usize, expanded: bool },
    Key { key: KeyInfo, depth: usize },
}

pub struct KeyList {
    connection_id: String,
    db: i64,
    pattern_input: Entity<TextInput>,
    kind_select: Entity<Select<String>>,
    /// The pattern and type scanned for.
    pattern: String,
    kind: String,
    view: KeyView,
    keys: Vec<KeyInfo>,
    tree: Vec<KeyNode>,
    rows: Vec<Row>,
    expanded: HashSet<String>,
    /// Where the next page starts; `"0"` when the scan is complete.
    cursor: String,
    loading: bool,
    error: Option<String>,
    /// The newest scan asked for; a page of an older one is dropped.
    scan: u64,
    selected: Option<String>,
    menu: Option<(Entity<PopoverMenu>, Point<Pixels>, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<KeyListEvent> for KeyList {}

impl KeyList {
    pub fn new(connection_id: String, db: i64, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let m = &cx.messages().keyspace;
        let pattern_input = cx.new(|cx| TextInput::new(m.pattern_placeholder, cx));
        let mut kinds = vec![SelectOption::new(String::new(), m.all_types)];
        kinds.extend(KINDS.iter().map(|kind| SelectOption::new((*kind).to_owned(), *kind)));
        let kind_select = cx.new(|cx| Select::new(kinds, Some(String::new()), cx));
        let subscriptions = vec![
            cx.subscribe_in(&pattern_input, window, |this, input, event: &TextInputEvent, _, cx| match event {
                TextInputEvent::Submitted => {
                    let pattern = input.read(cx).text().trim().to_owned();
                    this.set_pattern(pattern, cx);
                }
                TextInputEvent::Changed | TextInputEvent::Cancelled | TextInputEvent::Pasted(_) => {}
            }),
            cx.subscribe(&kind_select, |this, _, SelectEvent(kind): &SelectEvent<String>, cx| {
                this.kind = kind.clone();
                this.reload(cx);
            }),
        ];
        let mut list = Self {
            connection_id,
            db,
            pattern_input,
            kind_select,
            pattern: String::new(),
            kind: String::new(),
            view: KeyView::Tree,
            keys: Vec::new(),
            tree: Vec::new(),
            rows: Vec::new(),
            expanded: HashSet::new(),
            cursor: "0".into(),
            loading: true,
            error: None,
            scan: 0,
            selected: None,
            menu: None,
            _subscriptions: subscriptions,
        };
        list.reload(cx);
        list
    }

    pub fn keys(&self) -> &[KeyInfo] {
        &self.keys
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The key shown selected (the browser's choice).
    pub fn set_selected(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.selected = name;
        cx.notify();
    }

    pub fn set_view(&mut self, view: KeyView, cx: &mut Context<Self>) {
        self.view = view;
        self.rebuild_rows();
        cx.notify();
    }

    /// Scans for `pattern` from the start, the field showing it.
    pub fn set_pattern(&mut self, pattern: String, cx: &mut Context<Self>) {
        if self.pattern_input.read(cx).text() != pattern {
            self.pattern_input.update(cx, |input, cx| input.set_text(&pattern, cx));
        }
        self.pattern = pattern;
        self.reload(cx);
    }

    /// Scans again from the start: keys were added, renamed or deleted.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load("0".into(), false, cx);
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        if !self.loading && self.cursor != "0" {
            self.load(self.cursor.clone(), true, cx);
        }
    }

    fn load(&mut self, from: String, append: bool, cx: &mut Context<Self>) {
        self.scan += 1;
        let scan = self.scan;
        self.loading = true;
        self.error = None;
        let (connection, db, pattern) = (self.connection_id.clone(), self.db, self.pattern.clone());
        let kind = Some(self.kind.clone()).filter(|kind| !kind.is_empty());
        let scanning = Backend::global(cx).spawn(cx, move |core| async move {
            redis_scan(core.connections(), core.statement_log(), connection, db, pattern, kind, from, PAGE).await
        });
        cx.spawn(async move |this, cx| {
            let page = scanning.await;
            let _ = this.update(cx, |this, cx| {
                if this.scan != scan {
                    return;
                }
                this.loading = false;
                match page {
                    Ok(ScanPage { keys, cursor }) => {
                        let current = if append { std::mem::take(&mut this.keys) } else { Vec::new() };
                        this.keys = join_page(current, keys);
                        this.cursor = cursor;
                        this.tree = key_tree(&this.keys);
                        this.rebuild_rows();
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn rebuild_rows(&mut self) {
        self.rows = match self.view {
            KeyView::Tree => visible_key_rows(&self.tree, &|prefix| self.expanded.contains(prefix))
                .into_iter()
                .map(|row| match row.node {
                    KeyNode::Folder { prefix, count, .. } => Row::Folder {
                        prefix: prefix.clone(),
                        count: *count,
                        depth: row.depth,
                        expanded: self.expanded.contains(prefix),
                    },
                    KeyNode::Key(key) => Row::Key { key: key.clone(), depth: row.depth },
                })
                .collect(),
            KeyView::List => self.keys.iter().map(|key| Row::Key { key: key.clone(), depth: 0 }).collect(),
        };
    }

    fn toggle(&mut self, prefix: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(prefix) {
            self.expanded.insert(prefix.to_owned());
        }
        self.rebuild_rows();
        cx.notify();
    }

    fn open_menu(
        &mut self,
        items: Vec<MenuItem>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = cx.new(|cx| PopoverMenu::new(items, window, cx));
        let closing = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, position, closing));
        cx.notify();
    }

    fn folder_menu(&self, prefix: &str, cx: &mut Context<Self>) -> Vec<MenuItem> {
        let m = &cx.messages().keyspace;
        let pattern = format!("{prefix}*");
        let this = cx.entity().downgrade();
        let copied = pattern.clone();
        vec![
            MenuItem::new(m.copy_pattern, move |_, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
            })
            .icon("copy"),
            MenuItem::new(m.show_only_these, move |_, cx| {
                let _ = this.update(cx, |this, cx| this.set_pattern(pattern.clone(), cx));
            })
            .icon("search"),
        ]
    }

    fn key_menu(&self, key: &KeyInfo, cx: &mut Context<Self>) -> Vec<MenuItem> {
        let m = &cx.messages().keyspace;
        let this = cx.entity().downgrade();
        let copy_group = Some(SharedString::from(m.copy_menu));
        let copy = |label: &'static str, text: String| {
            MenuItem::new(label, move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())))
                .group(copy_group.clone())
        };
        let action = |label: String, icon_name: Option<&'static str>, action: KeyAction| {
            let (this, key) = (this.clone(), key.clone());
            let item = MenuItem::new(label, move |_, cx| {
                let _ = this.update(cx, |_, cx| cx.emit(KeyListEvent::Action(key.clone(), action)));
            });
            match icon_name {
                Some(name) => item.icon(name),
                None => item,
            }
        };
        vec![
            copy(m.copy_key_name, key.name.clone()),
            copy(m.copy_read_command, read_command(&key.name, &key.kind)),
            action(format!("{}…", m.rename_key), Some("edit"), KeyAction::Rename),
            action(format!("{}…", m.set_ttl), Some("clock"), KeyAction::Ttl),
            action(format!("{}…", m.delete_key), Some("trash"), KeyAction::Delete),
        ]
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().keyspace;
        let row = div()
            .id(("key-row", index))
            .w_full()
            .h(px(ROW))
            .pr(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(7.))
            .text_size(px(12.5))
            .cursor_pointer();
        match &self.rows[index] {
            Row::Folder { prefix, count, depth, expanded } => {
                let label = if *depth > 0 { last_segment(prefix) } else { prefix.as_str() };
                let (toggled, menu) = (prefix.clone(), prefix.clone());
                row.pl(px(8. + *depth as f32 * INDENT))
                    .text_color(theme.text2)
                    .hover(|row| row.bg(theme.sel))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(&toggled, cx)))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            let items = this.folder_menu(&menu, cx);
                            this.open_menu(items, event.position, window, cx);
                        }),
                    )
                    .child(icon(if *expanded { "down" } else { "right" }, theme.text3, 12.))
                    .child(div().flex_1().min_w_0().truncate().font_family(MONO).child(label.to_owned()))
                    .child(div().flex_none().text_size(px(11.)).text_color(theme.text3).child(thousands(*count as u64)))
                    .into_any_element()
            }
            Row::Key { key, depth } => {
                let selected = self.selected.as_deref() == Some(key.name.as_str());
                let label = if self.view == KeyView::Tree && *depth > 0 { last_segment(&key.name) } else { &key.name };
                let (clicked, menu) = (key.clone(), key.clone());
                row.pl(px(8. + *depth as f32 * INDENT))
                    .text_color(if selected { theme.text } else { theme.text2 })
                    .when(selected, |row| row.bg(theme.accent.opacity(0.2)))
                    .when(!selected, |row| row.hover(|row| row.bg(theme.sel)))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(KeyListEvent::Select(clicked.clone()))))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.emit(KeyListEvent::Select(menu.clone()));
                            let items = this.key_menu(&menu, cx);
                            this.open_menu(items, event.position, window, cx);
                        }),
                    )
                    .child(div().w(px(52.)).flex_none().child(kind_badge(&theme, &key.kind, &kind_label(&key.kind))))
                    .child(div().flex_1().min_w_0().truncate().font_family(MONO).child(label.to_owned()))
                    .child(
                        div()
                            .w(px(70.))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .text_size(px(11.))
                            .text_color(theme.text3)
                            .child(format_ttl(key.ttl_ms, m, false)),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let m = &cx.messages().keyspace;
        let note = |text: String, tint| {
            div().px(px(18.)).py(px(10.)).text_size(px(12.)).text_color(tint).child(text).into_any_element()
        };
        if let Some(error) = &self.error
            && self.keys.is_empty()
        {
            return note(error.clone(), theme.err);
        }
        if !self.loading && self.keys.is_empty() {
            return note(m.no_keys_match.to_owned(), theme.text3);
        }
        uniform_list(
            "key-rows",
            self.rows.len(),
            cx.processor(|this, range: Range<usize>, _window, cx| {
                range.map(|index| this.render_row(index, cx)).collect::<Vec<_>>()
            }),
        )
        .flex_1()
        .px(px(8.))
        .into_any_element()
    }
}

impl Render for KeyList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().keyspace;
        let scanned = if self.cursor == "0" { m.scan_complete.to_owned() } else { (m.scan_cursor)(&self.cursor) };
        let matched = (m.matched)(&thousands(self.keys.len() as u64));
        let focused = self.pattern_input.read(cx).is_focused(window);
        let view_button = |view: KeyView, label: &'static str, id: &'static str| {
            segment(&theme, None, label, self.view == view)
                .h(px(22.))
                .px(px(9.))
                .text_size(px(11.5))
                .id(id)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.set_view(view, cx)))
                .into_any_element()
        };
        let more = (self.cursor != "0").then(|| {
            let label = if self.loading { m.scanning } else { m.load_more };
            ghost_button(&theme, label).id("keys-more").mx(px(8.)).my(px(6.)).when(!self.loading, |more| {
                more.cursor_pointer()
                    .hover(|more| more.bg(theme.sel))
                    .on_click(cx.listener(|this, _, _, cx| this.load_more(cx)))
            })
        });
        div()
            .w(px(380.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(theme.line)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .p(px(10.))
                    .child(
                        frame(&theme, focused, false, false)
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .font_family(MONO)
                            .text_size(px(12.5))
                            .text_color(theme.text)
                            .child(icon("search", theme.text3, 14.))
                            .child(div().flex_1().min_w_0().child(self.pattern_input.clone())),
                    )
                    .child(div().w(px(138.)).flex_none().child(self.kind_select.clone()))
                    .child(
                        icon_button(&theme, "add-key", "plus", true, false)
                            .tooltip(Tooltip::text(m.add_key))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(KeyListEvent::AddKey))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .pb(px(6.))
                    .text_size(px(11.5))
                    .text_color(theme.text3)
                    .child(div().flex_1().min_w_0().truncate().child(format!("{scanned} · {matched}")))
                    .child(
                        group(
                            &theme,
                            vec![
                                view_button(KeyView::Tree, m.tree, "keys-tree"),
                                view_button(KeyView::List, m.list, "keys-list"),
                            ],
                        )
                        .h(px(28.))
                        .rounded(px(14.)),
                    ),
            )
            .when_some(self.error.clone().filter(|_| !self.keys.is_empty()), |list, error| {
                list.child(div().px(px(18.)).pb(px(6.)).text_size(px(12.)).text_color(theme.err).child(error))
            })
            .child(div().flex_1().min_h_0().flex().flex_col().child(self.render_rows(cx)))
            .children(more.map(|more| more.font_weight(FontWeight::MEDIUM)))
            .children(self.menu.as_ref().map(|(menu, position, _)| popover(menu.clone(), *position)))
    }
}
