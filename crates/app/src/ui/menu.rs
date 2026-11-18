//! A small popover menu: a list of commands at a point (the pointer on a
//! right-click, a button's corner, under a dropdown). It paints above
//! everything, takes the keyboard (↑ ↓ Enter), and closes on Escape, a click
//! outside it or a choice, telling its owner with a [`DismissEvent`]. Items
//! may sit under group headings, carry a colour swatch and show a check.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, FontWeight, Hsla, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Pixels, Point, Render, SharedString, StatefulInteractiveElement, Styled,
    Window, actions, anchored, deferred, div, prelude::FluentBuilder, px,
};

use super::{dot, glass, icon};
use crate::theme::ActiveTheme;

actions!(popover_menu, [SelectPrevious, SelectNext, Confirm, Dismiss]);

const CONTEXT: &str = "PopoverMenu";

/// Space kept between the menu and the window's edges.
const MARGIN: f32 = 8.;

pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    vec![
        KeyBinding::new("up", SelectPrevious, context),
        KeyBinding::new("down", SelectNext, context),
        KeyBinding::new("enter", Confirm, context),
        KeyBinding::new("escape", Dismiss, context),
    ]
}

/// What a menu item does when chosen; the menu is closed by then.
pub type MenuHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// A coloured dot before an item's label: an engine's colour, a tag's, or
/// an empty ring for none.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Swatch {
    Color(Hsla),
    Empty,
}

/// One command in a [`PopoverMenu`].
#[derive(Clone)]
pub struct MenuItem {
    pub label: SharedString,
    pub icon: Option<&'static str>,
    pub swatch: Option<Swatch>,
    /// The heading it is listed under; a heading is drawn where it changes.
    pub group: Option<SharedString>,
    /// The current choice, as a dropdown marks it.
    pub checked: bool,
    /// Shown but not choosable now (Revert on a cell with no change).
    pub disabled: bool,
    /// A faint note after the label: the value a choice is written as.
    pub detail: Option<SharedString>,
    pub handler: MenuHandler,
}

impl MenuItem {
    pub fn new(label: impl Into<SharedString>, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            label: label.into(),
            icon: None,
            swatch: None,
            group: None,
            checked: false,
            disabled: false,
            detail: None,
            handler: Rc::new(handler),
        }
    }

    /// An icon from [`crate::assets`] before the label.
    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }

    pub fn swatch(mut self, swatch: Option<Swatch>) -> Self {
        self.swatch = swatch;
        self
    }

    pub fn group(mut self, group: Option<SharedString>) -> Self {
        self.group = group;
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn detail(mut self, detail: Option<impl Into<SharedString>>) -> Self {
        self.detail = detail.map(Into::into);
        self
    }
}

/// A [`Swatch`] as drawn: a dot, or a dashed-looking ring for none.
pub fn swatch(swatch: Swatch, ring: Hsla) -> impl IntoElement {
    match swatch {
        Swatch::Color(tint) => dot(tint),
        Swatch::Empty => div().size(px(8.)).rounded(px(4.)).flex_none().border_1().border_color(ring),
    }
}

pub struct PopoverMenu {
    items: Vec<MenuItem>,
    /// The item the keyboard is on.
    highlighted: Option<usize>,
    /// As wide as the control it drops from, when it does.
    width: Option<Pixels>,
    focus: FocusHandle,
}

impl EventEmitter<DismissEvent> for PopoverMenu {}

impl Focusable for PopoverMenu {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PopoverMenu {
    /// A menu of `items` that takes the keyboard at once.
    /// The checked item starts highlighted, else the first.
    pub fn new(items: Vec<MenuItem>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let highlighted =
            items.iter().position(|item| item.checked).or_else(|| items.iter().position(|item| !item.disabled));
        Self { highlighted, items, width: None, focus }
    }

    /// Draws the menu `width` wide, as the dropdown it opens from.
    pub fn with_width(mut self, width: Pixels) -> Self {
        self.width = Some(width);
        self
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.items.len();
        if count == 0 {
            return;
        }
        // Disabled items are stepped over; with every item disabled nothing is highlighted.
        let mut index = self.highlighted;
        for _ in 0..count {
            let next = match index {
                Some(index) if forward => (index + 1) % count,
                Some(index) => (index + count - 1) % count,
                None if forward => 0,
                None => count - 1,
            };
            index = Some(next);
            if !self.items[next].disabled {
                self.highlighted = Some(next);
                break;
            }
        }
        cx.notify();
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(index).filter(|item| !item.disabled) else { return };
        let handler = item.handler.clone();
        cx.emit(DismissEvent);
        handler(window, cx);
    }

    fn render_item(&self, index: usize, item: &MenuItem, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let highlighted = self.highlighted == Some(index);
        if item.disabled {
            return div()
                .id(("menu-item", index))
                .h(px(28.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text3)
                .opacity(0.6)
                .when(item.icon.is_some(), |row| row.child(div().w(px(14.)).flex_none()))
                .child(div().flex_1().min_w_0().truncate().child(item.label.clone()))
                .into_any_element();
        }
        div()
            .id(("menu-item", index))
            .h(px(28.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(8.))
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.text)
            .when(highlighted, |row| row.bg(theme.sel))
            .hover(|style| style.bg(theme.sel))
            .cursor_pointer()
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.highlighted != Some(index) {
                    this.highlighted = Some(index);
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.choose(index, window, cx)))
            .when(item.icon.is_some() || item.swatch.is_some(), |row| {
                row.child(
                    div()
                        .w(px(14.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(item.icon.map(|name| icon(name, theme.text2, 14.)))
                        .children(item.swatch.map(|kind| swatch(kind, theme.text3))),
                )
            })
            .child(div().flex_1().min_w_0().truncate().child(item.label.clone()))
            .children(item.detail.clone().map(|detail| {
                div()
                    .flex_none()
                    .text_size(px(11.5))
                    .text_color(theme.text3)
                    .font_family(crate::theme::MONO)
                    .child(detail)
            }))
            .when(item.checked, |row| row.child(icon("check", theme.accent, 13.)))
            .into_any_element()
    }

    fn render_heading(&self, index: usize, label: SharedString, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(("menu-heading", index))
            .h(px(24.))
            .px(px(10.))
            .flex()
            .items_end()
            .pb(px(3.))
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().text3)
            .child(label)
            .into_any_element()
    }
}

impl Render for PopoverMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut items: Vec<AnyElement> = Vec::new();
        let mut group: Option<&SharedString> = None;
        for (index, item) in self.items.iter().enumerate() {
            if item.group.is_some() && item.group.as_ref() != group {
                group = item.group.as_ref();
                items.push(self.render_heading(index, item.group.clone().unwrap_or_default(), cx));
            } else if item.group.is_none() && group.is_some() {
                // An item after a group stands apart from it.
                group = None;
                items.push(separator(index, cx));
            }
            items.push(self.render_item(index, item, cx));
        }
        let theme = cx.theme();
        glass(theme)
            .bg(theme.card)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.step(false, cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step(true, cx)))
            .on_action(cx.listener(|this, _: &Confirm, window, cx| {
                if let Some(index) = this.highlighted {
                    this.choose(index, window, cx);
                }
            }))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(DismissEvent)))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
            .min_w(px(180.))
            .when_some(self.width, |menu, width| menu.w(width))
            .rounded(px(12.))
            .p(px(4.))
            .flex()
            .flex_col()
            .children(items)
    }
}

/// A thin line between a group and what follows it.
fn separator(index: usize, cx: &mut Context<PopoverMenu>) -> AnyElement {
    div().id(("menu-separator", index)).h(px(1.)).mx(px(8.)).my(px(4.)).bg(cx.theme().line).into_any_element()
}

/// `menu` at `position` (window coordinates), kept inside the window and
/// painted above the rest of it. Render it as the last child of its owner.
pub fn popover(menu: gpui::Entity<PopoverMenu>, position: Point<Pixels>) -> impl IntoElement {
    deferred(anchored().position(position).snap_to_window_with_margin(px(MARGIN)).child(menu)).with_priority(2)
}
