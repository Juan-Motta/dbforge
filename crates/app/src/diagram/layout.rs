//! Where an ER diagram's cards go and how the view moves over them, apart
//! from drawing: which tables and relationships are shown, the columns a
//! card lists, the automatic layout (referenced tables on the left), the
//! curve of each relationship, and fitting, zooming and centring the view.
//! Coordinates are the diagram's own, in pixels at 100%.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use dbforge_core::drivers::{DiagramColumn, DiagramTable, Relationship, SchemaDiagram};

pub const CARD_WIDTH: f32 = 240.;
pub const HEADER_HEIGHT: f32 = 32.;
pub const ROW_HEIGHT: f32 = 24.;
const GAP_X: f32 = 120.;
const GAP_Y: f32 = 32.;
/// Past this many tables, start with only the related ones so the diagram stays readable.
const SHOW_ALL_LIMIT: usize = 40;

/// The columns a card lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnMode {
    All,
    Keys,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// What part of the diagram the viewport shows: `scale` times the
/// diagram, its origin at (`x`, `y`) in viewport pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
}

impl Default for View {
    fn default() -> Self {
        Self { x: 40., y: 40., scale: 1. }
    }
}

impl View {
    /// The diagram point under viewport point `at`.
    pub fn to_diagram(&self, at: Point) -> Point {
        Point::new((at.x - self.x) / self.scale, (at.y - self.y) / self.scale)
    }

    /// Where diagram point `at` is in the viewport.
    pub fn to_viewport(&self, at: Point) -> Point {
        Point::new(at.x * self.scale + self.x, at.y * self.scale + self.y)
    }
}

/// A viewport's size, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

/// Relationships inside the schema; those pointing elsewhere have no card to reach.
pub fn local_relationships(diagram: &SchemaDiagram, schema: &str) -> Vec<Relationship> {
    let names: HashSet<&str> = diagram.tables.iter().map(|table| table.name.as_str()).collect();
    diagram
        .relationships
        .iter()
        .filter(|r| r.referenced_schema == schema && names.contains(r.referenced_table.as_str()))
        .cloned()
        .collect()
}

/// Every table when there are few; otherwise only the ones that take part in a relationship.
pub fn default_selection(diagram: &SchemaDiagram, relationships: &[Relationship]) -> HashSet<String> {
    if diagram.tables.len() <= SHOW_ALL_LIMIT {
        return diagram.tables.iter().map(|table| table.name.clone()).collect();
    }
    relationships.iter().flat_map(|r| [r.table.clone(), r.referenced_table.clone()]).collect()
}

/// Every `table.column` that is a foreign key, wherever it points.
pub fn foreign_key_columns(diagram: &SchemaDiagram) -> HashSet<String> {
    diagram.relationships.iter().flat_map(|r| r.columns.iter().map(move |c| format!("{}.{c}", r.table))).collect()
}

/// The columns a card shows: all of them, or only primary and foreign keys.
pub fn shown_columns<'a>(
    table: &'a DiagramTable,
    mode: ColumnMode,
    foreign_keys: &HashSet<String>,
) -> Vec<&'a DiagramColumn> {
    table
        .columns
        .iter()
        .filter(|c| {
            mode == ColumnMode::All || c.primary_key || foreign_keys.contains(&format!("{}.{}", table.name, c.name))
        })
        .collect()
}

pub fn card_height(rows: usize) -> f32 {
    HEADER_HEIGHT + rows as f32 * ROW_HEIGHT + 6.
}

/// A card to place: its table's name and height.
pub struct Placed<'a> {
    pub name: &'a str,
    pub height: f32,
}

/// Places tables in columns by dependency: a table goes one column right of
/// the furthest table it references, so referenced tables sit on the left
/// and lines mostly run right to left. Cycles and self-references are
/// ignored for ranking.
pub fn layout<'a>(tables: &[Placed<'a>], relationships: &[(&str, &str)]) -> HashMap<String, Point> {
    let names: HashSet<&str> = tables.iter().map(|table| table.name).collect();
    let mut references: HashMap<&str, Vec<&str>> = HashMap::new();
    for &(table, referenced) in relationships {
        if table == referenced || !names.contains(table) || !names.contains(referenced) {
            continue;
        }
        references.entry(table).or_default().push(referenced);
    }

    fn rank_of<'b>(
        name: &'b str,
        references: &HashMap<&'b str, Vec<&'b str>>,
        rank: &mut HashMap<&'b str, usize>,
        visiting: &mut HashSet<&'b str>,
    ) -> usize {
        if let Some(&known) = rank.get(name) {
            return known;
        }
        if !visiting.insert(name) {
            // A cycle: break it here.
            return 0;
        }
        let mut furthest: Option<usize> = None;
        for &referenced in references.get(name).into_iter().flatten() {
            let level = rank_of(referenced, references, rank, visiting);
            furthest = Some(furthest.map_or(level, |f| f.max(level)));
        }
        visiting.remove(name);
        let value = furthest.map_or(0, |f| f + 1);
        rank.insert(name, value);
        value
    }

    let mut rank = HashMap::new();
    let mut visiting = HashSet::new();
    let mut sorted: Vec<&Placed> = tables.iter().collect();
    sorted.sort_by(|a, b| compare_names(a.name, b.name));
    let mut levels: BTreeMap<usize, Vec<&Placed>> = BTreeMap::new();
    for table in sorted {
        let level = rank_of(table.name, &references, &mut rank, &mut visiting);
        levels.entry(level).or_default().push(table);
    }

    // A crowded level wraps into several columns so the diagram grows wider rather
    // than very tall: aim for a column height that makes the whole thing about 16:10.
    let area: f32 = tables.iter().map(|t| (t.height + GAP_Y) * (CARD_WIDTH + GAP_X)).sum();
    let max_height = tables.iter().map(|t| t.height).fold((area / 1.6).sqrt(), f32::max);

    let mut positions = HashMap::new();
    let mut column = 0.;
    for level in levels.values() {
        let mut y = 0.;
        for table in level {
            if y > 0. && y + table.height > max_height {
                column += 1.;
                y = 0.;
            }
            positions.insert(table.name.to_owned(), Point::new(column * (CARD_WIDTH + GAP_X), y));
            y += table.height + GAP_Y;
        }
        column += 1.;
    }
    positions
}

/// Names in the order the web UI sorted them (`localeCompare`): ignoring
/// case first, then lower case before upper.
fn compare_names(a: &str, b: &str) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| b.cmp(a))
}

/// A cubic curve: from `from` to `to`, pulled by `c1` and `c2`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve {
    pub from: Point,
    pub c1: Point,
    pub c2: Point,
    pub to: Point,
}

/// A curve from the right or left edge of one card's row to the facing edge
/// of another's. A row of -1 is the card's header.
pub fn relationship_path(from: Point, from_row: isize, to: Point, to_row: isize) -> Curve {
    let y1 = from.y + row_center(from_row);
    let y2 = to.y + row_center(to_row);
    if from == to {
        // A table referencing itself: loop out of the right edge and back.
        let x = from.x + CARD_WIDTH;
        return Curve {
            from: Point::new(x, y1),
            c1: Point::new(x + 50., y1),
            c2: Point::new(x + 50., y2),
            to: Point::new(x, y2),
        };
    }
    let leftward = to.x + CARD_WIDTH <= from.x;
    let x1 = if leftward { from.x } else { from.x + CARD_WIDTH };
    let x2 = if leftward { to.x + CARD_WIDTH } else { to.x };
    let bend = ((x2 - x1).abs() / 2.).max(40.) * if leftward { -1. } else { 1. };
    Curve {
        from: Point::new(x1, y1),
        c1: Point::new(x1 + bend, y1),
        c2: Point::new(x2 - bend, y2),
        to: Point::new(x2, y2),
    }
}

fn row_center(row: isize) -> f32 {
    if row < 0 { HEADER_HEIGHT / 2. } else { HEADER_HEIGHT + row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2. }
}

/// Room kept free around the cards: the toolbar sits on top, the zoom controls at the bottom.
const FIT_MARGIN_TOP: f32 = 64.;
const FIT_MARGIN_RIGHT: f32 = 32.;
const FIT_MARGIN_BOTTOM: f32 = 64.;
const FIT_MARGIN_LEFT: f32 = 32.;

/// A card's place and height, for fitting.
#[derive(Debug, Clone, Copy)]
pub struct CardBox {
    pub position: Point,
    pub height: f32,
}

/// The scale and offset that fit every card into a viewport, never zooming past 100%.
pub fn fit_view(cards: &[CardBox], viewport: Size) -> View {
    if cards.is_empty() {
        return View { x: FIT_MARGIN_LEFT, y: FIT_MARGIN_TOP, scale: 1. };
    }
    let left = cards.iter().map(|c| c.position.x).fold(f32::INFINITY, f32::min);
    let top = cards.iter().map(|c| c.position.y).fold(f32::INFINITY, f32::min);
    let right = cards.iter().map(|c| c.position.x + CARD_WIDTH).fold(f32::NEG_INFINITY, f32::max);
    let bottom = cards.iter().map(|c| c.position.y + c.height).fold(f32::NEG_INFINITY, f32::max);
    let fits_width = (viewport.width - FIT_MARGIN_LEFT - FIT_MARGIN_RIGHT) / (right - left);
    let fits_height = (viewport.height - FIT_MARGIN_TOP - FIT_MARGIN_BOTTOM) / (bottom - top);
    let scale = 1_f32.min(fits_width).min(fits_height).max(MIN_SCALE);
    View { x: FIT_MARGIN_LEFT - left * scale, y: FIT_MARGIN_TOP - top * scale, scale }
}

pub const MIN_SCALE: f32 = 0.1;
pub const MAX_SCALE: f32 = 2.;

/// Zooms by `factor` keeping the diagram point under `anchor` (viewport pixels) in place.
pub fn zoom_at(view: View, factor: f32, anchor: Point) -> View {
    let scale = (view.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
    let ratio = scale / view.scale;
    View { scale, x: anchor.x - (anchor.x - view.x) * ratio, y: anchor.y - (anchor.y - view.y) * ratio }
}

/// Centres one card in the viewport at `scale`, e.g. to read it after a double click.
pub fn center_on(position: Point, height: f32, viewport: Size, scale: f32) -> View {
    View {
        scale,
        x: viewport.width / 2. - (position.x + CARD_WIDTH / 2.) * scale,
        y: viewport.height / 2. - (position.y + height / 2.) * scale,
    }
}

/// `table` and every table within `steps` relations of it, following
/// foreign keys in either direction: what a table's own diagram shows.
pub fn neighborhood(relationships: &[Relationship], table: &str, steps: usize) -> HashSet<String> {
    let mut reached: HashSet<String> = HashSet::from([table.to_owned()]);
    let mut frontier = vec![table.to_owned()];
    for _ in 0..steps {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for r in relationships {
            for (from, to) in [(&r.table, &r.referenced_table), (&r.referenced_table, &r.table)] {
                if frontier.contains(from) && !reached.contains(to) {
                    reached.insert(to.clone());
                    next.push(to.clone());
                }
            }
        }
        frontier = next;
    }
    reached
}

/// A row estimate as the card's header shows it: `12K`, `3.4M`.
pub fn compact(n: i64) -> String {
    let value = n.unsigned_abs() as f64;
    let sign = if n < 0 { "-" } else { "" };
    let (scaled, suffix) = match value {
        v if v >= 1e12 => (v / 1e12, "T"),
        v if v >= 1e9 => (v / 1e9, "B"),
        v if v >= 1e6 => (v / 1e6, "M"),
        v if v >= 1e3 => (v / 1e3, "K"),
        v => return format!("{sign}{v}"),
    };
    // One decimal at most, dropped when it is zero; 999.95K rounds up to 1M.
    let rounded = (scaled * 10.).round() / 10.;
    if rounded >= 1000. && suffix != "T" {
        let next = match suffix {
            "K" => "M",
            "M" => "B",
            _ => "T",
        };
        return format!("{sign}1{next}");
    }
    if rounded.fract() == 0. { format!("{sign}{rounded:.0}{suffix}") } else { format!("{sign}{rounded:.1}{suffix}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(table: &str, referenced: &str, schema: &str) -> Relationship {
        Relationship {
            name: format!("{table}_{referenced}_fkey"),
            table: table.into(),
            columns: vec![format!("{referenced}_id")],
            referenced_schema: schema.into(),
            referenced_table: referenced.into(),
            referenced_columns: vec!["id".into()],
        }
    }

    fn column(name: &str, data_type: &str, nullable: bool, primary_key: bool) -> DiagramColumn {
        DiagramColumn { name: name.into(), data_type: data_type.into(), nullable, primary_key }
    }

    fn table(name: &str) -> DiagramTable {
        DiagramTable {
            name: name.into(),
            row_estimate: None,
            columns: vec![
                column("id", "integer", false, true),
                column("customers_id", "integer", true, false),
                column("note", "text", true, false),
            ],
        }
    }

    fn placed(names: &[&'static str], height: f32) -> Vec<Placed<'static>> {
        names.iter().map(|name| Placed { name, height }).collect()
    }

    #[test]
    fn puts_referenced_tables_left_of_the_tables_that_reference_them() {
        let cards = placed(&["order_items", "orders", "customers", "products"], 100.);
        let positions =
            layout(&cards, &[("orders", "customers"), ("order_items", "orders"), ("order_items", "products")]);
        let column_of = |name: &str| positions[name].x / (CARD_WIDTH + 120.);
        let columns: Vec<f32> = ["customers", "products", "orders", "order_items"].map(column_of).to_vec();
        assert_eq!(columns, [0., 0., 1., 2.]);
        // A column's tables stack by name.
        assert_eq!(positions["customers"].y, 0.);
        assert_eq!(positions["products"].y, 100. + 32.);
    }

    #[test]
    fn wraps_a_crowded_level_into_several_columns_keeping_levels_left_to_right() {
        // Like a real schema: one table everything references, and 30 tables that reference it.
        let names: Vec<String> = (0..30).map(|i| format!("t{i:02}")).collect();
        let mut cards = vec![Placed { name: "accounts", height: 150. }];
        cards.extend(names.iter().map(|name| Placed { name, height: 150. }));
        let relationships: Vec<(&str, &str)> = names.iter().map(|name| (name.as_str(), "accounts")).collect();
        let wrapped = layout(&cards, &relationships);
        let xs: HashSet<i64> = names.iter().map(|name| wrapped[name].x as i64).collect();
        let tallest = names.iter().map(|name| wrapped[name].y).fold(0., f32::max) + 150.;
        assert!(xs.len() > 1);
        assert!(*xs.iter().min().unwrap() as f32 > wrapped["accounts"].x);
        assert!(tallest < 30. * 150.);
    }

    #[test]
    fn survives_cycles_and_self_references() {
        let cyclic = layout(&placed(&["a", "b"], 50.), &[("a", "b"), ("b", "a"), ("a", "a")]);
        assert_eq!(cyclic.len(), 2);
    }

    #[test]
    fn keeps_only_relationships_whose_target_has_a_card() {
        let diagram = SchemaDiagram {
            tables: vec![table("orders"), table("customers")],
            relationships: vec![rel("orders", "customers", "public"), rel("orders", "currencies", "billing")],
        };
        let local: Vec<String> =
            local_relationships(&diagram, "public").into_iter().map(|r| r.referenced_table).collect();
        assert_eq!(local, ["customers"]);
    }

    #[test]
    fn shows_every_table_of_a_small_schema() {
        let diagram = SchemaDiagram { tables: vec![table("a"), table("b")], relationships: Vec::new() };
        assert_eq!(default_selection(&diagram, &[]), HashSet::from(["a".to_owned(), "b".to_owned()]));
    }

    #[test]
    fn starts_a_big_schema_with_only_its_related_tables() {
        let tables = (0..50).map(|i| table(&format!("t{i}"))).collect();
        let relationships = vec![rel("t1", "t2", "public")];
        let diagram = SchemaDiagram { tables, relationships: relationships.clone() };
        assert_eq!(default_selection(&diagram, &relationships), HashSet::from(["t1".to_owned(), "t2".to_owned()]));
    }

    #[test]
    fn can_show_only_key_columns() {
        let orders = table("orders");
        let keys = HashSet::from(["orders.customers_id".to_owned()]);
        let names: Vec<&str> =
            shown_columns(&orders, ColumnMode::Keys, &keys).iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "customers_id"]);
        assert_eq!(shown_columns(&orders, ColumnMode::All, &keys).len(), 3);
        let diagram = SchemaDiagram { tables: vec![orders], relationships: vec![rel("orders", "customers", "x")] };
        assert_eq!(foreign_key_columns(&diagram), HashSet::from(["orders.customers_id".to_owned()]));
    }

    #[test]
    fn runs_from_the_left_edge_of_the_referencing_card_to_the_right_edge_of_a_card_on_its_left() {
        let curve = relationship_path(Point::new(360., 0.), 1, Point::new(0., 0.), 0);
        assert_eq!(curve.from, Point::new(360., 32. + 24. + 12.));
        assert_eq!(curve.to, Point::new(CARD_WIDTH, 32. + 12.));
        // Half the gap, bending towards each card.
        assert_eq!(curve.c1, Point::new(300., 68.));
        assert_eq!(curve.c2, Point::new(300., 44.));
    }

    #[test]
    fn bends_at_least_40_pixels_and_aims_at_the_header_for_a_hidden_row() {
        let curve = relationship_path(Point::new(0., 0.), -1, Point::new(260., 0.), 0);
        assert_eq!(curve.from, Point::new(CARD_WIDTH, 16.));
        assert_eq!(curve.c1, Point::new(CARD_WIDTH + 40., 16.));
        assert_eq!(curve.c2, Point::new(220., 44.));
    }

    #[test]
    fn loops_a_self_reference_out_of_the_right_edge() {
        let curve = relationship_path(Point::new(0., 0.), 1, Point::new(0., 0.), 0);
        assert_eq!((curve.from, curve.c1), (Point::new(240., 68.), Point::new(290., 68.)));
        assert_eq!((curve.c2, curve.to), (Point::new(290., 44.), Point::new(240., 44.)));
    }

    #[test]
    fn scales_down_to_fit_and_never_zooms_in_past_100_percent() {
        let cards = [
            CardBox { position: Point::new(0., 0.), height: card_height(3) },
            CardBox { position: Point::new(2000., 0.), height: card_height(3) },
        ];
        let viewport = Size { width: 1000., height: 800. };
        assert!((fit_view(&cards, viewport).scale - (1000. - 64.) / 2240.).abs() < 1e-6);
        assert_eq!(fit_view(&cards[..1], viewport).scale, 1.);
    }

    #[test]
    fn leaves_the_top_free_for_the_toolbar() {
        let view =
            fit_view(&[CardBox { position: Point::new(0., 0.), height: 100. }], Size { width: 1000., height: 800. });
        assert_eq!(view, View { x: 32., y: 64., scale: 1. });
    }

    #[test]
    fn keeps_the_point_under_the_cursor_in_place_when_zooming() {
        let view = zoom_at(View { x: 100., y: 50., scale: 1. }, 2., Point::new(300., 250.));
        assert_eq!(view, View { x: -100., y: -150., scale: 2. });
        // The diagram point (200, 200) was at (300, 250) and still is.
        assert_eq!(view.to_viewport(Point::new(200., 200.)), Point::new(300., 250.));
        assert_eq!(view.to_diagram(Point::new(300., 250.)), Point::new(200., 200.));
    }

    #[test]
    fn zooms_between_10_and_200_percent() {
        let view = View { x: 0., y: 0., scale: 1. };
        assert_eq!(zoom_at(view, 10., Point::default()).scale, 2.);
        assert_eq!(zoom_at(view, 0.01, Point::default()).scale, 0.1);
    }

    #[test]
    fn puts_the_middle_of_the_card_in_the_middle_of_the_viewport() {
        let view = center_on(Point::new(400., 100.), 200., Size { width: 1000., height: 800. }, 1.);
        assert_eq!(view, View { scale: 1., x: 500. - (400. + CARD_WIDTH / 2.), y: 400. - 200. });
    }

    fn fk(table: &str, referenced: &str) -> Relationship {
        Relationship { columns: vec!["x".into()], ..rel(table, referenced, "public") }
    }

    #[test]
    fn follows_relations_both_ways_as_many_steps_as_asked() {
        // customers ← orders → stores; order_items → orders; order_items → products; products → vendors
        let relations = [
            fk("orders", "customers"),
            fk("orders", "stores"),
            fk("order_items", "orders"),
            fk("order_items", "products"),
            fk("products", "vendors"),
        ];
        let sorted = |set: HashSet<String>| {
            let mut names: Vec<String> = set.into_iter().collect();
            names.sort();
            names
        };
        assert_eq!(sorted(neighborhood(&relations, "orders", 1)), ["customers", "order_items", "orders", "stores"]);
        assert_eq!(
            sorted(neighborhood(&relations, "orders", 2)),
            ["customers", "order_items", "orders", "products", "stores"]
        );
        assert_eq!(sorted(neighborhood(&relations, "audit_log", 1)), ["audit_log"]);
    }

    #[test]
    fn writes_row_estimates_compactly() {
        let cases = [(12, "12"), (999, "999"), (1_000, "1K"), (12_345, "12.3K"), (48_000, "48K"), (999_960, "1M")];
        for (n, text) in cases {
            assert_eq!(compact(n), text, "{n}");
        }
        assert_eq!(compact(3_400_000), "3.4M");
        assert_eq!(compact(1_900_000), "1.9M");
    }
}
