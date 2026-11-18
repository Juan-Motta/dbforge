//! A small popover menu: a list of commands at a point (the pointer on a
//! right-click, a button's corner). It paints above everything, takes the
//! keyboard (↑ ↓ Enter), and closes on Escape, a click outside it or a choice,
//! telling its owner with a [`DismissEvent`].

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Pixels, Point, Render, SharedString, StatefulInteractiveElement, Styled,
    Window, actions, anchored, deferred, div, prelude::FluentBuilder, px,
};

use super::{glass, icon};
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

/// One command in a [`PopoverMenu`].
#[derive(Clone)]
pub struct MenuItem {
    pub label: SharedString,
    pub icon: Option<&'static str>,
    pub handler: MenuHandler,
}

impl MenuItem {
    pub fn new(label: impl Into<SharedString>, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { label: label.into(), icon: None, handler: Rc::new(handler) }
    }

    /// An icon from [`crate::assets`] before the label.
    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }
}

pub struct PopoverMenu {
    items: Vec<MenuItem>,
    /// The item the keyboard is on.
    highlighted: Option<usize>,
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
    pub fn new(items: Vec<MenuItem>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        Self { highlighted: (!items.is_empty()).then_some(0), items, focus }
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.items.len();
        if count == 0 {
            return;
        }
        self.highlighted = Some(match self.highlighted {
            Some(index) if forward => (index + 1) % count,
            Some(index) => (index + count - 1) % count,
            None if forward => 0,
            None => count - 1,
        });
        cx.notify();
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(index) else { return };
        let handler = item.handler.clone();
        cx.emit(DismissEvent);
        handler(window, cx);
    }

    fn render_item(&self, index: usize, item: &MenuItem, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let highlighted = self.highlighted == Some(index);
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
            .child(div().w(px(14.)).flex_none().children(item.icon.map(|name| icon(name, theme.text2, 14.))))
            .child(div().truncate().child(item.label.clone()))
            .into_any_element()
    }
}

impl Render for PopoverMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items: Vec<AnyElement> =
            self.items.iter().enumerate().map(|(index, item)| self.render_item(index, item, cx)).collect();
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
            .rounded(px(12.))
            .p(px(4.))
            .flex()
            .flex_col()
            .children(items)
    }
}

/// `menu` at `position` (window coordinates), kept inside the window and
/// painted above the rest of it. Render it as the last child of its owner.
pub fn popover(menu: gpui::Entity<PopoverMenu>, position: Point<Pixels>) -> impl IntoElement {
    deferred(anchored().position(position).snap_to_window_with_margin(px(MARGIN)).child(menu)).with_priority(2)
}
