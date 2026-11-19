//! A table's or view's tab: its actions on the left of a bar and its views
//! on the right (the rows, each section of its structure with its count,
//! and for relational engines its diagram), and the view shown under them.
//! The structure is read once for the whole tab: the sections show it, and
//! the rows take their footer, editors and links from it. The rows stay
//! loaded while another view shows, so their page and scroll survive a look
//! at the structure.

use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::{ObjectKind, RowFilter, TableRef};
use gpui::{
    AnyElement, AnyView, AppContext, Context, Entity, EventEmitter, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};

use super::view::{TableEvent, TableView};
use crate::connections::engine::is_relational;
use crate::diagram::{DiagramEvent, DiagramView};
use crate::i18n::Translate;
use crate::structure::sections::Section;
use crate::structure::{StructureSource, StructureView};
use crate::theme::ActiveTheme;
use crate::ui::field::Tooltip;
use crate::ui::{any, card, group, icon_button, segment};

/// What a table's tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableTabView {
    Data,
    Section(Section),
    Diagram,
}

/// What a table's tab asks of the workspace.
#[derive(Debug, Clone)]
pub enum TablePageEvent {
    /// Open this table of the same connection, filtered (a foreign key followed).
    OpenTable { table: TableRef, filter: RowFilter },
    /// Open the diagram of the table's whole schema.
    OpenSchemaDiagram { database: String, schema: String },
    /// Show this dialog over the window, or close the one shown.
    Dialog(Option<AnyView>),
    /// Export the rows `filter` keeps.
    Export(RowFilter),
    /// Import a file's rows into the table; its rows here are read again once they are in.
    Import(gpui::WeakEntity<TableView>),
}

pub struct TablePage {
    connection_id: String,
    engine: Engine,
    table: TableRef,
    view: TableTabView,
    source: Entity<StructureSource>,
    data: Entity<TableView>,
    /// The structure's sections, made when one is first shown.
    structure: Option<Entity<StructureView>>,
    /// The table's diagram, made when first shown.
    diagram: Option<(Entity<DiagramView>, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TablePageEvent> for TablePage {}

impl TablePage {
    pub fn new(
        connection_id: String,
        engine: Engine,
        table: TableRef,
        filter: RowFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Read up front: the section counts and the rows' footer come from it.
        let source = cx.new(|cx| StructureSource::new(connection_id.clone(), table.clone(), cx));
        let data = cx
            .new(|cx| TableView::new(connection_id.clone(), engine, table.clone(), filter, source.clone(), window, cx));
        let subscriptions = vec![
            // The bar's actions follow the rows: what is selected, what is pending.
            cx.observe(&data, |_, _, cx| cx.notify()),
            // The counts follow the structure.
            cx.observe(&source, |_, _, cx| cx.notify()),
            cx.subscribe(&data, |_, _, event: &TableEvent, cx| {
                cx.emit(match event.clone() {
                    TableEvent::OpenTable { table, filter } => TablePageEvent::OpenTable { table, filter },
                    TableEvent::Dialog(dialog) => TablePageEvent::Dialog(dialog),
                });
            }),
        ];
        Self {
            connection_id,
            engine,
            table,
            view: TableTabView::Data,
            source,
            data,
            structure: None,
            diagram: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn data(&self) -> &Entity<TableView> {
        &self.data
    }

    pub fn source(&self) -> &Entity<StructureSource> {
        &self.source
    }

    pub fn view(&self) -> TableTabView {
        self.view
    }

    pub fn structure_view(&self) -> Option<&Entity<StructureView>> {
        self.structure.as_ref()
    }

    pub fn diagram_view(&self) -> Option<&Entity<DiagramView>> {
        self.diagram.as_ref().map(|(diagram, _)| diagram)
    }

    /// Relational engines draw the table with its relations.
    pub fn has_diagram(&self) -> bool {
        is_relational(self.engine)
    }

    /// Shows `view`; a table without a diagram stays where it is.
    pub fn set_view(&mut self, view: TableTabView, window: &mut Window, cx: &mut Context<Self>) {
        match view {
            TableTabView::Diagram if !self.has_diagram() => return,
            TableTabView::Diagram if self.diagram.is_none() => {
                let (connection, table) = (self.connection_id.clone(), self.table.clone());
                let diagram = cx
                    .new(|cx| DiagramView::new(connection, table.database, table.schema, Some(table.name), window, cx));
                let events = cx.subscribe(&diagram, |this, _, event: &DiagramEvent, cx| match event {
                    DiagramEvent::OpenSchema => cx.emit(TablePageEvent::OpenSchemaDiagram {
                        database: this.table.database.clone(),
                        schema: this.table.schema.clone(),
                    }),
                });
                self.diagram = Some((diagram, events));
            }
            TableTabView::Section(section) => match &self.structure {
                Some(structure) => structure.update(cx, |structure, cx| structure.set_section(section, cx)),
                None => {
                    let (source, engine) = (self.source.clone(), self.engine);
                    self.structure = Some(cx.new(|cx| StructureView::new(source, engine, section, cx)));
                }
            },
            _ => {}
        }
        self.view = view;
        if view == TableTabView::Data {
            self.data.read(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// The rows take the keyboard when they show.
    pub fn focus(&self, window: &mut Window, cx: &gpui::App) {
        if self.view == TableTabView::Data {
            self.data.read(cx).focus(window, cx);
        }
    }

    fn view_tab(&self, view: TableTabView, label: &str, count: Option<usize>, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let id = match view {
            TableTabView::Data => "view-data".into(),
            TableTabView::Diagram => "view-diagram".into(),
            TableTabView::Section(section) => format!("view-{section:?}"),
        };
        any(segment(&theme, None, label, self.view == view)
            .id(gpui::SharedString::from(id))
            .cursor_pointer()
            .when_some(count, |tab, count| tab.child(div().text_color(theme.text3).child(count.to_string())))
            .on_click(cx.listener(move |this, _, window, cx| this.set_view(view, window, cx))))
    }

    fn bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let messages = cx.messages();
        let structure = self.source.read(cx).structure().cloned();
        let loading = self.source.read(cx).is_loading();
        // The rows' actions while they show; the structure's reload while a section does.
        let actions = match self.view {
            TableTabView::Data => self.data.update(cx, |data, cx| data.actions(cx)),
            TableTabView::Section(_) => {
                let reload = icon_button(&theme, "reload-structure", "refresh", !loading, false)
                    .tooltip(Tooltip::text(messages.structure.reload_structure))
                    .when(!loading, |reload| {
                        reload.on_click(cx.listener(|this, _, _, cx| {
                            this.source.update(cx, |source, cx| source.reload(cx));
                        }))
                    });
                vec![reload.into_any_element()]
            }
            TableTabView::Diagram => Vec::new(),
        };
        // The table's own actions after the view's: export, and import into a table (not a view).
        let mut actions = actions;
        let export = icon_button(&theme, "export-rows", "export", true, false)
            .tooltip(Tooltip::text(messages.table.export_rows))
            .on_click(cx.listener(|this, _, _, cx| {
                let filter = this.data.read(cx).filter().clone();
                cx.emit(TablePageEvent::Export(filter));
            }));
        let insert_at = if matches!(self.view, TableTabView::Section(_)) { actions.len() - 1 } else { actions.len() };
        let mut own = vec![export.into_any_element()];
        if structure.as_ref().is_none_or(|structure| structure.kind != ObjectKind::View) {
            let import = icon_button(&theme, "import-rows", "upload", true, false)
                .tooltip(Tooltip::text(messages.table.import_rows))
                .on_click(cx.listener(|this, _, _, cx| cx.emit(TablePageEvent::Import(this.data.downgrade()))));
            own.push(import.into_any_element());
        }
        actions.splice(insert_at..insert_at, own);
        let mut tabs = vec![self.view_tab(TableTabView::Data, messages.table.data, None, cx)];
        for section in Section::ALL {
            let count = structure.as_ref().and_then(|structure| section.count(structure));
            tabs.push(self.view_tab(TableTabView::Section(section), section.label(&messages.structure), count, cx));
        }
        if self.has_diagram() {
            tabs.push(self.view_tab(TableTabView::Diagram, messages.structure.diagram, None, cx));
        }
        div()
            .h(px(34.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .children(actions)
            .child(div().flex_1())
            .child(group(&theme, tabs))
            .into_any_element()
    }
}

impl Render for TablePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let content: AnyElement = match (self.view, &self.structure, &self.diagram) {
            (TableTabView::Section(_), Some(structure), _) => structure.clone().into_any_element(),
            (TableTabView::Diagram, _, Some((diagram, _))) => diagram.clone().into_any_element(),
            _ => self.data.clone().into_any_element(),
        };
        div().flex_1().min_h_0().flex().flex_col().gap(px(10.)).child(self.bar(cx)).child(card(&theme).child(content))
    }
}
