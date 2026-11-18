//! What a column's type says about drawing it: numbers align right, long
//! server type names shorten for the header, and a first width from what
//! the column holds.

use std::sync::OnceLock;

use regex::Regex;

/// The narrowest and widest a column starts at, and the narrowest it can be dragged to.
pub const MIN_WIDTH: f32 = 64.;
pub const MAX_WIDTH: f32 = 340.;
pub const MIN_DRAG_WIDTH: f32 = 40.;

/// Room around a cell's text.
pub const CELL_PADDING: f32 = 12.;

/// Numbers read better right-aligned. Long names from table metadata and
/// short ones from query results (int4, float8, …), plus MySQL's and SQL
/// Server's spellings.
pub fn is_numeric_type(data_type: &str) -> bool {
    static NUMERIC: OnceLock<Regex> = OnceLock::new();
    NUMERIC
        .get_or_init(|| {
            Regex::new(
                r"^(tinyint|smallint|mediumint|int|integer|bigint|numeric|decimal|real|float|double|double precision|money|smallmoney|int2|int4|int8|float4|float8|oid)\b",
            )
            .expect("a valid pattern")
        })
        .is_match(data_type)
}

/// Shortens long server type names for the header: `character varying(80)` → `varchar(80)`.
pub fn short_type_name(data_type: &str) -> String {
    static RULES: OnceLock<Vec<(Regex, &str)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        [
            (r"^character varying", "varchar"),
            (r"^timestamp(\(\d\))? with time zone", "timestamptz$1"),
            (r"^timestamp(\(\d\))? without time zone", "timestamp$1"),
            (r"^double precision", "float8"),
            (r"^character\b", "char"),
        ]
        .into_iter()
        .map(|(pattern, short)| (Regex::new(pattern).expect("a valid pattern"), short))
        .collect()
    });
    rules.iter().fold(data_type.to_owned(), |name, (pattern, short)| pattern.replace(&name, *short).into_owned())
}

/// A column's first width, in pixels, for `char_width`-wide characters:
/// enough for its header and the longest of `values`, within limits.
pub fn initial_width<'a>(
    name: &str,
    data_type: &str,
    values: impl Iterator<Item = Option<&'a str>>,
    char_width: f32,
) -> f32 {
    let header = name.chars().count() + short_type_name(data_type).chars().count() + 2;
    let widest = values.map(|value| value.map_or(4, |value| value.chars().count())).max().unwrap_or(0);
    let chars = header.max(widest) as f32;
    (chars * char_width + CELL_PADDING * 2.).clamp(MIN_WIDTH, MAX_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_numeric_types() {
        for numeric in [
            "integer",
            "bigint",
            "numeric(10,2)",
            "double precision",
            "real",
            "int4",
            "float8",
            "int",
            "tinyint",
            "float",
            "smallmoney",
        ] {
            assert!(is_numeric_type(numeric), "{numeric}");
        }
        for other in ["text", "character varying(20)", "uuid", "text[]", "jsonb", "interval", "nvarchar(40)", "bit"] {
            assert!(!is_numeric_type(other), "{other}");
        }
    }

    #[test]
    fn shortens_long_type_names() {
        for (long, short) in [
            ("character varying(80)", "varchar(80)"),
            ("timestamp with time zone", "timestamptz"),
            ("timestamp(6) with time zone", "timestamptz(6)"),
            ("timestamp(3) without time zone", "timestamp(3)"),
            ("double precision", "float8"),
            ("character(2)", "char(2)"),
            ("integer", "integer"),
        ] {
            assert_eq!(short_type_name(long), short);
        }
    }

    #[test]
    fn starts_columns_as_wide_as_their_content_within_limits() {
        let narrow = initial_width("n", "", [Some("1"), Some("22")].into_iter(), 7.);
        assert_eq!(narrow, MIN_WIDTH);
        let wide = initial_width("note", "text", [Some("x".repeat(200).as_str())].into_iter(), 7.);
        assert_eq!(wide, MAX_WIDTH);
        let name = initial_width("customer_name", "varchar", [None, Some("Ada")].into_iter(), 7.);
        assert_eq!(name, (13 + 7 + 2) as f32 * 7. + 24.);
    }
}
