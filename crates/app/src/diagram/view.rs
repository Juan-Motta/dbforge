//! An ER diagram's view: the schema read from the core (the drawing stays
//! while it is read again), which tables are shown (a list beside a
//! schema's diagram, a reach of relations in a table's own), the cards laid
//! out, and what the pointer does: drag the canvas to pan, drag a card to
//! move it, the wheel to zoom around the pointer, a click to pin a table's
//! highlight, a double click to bring it close.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use dbforge_core::drivers::{Relationship, SchemaDiagram};
use dbforge_core::explorer::schema_diagram;
use gpui::{
    AnyElement, AppContext, Context, Entity, EventEmitter, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Subscription, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder, px, uniform_list,
};

use super::canvas::DiagramCanvas;
use super::layout::{
    CardBox, ColumnMode, Curve, Placed, Point, Size, View, card_height, center_on, compact, default_selection,
    fit_view, foreign_key_columns, layout, local_relationships, neighborhood, relationship_path, shown_columns,
    zoom_at,
};
use crate::backend::Backend;
use crate::grid::columns::short_type_name;
use crate::i18n::Translate;
use crate::theme::{ActiveTheme, Theme};
use crate::ui::field::Tooltip;
use crate::ui::{Checkbox, TextInput, TextInputEvent, any, button, glass, group, icon, icon_button, segment};

/// How far the pointer moves before a press becomes a drag.
const DRAG_THRESHOLD: f32 = 3.;
/// The sidebar's width beside a schema's diagram.
const SIDEBAR_WIDTH: f32 = 240.;
/// A row of the sidebar's table list.
const LIST_ROW: f32 = 26.;
/// What the zoom buttons multiply the scale by.
const ZOOM_STEP: f32 = 1.2;

/// What a diagram asks of its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagramEvent {
    /// A table's diagram: open the whole schema's.
    OpenSchema,
}

/// How far a table's diagram follows relations; `None` shows the whole schema.
const REACH: [Option<usize>; 3] = [Some(1), Some(2), None];

/// A column as its card lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct CardColumn {
    pub name: String,
    /// Shortened, as the grid's headers write it.
    pub data_type: String,
    pub key: Option<ColumnKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKey {
    Primary,
    Foreign,
}

/// A table drawn: its name, row estimate and columns, where it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub name: String,
    pub rows: Option<String>,
    pub columns: Vec<CardColumn>,
    pub position: Point,
    pub height: f32,
}

impl Card {
    fn contains(&self, at: Point) -> bool {
        at.x >= self.position.x
            && at.x <= self.position.x + super::layout::CARD_WIDTH
            && at.y >= self.position.y
            && at.y <= self.position.y + self.height
    }
}

/// A foreign key drawn between two cards.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub table: String,
    pub referenced: String,
    pub curve: Curve,
}

/// What the canvas draws: the cards and lines, and which are highlighted.
#[derive(Debug, Default, PartialEq)]
pub struct Scene {
    pub cards: Vec<Card>,
    pub lines: Vec<Line>,
    /// The table pinned, hovered or dragged, whose relations stand out.
    pub active: Option<String>,
    pub pinned: Option<String>,
    /// The active table and those tied to it; the rest are dimmed.
    pub related: HashSet<String>,
}

/// What a pointer drag is moving: the whole canvas, or one card.
enum Drag {
    Pan { start: Point, origin: View, moved: bool },
    Card { name: String, start: Point, origin: Point, moved: bool },
}

/// The diagram once read, and what the user did with it.
struct Shown {
    diagram: Rc<SchemaDiagram>,
    relationships: Vec<Relationship>,
    /// Every foreign key column gets a mark, even when its target is in another schema.
    foreign_keys: HashSet<String>,
    reach: Option<usize>,
    visible: HashSet<String>,
    mode: ColumnMode,
    /// Cards the user moved, where they put them.
    moved: HashMap<String, Point>,
    /// Where the layout puts the cards shown.
    auto: HashMap<String, Point>,
    view: View,
    pinned: Option<String>,
    hovered: Option<String>,
    drag: Option<Drag>,
    scene: Rc<Scene>,
}

pub struct DiagramView {
    connection_id: String,
    database: String,
    schema: String,
    /// A table's own diagram: centred on it, with its neighbours, and no table list.
    focus: Option<String>,
    shown: Option<Shown>,
    error: Option<String>,
    loading: bool,
    /// The newest read asked for; an older one arriving late is dropped.
    request: u64,
    filter: Entity<TextInput>,
    list: UniformListScrollHandle,
    /// The canvas's size, from its last paint.
    pub(super) viewport: Size,
    /// Fit the cards into the canvas at its next paint.
    pub(super) pending_fit: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DiagramEvent> for DiagramView {}

impl DiagramView {
    pub fn new(
        connection_id: String,
        database: String,
        schema: String,
        focus: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let m = &cx.messages().diagram;
        let filter = cx.new(|cx| TextInput::new(m.filter_placeholder, cx));
        let subscriptions = vec![cx.subscribe_in(&filter, window, |this, _, event: &TextInputEvent, _, cx| {
            if matches!(event, TextInputEvent::Changed) {
                this.list.scroll_to_item(0, gpui::ScrollStrategy::Top);
                cx.notify();
            }
        })];
        let mut view = Self {
            connection_id,
            database,
            schema,
            focus,
            shown: None,
            error: None,
            loading: false,
            request: 0,
            filter,
            list: UniformListScrollHandle::new(),
            viewport: Size { width: 0., height: 0. },
            pending_fit: false,
            _subscriptions: subscriptions,
        };
        view.reload(cx);
        view
    }

    /// Reads the schema again; the drawing shown stays until it arrives.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.request += 1;
        let request = self.request;
        self.loading = true;
        let (connection, database, schema) = (self.connection_id.clone(), self.database.clone(), self.schema.clone());
        let reading = Backend::global(cx).spawn(cx, move |core| async move {
            schema_diagram(core.connections(), core.statement_log(), connection, database, schema).await
        });
        cx.spawn(async move |this, cx| {
            let result = reading.await;
            let _ = this.update(cx, |this, cx| {
                if this.request != request {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(diagram) => this.show(diagram),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Draws `diagram` as if it had just opened: its default tables, laid
    /// out and fitted into the canvas.
    pub fn show(&mut self, diagram: SchemaDiagram) {
        self.error = None;
        let relationships = local_relationships(&diagram, &self.schema);
        let foreign_keys = foreign_key_columns(&diagram);
        let visible = match &self.focus {
            Some(focus) => neighborhood(&relationships, focus, 1),
            None => default_selection(&diagram, &relationships),
        };
        self.shown = Some(Shown {
            diagram: Rc::new(diagram),
            relationships,
            foreign_keys,
            reach: Some(1),
            visible,
            mode: ColumnMode::All,
            moved: HashMap::new(),
            auto: HashMap::new(),
            view: View::default(),
            pinned: self.focus.clone(),
            hovered: None,
            drag: None,
            scene: Rc::default(),
        });
        self.relayout();
        self.pending_fit = true;
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// What the canvas draws now.
    pub fn scene(&self) -> Option<Rc<Scene>> {
        self.shown.as_ref().map(|shown| shown.scene.clone())
    }

    pub fn view(&self) -> Option<View> {
        self.shown.as_ref().map(|shown| shown.view)
    }

    #[cfg(test)]
    pub(super) fn set_view_for_tests(&mut self, view: View) {
        if let Some(shown) = &mut self.shown {
            shown.view = view;
        }
    }

    pub(super) fn is_dragging(&self) -> bool {
        self.shown.as_ref().is_some_and(|shown| shown.drag.is_some())
    }

    /// Lays the cards shown out again, then places them.
    fn relayout(&mut self) {
        let Some(shown) = &mut self.shown else { return };
        let cards = build_cards(shown);
        let placed: Vec<Placed> = cards.iter().map(|card| Placed { name: &card.name, height: card.height }).collect();
        let references: Vec<(&str, &str)> =
            shown.relationships.iter().map(|r| (r.table.as_str(), r.referenced_table.as_str())).collect();
        shown.auto = layout(&placed, &references);
        self.place();
    }

    /// Puts each card where it was moved or laid out, and draws the lines between them.
    fn place(&mut self) {
        let Some(shown) = &mut self.shown else { return };
        let mut cards = build_cards(shown);
        for card in &mut cards {
            card.position =
                shown.moved.get(&card.name).or_else(|| shown.auto.get(&card.name)).copied().unwrap_or_default();
        }
        let by_name: HashMap<&str, &Card> = cards.iter().map(|card| (card.name.as_str(), card)).collect();
        let row_of = |card: &Card, column: Option<&String>| {
            column.and_then(|name| card.columns.iter().position(|c| &c.name == name)).map_or(-1, |row| row as isize)
        };
        let lines = shown
            .relationships
            .iter()
            .filter_map(|r| {
                let (from, to) = (by_name.get(r.table.as_str())?, by_name.get(r.referenced_table.as_str())?);
                let curve = relationship_path(
                    from.position,
                    row_of(from, r.columns.first()),
                    to.position,
                    row_of(to, r.referenced_columns.first()),
                );
                Some(Line { table: r.table.clone(), referenced: r.referenced_table.clone(), curve })
            })
            .collect();
        let mut scene = Scene { cards, lines, ..Scene::default() };
        highlight(&mut scene, shown);
        shown.scene = Rc::new(scene);
    }

    /// The highlight follows the pointer and the pin; nothing moves.
    fn rehighlight(&mut self) {
        let Some(shown) = &mut self.shown else { return };
        let mut scene =
            Scene { cards: shown.scene.cards.clone(), lines: shown.scene.lines.clone(), ..Scene::default() };
        highlight(&mut scene, shown);
        shown.scene = Rc::new(scene);
    }

    // What the user changes

    pub fn set_mode(&mut self, mode: ColumnMode, cx: &mut Context<Self>) {
        if let Some(shown) = &mut self.shown {
            shown.mode = mode;
            self.relayout();
            cx.notify();
        }
    }

    /// A table's diagram follows relations `steps` far (the whole schema
    /// when `None`), laid out again and fitted.
    pub fn set_reach(&mut self, steps: Option<usize>, cx: &mut Context<Self>) {
        let (Some(shown), Some(focus)) = (&mut self.shown, &self.focus) else { return };
        shown.reach = steps;
        shown.visible = match steps {
            Some(steps) => neighborhood(&shown.relationships, focus, steps),
            None => shown.diagram.tables.iter().map(|table| table.name.clone()).collect(),
        };
        shown.moved.clear();
        self.relayout();
        self.pending_fit = true;
        cx.notify();
    }

    /// Shows or hides one table.
    pub fn toggle(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(shown) = &mut self.shown else { return };
        if !shown.visible.remove(name) {
            shown.visible.insert(name.to_owned());
        }
        self.relayout();
        cx.notify();
    }

    /// Shows, or hides, every table the filter lists.
    fn show_listed(&mut self, show: bool, cx: &mut Context<Self>) {
        let listed: Vec<String> = self.listed(cx).into_iter().map(|(name, _)| name).collect();
        let Some(shown) = &mut self.shown else { return };
        for name in listed {
            if show {
                shown.visible.insert(name);
            } else {
                shown.visible.remove(&name);
            }
        }
        self.relayout();
        cx.notify();
    }

    /// Puts every card back where the layout places it.
    pub fn auto_layout(&mut self, cx: &mut Context<Self>) {
        if let Some(shown) = &mut self.shown {
            shown.moved.clear();
            self.place();
            cx.notify();
        }
    }

    /// Fits every card into the canvas.
    pub fn fit(&mut self, cx: &mut Context<Self>) {
        self.pending_fit = true;
        cx.notify();
    }

    /// The canvas is `viewport` big: fits the cards into it if asked to;
    /// whether it did.
    pub(super) fn fit_now(&mut self) -> bool {
        if !self.pending_fit || self.viewport.width <= 0. {
            return false;
        }
        let Some(shown) = &mut self.shown else { return false };
        self.pending_fit = false;
        let cards: Vec<CardBox> =
            shown.scene.cards.iter().map(|card| CardBox { position: card.position, height: card.height }).collect();
        shown.view = fit_view(&cards, self.viewport);
        true
    }

    pub fn zoom_by(&mut self, factor: f32, cx: &mut Context<Self>) {
        let center = Point::new(self.viewport.width / 2., self.viewport.height / 2.);
        self.zoom(factor, center, cx);
    }

    fn zoom(&mut self, factor: f32, anchor: Point, cx: &mut Context<Self>) {
        if let Some(shown) = &mut self.shown {
            shown.view = zoom_at(shown.view, factor, anchor);
            cx.notify();
        }
    }

    // The pointer, in the canvas's own pixels

    /// The wheel zooms around the pointer: scrolling down zooms out.
    pub(super) fn wheel(&mut self, delta_y: f32, at: Point, cx: &mut Context<Self>) {
        self.zoom((delta_y / 300.).exp(), at, cx);
    }

    /// Anywhere on a card moves that card; anywhere else pans the canvas.
    pub(super) fn press(&mut self, at: Point, cx: &mut Context<Self>) {
        let Some(shown) = &mut self.shown else { return };
        let point = shown.view.to_diagram(at);
        let card = shown.scene.cards.iter().rev().find(|card| card.contains(point));
        shown.drag = Some(match card {
            Some(card) => Drag::Card { name: card.name.clone(), start: at, origin: card.position, moved: false },
            None => Drag::Pan { start: at, origin: shown.view, moved: false },
        });
        self.rehighlight();
        cx.notify();
    }

    /// The pointer moved: a drag goes on, or the card under it lights up.
    pub(super) fn pointer_moved(&mut self, at: Point, over_canvas: bool, cx: &mut Context<Self>) {
        let Some(shown) = &mut self.shown else { return };
        let scale = shown.view.scale;
        match &mut shown.drag {
            Some(Drag::Pan { start, origin, moved }) => {
                let (dx, dy) = (at.x - start.x, at.y - start.y);
                *moved |= dx.hypot(dy) > DRAG_THRESHOLD;
                shown.view = View { x: origin.x + dx, y: origin.y + dy, scale: origin.scale };
                cx.notify();
            }
            Some(Drag::Card { name, start, origin, moved }) => {
                let (dx, dy) = (at.x - start.x, at.y - start.y);
                if *moved || dx.hypot(dy) > DRAG_THRESHOLD {
                    *moved = true;
                    let position = Point::new(origin.x + dx / scale, origin.y + dy / scale);
                    shown.moved.insert(name.clone(), position);
                    self.place();
                    cx.notify();
                }
            }
            None => {
                let point = shown.view.to_diagram(at);
                let hovered = over_canvas
                    .then(|| shown.scene.cards.iter().rev().find(|card| card.contains(point)))
                    .flatten()
                    .map(|card| card.name.clone());
                if hovered != shown.hovered {
                    shown.hovered = hovered;
                    self.rehighlight();
                    cx.notify();
                }
            }
        }
    }

    /// The press ends. A click on a card pins its highlight (or unpins it),
    /// a double click brings it close; a click on empty canvas unpins.
    pub(super) fn release(&mut self, click_count: usize, cx: &mut Context<Self>) {
        let Some(shown) = &mut self.shown else { return };
        match shown.drag.take() {
            Some(Drag::Pan { moved: false, .. }) => shown.pinned = None,
            Some(Drag::Card { name, moved: false, .. }) => {
                if click_count >= 2 {
                    if let Some(card) = shown.scene.cards.iter().find(|card| card.name == name) {
                        shown.view = center_on(card.position, card.height, self.viewport, 1.);
                    }
                    shown.pinned = Some(name);
                } else if shown.pinned.as_ref() == Some(&name) {
                    shown.pinned = None;
                } else {
                    shown.pinned = Some(name);
                }
            }
            _ => {}
        }
        self.rehighlight();
        cx.notify();
    }

    // Drawing

    /// The tables the sidebar lists, those the filter matches, with whether each is shown.
    fn listed(&self, cx: &gpui::App) -> Vec<(String, bool)> {
        let Some(shown) = &self.shown else { return Vec::new() };
        let filter = self.filter.read(cx).text().to_lowercase();
        shown
            .diagram
            .tables
            .iter()
            .filter(|table| table.name.to_lowercase().contains(&filter))
            .map(|table| (table.name.clone(), shown.visible.contains(&table.name)))
            .collect()
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().diagram;
        let (shown_count, total) =
            self.shown.as_ref().map_or((0, 0), |shown| (shown.visible.len(), shown.diagram.tables.len()));
        let link = |id: &'static str, label: &'static str| {
            div().id(id).text_size(px(12.)).text_color(theme.accent_text).cursor_pointer().child(label)
        };
        let listed = self.listed(cx);
        let list = uniform_list(
            "diagram-tables",
            listed.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let listed = this.listed(cx);
                range
                    .filter_map(|index| listed.get(index).cloned().map(|entry| (index, entry)))
                    .map(|(index, (name, checked))| {
                        let toggled = name.clone();
                        div()
                            .h(px(LIST_ROW))
                            .px(px(4.))
                            .flex()
                            .items_center()
                            .child(
                                Checkbox::new(("diagram-table", index), name, checked)
                                    .icon("table")
                                    .on_change(cx.listener(move |this, _: &bool, _, cx| this.toggle(&toggled, cx))),
                            )
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.list.clone())
        .flex_1();
        div()
            .w(px(SIDEBAR_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(8.))
            .py(px(10.))
            .border_r_1()
            .border_color(theme.line)
            .child(
                div()
                    .px(px(4.))
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .text_size(px(11.5))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text2)
                            .child(format!("{}.{}", self.database, self.schema)),
                    )
                    .child(div().text_color(theme.text3).child((m.shown_of)(shown_count, total))),
            )
            .child(
                div()
                    .h(px(30.))
                    .rounded(px(15.))
                    .bg(theme.sel)
                    .border_1()
                    .border_color(theme.rim)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .text_size(px(12.))
                    .text_color(theme.text)
                    .child(icon("search", theme.text3, 13.))
                    .child(self.filter.clone()),
            )
            .child(
                div()
                    .px(px(4.))
                    .flex()
                    .gap(px(12.))
                    .child(
                        link("show-all", m.show_all).on_click(cx.listener(|this, _, _, cx| this.show_listed(true, cx))),
                    )
                    .child(
                        link("hide-all", m.hide_all)
                            .on_click(cx.listener(|this, _, _, cx| this.show_listed(false, cx))),
                    ),
            )
            .child(list)
            .into_any_element()
    }

    /// The floating bar over the canvas: reach, columns, layout, reload.
    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().diagram;
        let Some(shown) = &self.shown else { return div().into_any_element() };
        let mut bar = floating(&theme).id("diagram-toolbar").top(px(12.)).left(px(12.));
        if self.focus.is_some() {
            let reaches = REACH
                .iter()
                .enumerate()
                .map(|(index, &steps)| {
                    let label = match steps {
                        Some(1) => m.direct,
                        Some(_) => m.two_steps,
                        None => m.whole_schema,
                    };
                    any(segment(&theme, None, label, shown.reach == steps)
                        .id(("reach", index))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.set_reach(steps, cx))))
                })
                .collect();
            bar = bar.child(group(&theme, reaches));
        }
        let modes = [(ColumnMode::All, m.columns), (ColumnMode::Keys, m.keys_only)]
            .into_iter()
            .map(|(mode, label)| {
                any(segment(&theme, None, label, shown.mode == mode)
                    .id(if mode == ColumnMode::All { "mode-all" } else { "mode-keys" })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, cx))))
            })
            .collect();
        bar = bar
            .child(group(&theme, modes))
            .child(
                button(&theme, None, m.auto_layout)
                    .id("auto-layout")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.auto_layout(cx))),
            )
            .child(
                icon_button(&theme, "reload-diagram", "refresh", !self.loading, false)
                    .tooltip(Tooltip::text(m.reload))
                    .when(!self.loading, |reload| reload.on_click(cx.listener(|this, _, _, cx| this.reload(cx)))),
            );
        if self.focus.is_some() {
            bar = bar.child(
                button(&theme, None, m.open_schema)
                    .id("open-schema")
                    .cursor_pointer()
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DiagramEvent::OpenSchema))),
            );
        }
        bar.into_any_element()
    }

    fn zoom_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().diagram;
        let scale = self.shown.as_ref().map_or(1., |shown| shown.view.scale);
        floating(&theme)
            .id("diagram-zoom")
            .right(px(12.))
            .bottom(px(12.))
            .child(
                icon_button(&theme, "zoom-out", "minus", true, false)
                    .tooltip(Tooltip::text(m.zoom_out))
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_by(1. / ZOOM_STEP, cx))),
            )
            .child(
                div()
                    .min_w(px(40.))
                    .flex()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(theme.text2)
                    .child(format!("{}%", (scale * 100.).round() as i64)),
            )
            .child(
                icon_button(&theme, "zoom-in", "plus", true, false)
                    .tooltip(Tooltip::text(m.zoom_in))
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_by(ZOOM_STEP, cx))),
            )
            .child(
                button(&theme, None, m.fit)
                    .id("fit")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.fit(cx))),
            )
            .into_any_element()
    }

    fn message(text: String, tint: gpui::Hsla) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .px(px(24.))
            .text_size(px(12.5))
            .text_color(tint)
            .child(text)
            .into_any_element()
    }
}

/// A panel floating over the canvas, which the canvas's pointer handling never sees.
fn floating(theme: &Theme) -> gpui::Div {
    glass(theme).absolute().rounded(px(20.)).p(px(4.)).flex().items_center().gap(px(6.)).occlude()
}

/// The cards of the tables shown, with the columns the mode lists, not yet placed.
fn build_cards(shown: &Shown) -> Vec<Card> {
    shown
        .diagram
        .tables
        .iter()
        .filter(|table| shown.visible.contains(&table.name))
        .map(|table| {
            let columns: Vec<CardColumn> = shown_columns(table, shown.mode, &shown.foreign_keys)
                .into_iter()
                .map(|column| {
                    let foreign = shown.foreign_keys.contains(&format!("{}.{}", table.name, column.name));
                    let key = if column.primary_key {
                        Some(ColumnKey::Primary)
                    } else if foreign {
                        Some(ColumnKey::Foreign)
                    } else {
                        None
                    };
                    CardColumn { name: column.name.clone(), data_type: short_type_name(&column.data_type), key }
                })
                .collect();
            Card {
                name: table.name.clone(),
                rows: table.row_estimate.map(compact),
                height: card_height(columns.len()),
                columns,
                position: Point::default(),
            }
        })
        .collect()
}

/// Marks the active table (dragged, hovered or pinned) and those tied to it.
fn highlight(scene: &mut Scene, shown: &Shown) {
    let dragged = match &shown.drag {
        Some(Drag::Card { name, .. }) => Some(name.clone()),
        _ => None,
    };
    scene.active = dragged.or_else(|| shown.hovered.clone()).or_else(|| shown.pinned.clone());
    scene.pinned = shown.pinned.clone();
    scene.related.clear();
    if let Some(active) = &scene.active {
        scene.related.insert(active.clone());
        for line in &scene.lines {
            if &line.table == active {
                scene.related.insert(line.referenced.clone());
            }
            if &line.referenced == active {
                scene.related.insert(line.table.clone());
            }
        }
    }
}

impl Render for DiagramView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().diagram;
        let Some(shown) = &self.shown else {
            return match &self.error {
                Some(error) => Self::message(error.clone(), theme.err),
                None => Self::message((m.loading)(&self.schema), theme.text3),
            };
        };
        if let Some(error) = &self.error {
            return Self::message(error.clone(), theme.err);
        }
        if shown.diagram.tables.is_empty() {
            return Self::message((m.no_tables)(&self.schema), theme.text3);
        }
        if let Some(focus) = &self.focus
            && !shown.diagram.tables.iter().any(|table| &table.name == focus)
        {
            return Self::message((m.is_view)(focus), theme.text3);
        }
        let canvas = div()
            .flex_1()
            .min_w_0()
            .relative()
            .overflow_hidden()
            .child(DiagramCanvas::new(cx.entity()))
            .child(self.toolbar(cx))
            .child(self.zoom_controls(cx));
        div()
            .size_full()
            .flex()
            .when(self.focus.is_none(), |diagram| diagram.child(self.sidebar(cx)))
            .child(canvas)
            .into_any_element()
    }
}
