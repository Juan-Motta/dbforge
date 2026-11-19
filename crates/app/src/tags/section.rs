//! The settings' list of tags, as the web UI's `TagsSection.tsx`. Colours,
//! the confirmation switch, adding and deleting save at once; a name saves
//! on Enter or when its field is left. Saves run one after another: a
//! change made while one runs waits and applies to what that one saved. A
//! refused save shows the core's reason and changes nothing.

use std::collections::HashMap;

use dbforge_core::connections::commands;
use dbforge_core::connections::tags::{Tag, TagColor};
use gpui::{
    App, AppContext, Context, DismissEvent, Entity, EventEmitter, Focusable, FontWeight, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render, StatefulInteractiveElement, Styled,
    Subscription, Window, div, point, px,
};

use super::{MAX_TAG_NAME, TAG_COLORS, clamp_tag_name, new_tag, new_tag_name};
use crate::backend::Backend;
use crate::i18n::Translate;
use crate::theme::ActiveTheme;
use crate::ui::field::{Tooltip, frame};
use crate::ui::{Checkbox, MenuItem, PopoverMenu, Swatch, TextInput, TextInputEvent, button, icon_button, popover};

/// What the list tells its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagsEvent {
    /// The tags were saved; when `removed`, connections that had a deleted tag lost it too.
    Saved { removed: bool },
}

/// A tag's name field and what listens to it.
struct NameField {
    input: Entity<TextInput>,
    _subscriptions: Vec<Subscription>,
}

/// A change to the list, applied to the newest one when it is saved.
type Edit = Box<dyn FnOnce(&mut Vec<Tag>)>;

/// The colour picker open under a tag's swatch.
struct OpenPicker {
    menu: Entity<PopoverMenu>,
    position: Point<Pixels>,
    _closing: Subscription,
}

pub struct TagsSection {
    tags: Vec<Tag>,
    names: HashMap<String, NameField>,
    picker: Option<OpenPicker>,
    error: Option<String>,
    saving: bool,
    /// Changes made while a save ran: applied, in order, once it ends.
    queued: Vec<Edit>,
    /// A tag just added: its name field takes the keyboard, ready to be named.
    added: Option<String>,
}

impl EventEmitter<TagsEvent> for TagsSection {}

impl TagsSection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tags = Backend::global(cx).core().tags().list();
        let mut section = Self {
            tags: Vec::new(),
            names: HashMap::new(),
            picker: None,
            error: None,
            saving: false,
            queued: Vec::new(),
            added: None,
        };
        section.show(tags, window, cx);
        section
    }

    /// Reads the tags again, e.g. once a backup brought some in.
    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tags = Backend::global(cx).core().tags().list();
        self.show(tags, window, cx);
    }

    /// Shows `tags` as saved: each name field holds its tag's name, except
    /// the one being typed in, which keeps what was typed.
    fn show(&mut self, tags: Vec<Tag>, window: &mut Window, cx: &mut Context<Self>) {
        self.names.retain(|id, _| tags.iter().any(|tag| &tag.id == id));
        for tag in &tags {
            match self.names.get(&tag.id) {
                Some(field) => {
                    if !field.input.read(cx).is_focused(window) {
                        field.input.update(cx, |input, cx| input.set_text(&tag.name, cx));
                    }
                }
                None => {
                    let field = self.name_field(&tag.id, &tag.name, window, cx);
                    self.names.insert(tag.id.clone(), field);
                }
            }
        }
        self.tags = tags;
        if let Some(field) = self.added.as_ref().and_then(|id| self.names.get(id)) {
            field.input.update(cx, |input, cx| input.select_all(cx));
            field.input.read(cx).focus(window);
        }
        cx.notify();
    }

    fn name_field(&self, id: &str, name: &str, window: &mut Window, cx: &mut Context<Self>) -> NameField {
        let input = cx.new(|cx| {
            let mut input = TextInput::new("", cx);
            input.set_text(name, cx);
            input
        });
        let id = id.to_owned();
        let edited = {
            let id = id.clone();
            cx.subscribe_in(&input, window, move |this, input, event: &TextInputEvent, window, cx| match event {
                // A badge has room for so many characters, as the web UI's field allowed.
                TextInputEvent::Changed => {
                    let text = input.read(cx).text().to_owned();
                    let kept = clamp_tag_name(&text);
                    if kept.len() < text.len() {
                        let kept = kept.to_owned();
                        input.update(cx, |input, cx| input.set_text(&kept, cx));
                    }
                }
                // Enter leaves the field, which saves the name.
                TextInputEvent::Submitted => window.blur(),
                TextInputEvent::Cancelled => {
                    if let Some(tag) = this.tags.iter().find(|tag| tag.id == id) {
                        let name = tag.name.clone();
                        input.update(cx, |input, cx| input.set_text(&name, cx));
                    }
                    window.blur();
                }
                TextInputEvent::Pasted(_) => {}
            })
        };
        let left = cx.on_blur(&input.focus_handle(cx), window, move |this, window, cx| {
            if this.added.as_deref() == Some(id.as_str()) {
                this.added = None;
            }
            if this.with_names(cx) != this.tags {
                // The names typed are read as the save starts.
                this.commit(Box::new(|_| {}), window, cx);
            }
        });
        NameField { input, _subscriptions: vec![edited, left] }
    }

    /// The tags with the names being typed.
    fn with_names(&self, cx: &App) -> Vec<Tag> {
        self.tags
            .iter()
            .map(|tag| {
                let name = self.names.get(&tag.id).map(|field| field.input.read(cx).text().to_owned());
                Tag { name: name.unwrap_or_else(|| tag.name.clone()), ..tag.clone() }
            })
            .collect()
    }

    /// Saves the list with the names typed and `edit` applied, once the
    /// save running now, if any, ends.
    fn commit(&mut self, edit: Edit, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        self.queued.push(edit);
        if !self.saving {
            self.save_queued(window, cx);
        }
        cx.notify();
    }

    /// Saves the list with every waiting change applied.
    fn save_queued(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.queued.is_empty() {
            return;
        }
        let mut next = self.with_names(cx);
        for edit in self.queued.drain(..) {
            edit(&mut next);
        }
        self.saving = true;
        let before = self.tags.clone();
        let saving = Backend::global(cx).spawn_blocking(cx, move |core| {
            commands::save_tags(core.connections(), core.tags(), next).map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let saved = saving.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match saved {
                    Ok(saved) => {
                        let removed = before.iter().any(|tag| !saved.iter().any(|kept| kept.id == tag.id));
                        this.show(saved, window, cx);
                        cx.emit(TagsEvent::Saved { removed });
                    }
                    Err(message) => {
                        this.error = Some(message);
                        this.added = None;
                        cx.notify();
                    }
                }
                this.save_queued(window, cx);
            });
        })
        .detach();
    }

    fn change(
        &mut self,
        id: &str,
        change: impl FnOnce(&mut Tag) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = id.to_owned();
        let edit = move |tags: &mut Vec<Tag>| {
            if let Some(tag) = tags.iter_mut().find(|tag| tag.id == id) {
                change(tag);
            }
        };
        self.commit(Box::new(edit), window, cx);
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let new_name = cx.messages().tags.new_name;
        let id = uuid::Uuid::new_v4().to_string();
        self.added = Some(id.clone());
        let edit = move |tags: &mut Vec<Tag>| {
            let taken: Vec<String> = tags.iter().map(|tag| tag.name.clone()).collect();
            let tag = new_tag(tags, new_tag_name(&taken, new_name), id);
            tags.push(tag);
        };
        self.commit(Box::new(edit), window, cx);
    }

    fn remove(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = id.to_owned();
        self.commit(Box::new(move |tags: &mut Vec<Tag>| tags.retain(|tag| tag.id != id)), window, cx);
    }

    /// Opens the colours `id` can take, under its swatch.
    fn pick_color(&mut self, id: &str, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.tags.iter().find(|tag| tag.id == id).map(|tag| tag.color) else { return };
        let theme = *cx.theme();
        let m = &cx.messages().tags;
        let this = cx.entity().downgrade();
        let items = TAG_COLORS
            .into_iter()
            .map(|color: TagColor| {
                let (this, id) = (this.clone(), id.to_owned());
                MenuItem::new((m.color_name)(color), move |window, cx| {
                    let _ = this.update(cx, |this, cx| this.change(&id, move |tag| tag.color = color, window, cx));
                })
                .swatch(Some(Swatch::Color(theme.tag(color))))
                .checked(color == current)
            })
            .collect();
        let menu = cx.new(|cx| PopoverMenu::new(items, window, cx));
        let closing = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.picker = None;
            cx.notify();
        });
        self.picker = Some(OpenPicker { menu, position, _closing: closing });
        cx.notify();
    }

    fn render_row(&self, index: usize, tag: &Tag, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().tags;
        let id = tag.id.clone();
        let name = self.names.get(&tag.id).map(|field| field.input.clone());
        let focused = name.as_ref().is_some_and(|input| input.read(cx).is_focused(window));
        let (pick, toggle, remove) = (id.clone(), id.clone(), id);
        div()
            .h(px(44.))
            .flex()
            .items_center()
            .gap(px(12.))
            .border_b_1()
            .border_color(theme.line)
            .child(
                div()
                    .id(("tag-color", index))
                    .size(px(20.))
                    .flex_none()
                    .rounded(px(10.))
                    .bg(theme.tag(tag.color))
                    .border_1()
                    .border_color(theme.rim)
                    .cursor_pointer()
                    .tooltip(Tooltip::text((m.color)(&tag.name)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            if this.picker.is_none() {
                                this.pick_color(&pick, event.position + point(px(0.), px(14.)), window, cx);
                            }
                        }),
                    ),
            )
            .child(
                frame(&theme, focused, false, false)
                    .w(px(MAX_TAG_NAME as f32 * 9. + 24.))
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.tag(tag.color))
                    .children(name),
            )
            .child(
                div().flex_1().min_w_0().id(("tag-confirm-help", index)).tooltip(Tooltip::text(m.confirm_help)).child(
                    Checkbox::new(("tag-confirm", index), m.confirm, tag.confirm).on_change({
                        let this = cx.entity().downgrade();
                        move |checked, window, cx| {
                            let checked = *checked;
                            let _ = this.update(cx, |this, cx| {
                                this.change(&toggle, move |tag| tag.confirm = checked, window, cx)
                            });
                        }
                    }),
                ),
            )
            .child(
                icon_button(&theme, ("tag-remove", index), "trash", true, false)
                    .tooltip(Tooltip::text((m.remove)(&tag.name)))
                    .on_click(cx.listener(move |this, _, window, cx| this.remove(&remove, window, cx))),
            )
    }
}

impl Render for TagsSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().tags;
        let rows: Vec<gpui::AnyElement> = self
            .tags
            .clone()
            .iter()
            .enumerate()
            .map(|(index, tag)| self.render_row(index, tag, window, cx).into_any_element())
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().flex().flex_col().children(rows))
            .child(
                div().flex().child(
                    button(&theme, Some("plus"), m.add)
                        .id("tag-add")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                ),
            )
            // Always there, so an error never moves the list.
            .child(div().min_h(px(18.)).text_size(px(12.5)).text_color(theme.err).children(self.error.clone()))
            .children(self.picker.as_ref().map(|open| popover(open.menu.clone(), open.position)))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::TestAppContext;

    use super::*;
    use crate::test_support::{draw, setup, wait_until};

    #[gpui::test]
    fn adds_recolors_and_deletes_tags_saving_each_change_at_once(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (section, cx) = cx.add_window_view(TagsSection::new);
        let events: Rc<RefCell<Vec<TagsEvent>>> = Rc::default();
        let heard = events.clone();
        cx.update(|_, cx| {
            cx.subscribe(&section, move |_, event: &TagsEvent, _| heard.borrow_mut().push(*event)).detach()
        });
        draw(cx);
        let saved = |cx: &mut gpui::VisualTestContext| cx.update(|_, cx| Backend::global(cx).core().tags().list());
        assert_eq!(saved(cx).len(), 4, "PROD, STAGE, TEST and DEV to start with");

        section.update_in(cx, |section, window, cx| section.add(window, cx));
        wait_until(cx, "the new tag to be saved", |cx| saved(cx).len() == 5);
        let added = saved(cx).pop().expect("the new tag");
        assert_eq!((added.name.as_str(), added.color, added.confirm), ("NEW", TagColor::Orange, false));
        draw(cx);

        let id = added.id.clone();
        section.update_in(cx, |section, window, cx| section.change(&id, |tag| tag.color = TagColor::Teal, window, cx));
        wait_until(cx, "the colour to be saved", |cx| saved(cx).last().is_some_and(|tag| tag.color == TagColor::Teal));

        section.update_in(cx, |section, window, cx| section.remove(&id, window, cx));
        wait_until(cx, "the tag to be deleted", |cx| saved(cx).len() == 4);
        cx.run_until_parked();
        assert_eq!(
            *events.borrow(),
            [
                TagsEvent::Saved { removed: false },
                TagsEvent::Saved { removed: false },
                TagsEvent::Saved { removed: true }
            ]
        );
    }

    #[gpui::test]
    fn a_change_made_while_a_save_runs_applies_to_what_that_save_kept(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (section, cx) = cx.add_window_view(TagsSection::new);
        let saved = |cx: &mut gpui::VisualTestContext| cx.update(|_, cx| Backend::global(cx).core().tags().list());
        // Both before the first save answers: the second waits, then applies to the first's result.
        section.update_in(cx, |section, window, cx| {
            section.change("production", |tag| tag.color = TagColor::Pink, window, cx);
            section.remove("staging", window, cx);
            assert!(section.saving, "the first save runs; the second waits");
        });
        wait_until(cx, "both changes", |cx| {
            let tags = saved(cx);
            tags.len() == 3 && tags[0].color == TagColor::Pink
        });
        wait_until(cx, "the saves to end", |cx| section.read_with(cx, |section, _| !section.saving));
        section.read_with(cx, |section, _| assert_eq!(section.tags.len(), 3));
    }

    #[gpui::test]
    fn shows_the_core_s_reason_when_it_refuses_a_list(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (section, cx) = cx.add_window_view(TagsSection::new);
        let first = section.read_with(cx, |section, _| section.tags[0].id.clone());
        // Two tags with one name: the core refuses the list and keeps the saved one.
        section
            .update_in(cx, |section, window, cx| section.change(&first, |tag| tag.name = "STAGE".into(), window, cx));
        wait_until(cx, "the refusal", |cx| section.read_with(cx, |section, _| section.error.is_some()));
        section.read_with(cx, |section, _| assert_eq!(section.tags[0].name, "PROD"));
        draw(cx);
    }
}
