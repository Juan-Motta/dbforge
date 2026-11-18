//! A dropdown: the chosen option in a framed box, and a [`PopoverMenu`] of
//! the others under it, as wide as the box. It takes the keyboard like a
//! field (Tab reaches it; Space or ↓ opens it, ↑ ↓ Enter choose), and tells
//! its owner of a choice with [`SelectEvent`]. Enter on a closed dropdown is
//! left to the form, which applies.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, AppContext, Bounds, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyBinding, MouseButton, ParentElement, Pixels, Render, SharedString, Styled, Subscription, Window,
    actions, div, point, prelude::FluentBuilder, px,
};

use super::field::frame;
use super::icon;
use super::menu::{MenuItem, PopoverMenu, Swatch, popover, swatch};
use crate::theme::ActiveTheme;

actions!(select, [Open]);

const CONTEXT: &str = "Select";

pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    vec![KeyBinding::new("space", Open, context), KeyBinding::new("down", Open, context)]
}

/// One choice of a [`Select`].
#[derive(Clone)]
pub struct SelectOption<T> {
    pub value: T,
    pub label: SharedString,
    /// The heading it is listed under in the menu.
    pub group: Option<SharedString>,
    pub swatch: Option<Swatch>,
}

impl<T> SelectOption<T> {
    pub fn new(value: T, label: impl Into<SharedString>) -> Self {
        Self { value, label: label.into(), group: None, swatch: None }
    }

    pub fn group(mut self, group: impl Into<SharedString>) -> Self {
        self.group = Some(group.into());
        self
    }

    pub fn swatch(mut self, swatch: Swatch) -> Self {
        self.swatch = Some(swatch);
        self
    }
}

/// The user chose `0`.
pub struct SelectEvent<T>(pub T);

pub struct Select<T: Clone + PartialEq + 'static> {
    options: Vec<SelectOption<T>>,
    value: Option<T>,
    /// Shown while nothing is chosen.
    placeholder: SharedString,
    invalid: bool,
    disabled: bool,
    focus: FocusHandle,
    menu: Option<(Entity<PopoverMenu>, Subscription)>,
    /// Where the box was last drawn, for the menu to drop from.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl<T: Clone + PartialEq + 'static> EventEmitter<SelectEvent<T>> for Select<T> {}

impl<T: Clone + PartialEq + 'static> Focusable for Select<T> {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<T: Clone + PartialEq + 'static> Select<T> {
    pub fn new(options: Vec<SelectOption<T>>, value: Option<T>, cx: &mut Context<Self>) -> Self {
        Self {
            options,
            value,
            placeholder: SharedString::default(),
            invalid: false,
            disabled: false,
            focus: cx.focus_handle().tab_stop(true),
            menu: None,
            bounds: Rc::default(),
        }
    }

    /// Its list of choices is showing.
    pub fn is_open(&self) -> bool {
        self.menu.is_some()
    }

    pub fn value(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// Sets the choice without a [`SelectEvent`]: the owner set it.
    pub fn set_value(&mut self, value: Option<T>, cx: &mut Context<Self>) {
        if self.value != value {
            self.value = value;
            cx.notify();
        }
    }

    pub fn set_options(&mut self, options: Vec<SelectOption<T>>, cx: &mut Context<Self>) {
        self.options = options;
        cx.notify();
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// Its border shows the choice is missing or can't be used.
    pub fn set_invalid(&mut self, invalid: bool, cx: &mut Context<Self>) {
        if self.invalid != invalid {
            self.invalid = invalid;
            cx.notify();
        }
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            self.focus = self.focus.clone().tab_stop(!disabled);
            self.menu = None;
            cx.notify();
        }
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    fn selected(&self) -> Option<&SelectOption<T>> {
        let value = self.value.as_ref()?;
        self.options.iter().find(|option| &option.value == value)
    }

    /// Opens the list of choices.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.menu.is_some() {
            return;
        }
        let this = cx.entity().downgrade();
        let items = self
            .options
            .iter()
            .map(|option| {
                let (this, value) = (this.clone(), option.value.clone());
                MenuItem::new(option.label.clone(), move |_, cx| {
                    let value = value.clone();
                    let _ = this.update(cx, |this, cx| this.choose(value, cx));
                })
                .swatch(option.swatch)
                .group(option.group.clone())
                .checked(self.value.as_ref() == Some(&option.value))
            })
            .collect();
        let width = self.bounds.get().map(|bounds| bounds.size.width);
        let menu = cx.new(|cx| {
            let menu = PopoverMenu::new(items, window, cx);
            match width {
                Some(width) => menu.with_width(width),
                None => menu,
            }
        });
        let closing = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
            this.menu = None;
            window.focus(&this.focus);
            cx.notify();
        });
        self.menu = Some((menu, closing));
        cx.notify();
    }

    fn choose(&mut self, value: T, cx: &mut Context<Self>) {
        if self.value.as_ref() != Some(&value) {
            self.value = Some(value.clone());
            cx.emit(SelectEvent(value));
        }
        cx.notify();
    }
}

impl<T: Clone + PartialEq + 'static> Render for Select<T> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let focused = self.focus.is_focused(window) || self.menu.is_some();
        let selected = self.selected().cloned();
        let bounds = self.bounds.clone();
        let (label, tint) = match &selected {
            Some(option) => (option.label.clone(), theme.text),
            None => (self.placeholder.clone(), theme.text3),
        };
        let position = self.bounds.get().map(|bounds| bounds.bottom_left() + point(px(0.), px(4.)));
        div()
            .on_children_prepainted(move |children, _, _| bounds.set(children.first().copied()))
            .child(
                frame(&theme, focused, self.invalid, self.disabled)
                    .track_focus(&self.focus)
                    .key_context(CONTEXT)
                    .on_action(cx.listener(|this, _: &Open, window, cx| this.open(window, cx)))
                    // On press, while an open menu's own click-outside is still closing it, so
                    // pressing the box again closes the menu rather than reopening it.
                    .when(!self.disabled, |frame| {
                        frame.cursor_pointer().on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.focus(window);
                                this.open(window, cx);
                            }),
                        )
                    })
                    .text_color(tint)
                    .children(selected.and_then(|option| option.swatch).map(|kind| swatch(kind, theme.text3)))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(icon("down", theme.text3, 13.)),
            )
            .children(self.menu.as_ref().zip(position).map(|((menu, _), position)| popover(menu.clone(), position)))
    }
}
