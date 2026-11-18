//! Dates and times as the editors show them: the kind a column holds, its
//! text read into parts (day, month, year, hour…) and written back in the
//! server's own form.

use std::sync::OnceLock;

use chrono::{Datelike, Local, Timelike};
use regex::Regex;

/// The kinds of date and time a column can hold, as the editor offers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporalKind {
    Date,
    Time,
    Timestamp,
    TimestampTz,
}

/// A date and time split into the parts the editor's controls show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Temporal {
    pub year: u32,
    /// 1–12.
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// Digits after the decimal point, kept as typed (empty for none).
    pub fraction: String,
    /// Minutes east of UTC; only a `TimestampTz` has one.
    pub offset: Option<i32>,
}

/// The kind of a column from its type as the server names it: PostgreSQL's
/// `timestamp(3) with time zone`, MySQL's `datetime(6)`, SQL Server's
/// `datetimeoffset(7)`… `None` for anything else (SQLite keeps dates as text).
pub fn temporal_kind_of(data_type: &str) -> Option<TemporalKind> {
    static PRECISION: OnceLock<Regex> = OnceLock::new();
    let precision = PRECISION.get_or_init(|| Regex::new(r"\(\d+\)").expect("a valid pattern"));
    let lower = data_type.to_lowercase();
    match precision.replace_all(&lower, "").trim() {
        "date" => Some(TemporalKind::Date),
        "time" | "time without time zone" => Some(TemporalKind::Time),
        "timestamptz" | "timestamp with time zone" | "datetimeoffset" => Some(TemporalKind::TimestampTz),
        "timestamp" | "timestamp without time zone" | "datetime" | "datetime2" | "smalldatetime" => {
            Some(TemporalKind::Timestamp)
        }
        _ => None,
    }
}

/// Whether a column of `data_type` writes its offset after a space, as SQL Server does.
pub fn offset_space(data_type: &str) -> bool {
    data_type.to_lowercase().contains("datetimeoffset")
}

fn pattern(kind: TemporalKind) -> &'static Regex {
    const DATE: &str = r"(\d{4})-(\d{2})-(\d{2})";
    const TIME: &str = r"(\d{2}):(\d{2})(?::(\d{2})(?:\.(\d+))?)?";
    const OFFSET: &str = r"\s?(Z|[+-]\d{2}(?::?\d{2})?)";
    static PATTERNS: OnceLock<[Regex; 4]> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [format!("^{DATE}$"), format!("^{TIME}$"), format!("^{DATE}[ T]{TIME}$"), format!("^{DATE}[ T]{TIME}{OFFSET}$")]
            .map(|source| Regex::new(&source).expect("a valid pattern"))
    });
    &patterns[kind as usize]
}

pub fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => 29,
        2 => 28,
        _ => 31,
    }
}

/// The parts of `text` read as `kind`; `None` when it isn't a valid value of that kind.
pub fn parse_temporal(text: &str, kind: TemporalKind) -> Option<Temporal> {
    let captures = pattern(kind).captures(text.trim())?;
    let parts: Vec<Option<&str>> = captures.iter().skip(1).map(|part| part.map(|part| part.as_str())).collect();
    // A missing part (the seconds of `10:00`) is 0.
    let number = |index: usize| parts[index].map_or(Some(0), |part| part.parse::<u32>().ok());
    let (year, month, day) = match kind {
        TemporalKind::Time => (2000, 1, 1),
        _ => (number(0)?, number(1)?, number(2)?),
    };
    let time = if kind == TemporalKind::Time { 0 } else { 3 };
    let (hour, minute, second, fraction) = match kind {
        TemporalKind::Date => (0, 0, 0, ""),
        _ => (number(time)?, number(time + 1)?, number(time + 2)?, parts[time + 3].unwrap_or("")),
    };
    let offset = match kind {
        TemporalKind::TimestampTz => Some(offset_minutes(parts[7]?)),
        _ => None,
    };
    let fraction = fraction.to_owned();
    let value = Temporal { year, month, day, hour, minute, second, fraction, offset };
    is_valid(&value).then_some(value)
}

fn offset_minutes(text: &str) -> i32 {
    if text == "Z" {
        return 0;
    }
    let sign = if text.starts_with('-') { -1 } else { 1 };
    let digits = text[1..].replace(':', "");
    let hours: i32 = digits.get(..2).and_then(|h| h.parse().ok()).unwrap_or(0);
    let minutes: i32 = digits.get(2..).filter(|m| !m.is_empty()).and_then(|m| m.parse().ok()).unwrap_or(0);
    sign * (hours * 60 + minutes)
}

fn is_valid(value: &Temporal) -> bool {
    value.year >= 1
        && (1..=12).contains(&value.month)
        && value.day >= 1
        && value.day <= days_in_month(value.year, value.month)
        && value.hour < 24
        && value.minute < 60
        && value.second < 60
        && value.offset.is_none_or(|offset| offset.abs() < 24 * 60)
}

/// The value as the server reads it back: `YYYY-MM-DD`, `HH:MM:SS[.f]`, or
/// both with a space between, plus `+HH:MM` for a `TimestampTz` (after a
/// space when `offset_space`, as SQL Server writes it).
pub fn format_temporal(value: &Temporal, kind: TemporalKind, offset_space: bool) -> String {
    let date = format!("{:04}-{:02}-{:02}", value.year, value.month, value.day);
    let fraction = if value.fraction.is_empty() { String::new() } else { format!(".{}", value.fraction) };
    let time = format!("{:02}:{:02}:{:02}{fraction}", value.hour, value.minute, value.second);
    match kind {
        TemporalKind::Date => date,
        TemporalKind::Time => time,
        TemporalKind::Timestamp => format!("{date} {time}"),
        TemporalKind::TimestampTz => {
            let offset = value.offset.unwrap_or(0);
            let sign = if offset < 0 { '-' } else { '+' };
            let zone = format!("{sign}{:02}:{:02}", offset.abs() / 60, offset.abs() % 60);
            format!("{date} {time}{}{zone}", if offset_space { " " } else { "" })
        }
    }
}

/// This computer's clock now, with its offset.
pub fn now() -> Temporal {
    let now = Local::now();
    Temporal {
        year: now.year().max(1) as u32,
        month: now.month(),
        day: now.day(),
        hour: now.hour(),
        minute: now.minute(),
        second: now.second(),
        fraction: String::new(),
        offset: Some(now.offset().local_minus_utc() / 60),
    }
}

/// `value` with the day pulled back into a shorter month (31 March → February keeps the 28th or 29th).
pub fn clamp_day(mut value: Temporal) -> Temporal {
    value.day = value.day.min(days_in_month(value.year, value.month));
    value
}

/// Now, as a cell of `kind` takes it: without a time zone the value keeps `offset`, the column's own.
pub fn now_for(kind: TemporalKind, offset: Option<i32>) -> Temporal {
    let current = now();
    if kind == TemporalKind::TimestampTz { current } else { Temporal { offset, ..current } }
}

/// A value of `kind` being edited: the typed text and the parts the
/// controls show, kept in step. A NULL value starts the controls at now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalDraft {
    pub kind: TemporalKind,
    pub offset_space: bool,
    pub text: String,
    /// The last valid value, which the controls show.
    pub parts: Temporal,
}

impl TemporalDraft {
    pub fn new(initial: Option<&str>, kind: TemporalKind, offset_space: bool) -> Self {
        let text = initial.unwrap_or("").to_owned();
        let parts = parse_temporal(&text, kind).unwrap_or_else(now);
        Self { kind, offset_space, text, parts }
    }

    /// Empty, or a valid value of the kind.
    pub fn is_valid(&self) -> bool {
        self.text.is_empty() || parse_temporal(&self.text, self.kind).is_some()
    }

    /// The value in the server's own form; `None` while the text isn't one.
    pub fn value(&self) -> Option<String> {
        parse_temporal(&self.text, self.kind).map(|parsed| format_temporal(&parsed, self.kind, self.offset_space))
    }

    /// The user typed `text`; valid text moves the controls.
    pub fn type_text(&mut self, text: &str) {
        self.text = text.to_owned();
        if let Some(parsed) = parse_temporal(text, self.kind) {
            self.parts = parsed;
        }
    }

    /// A control changed the parts: the text follows.
    pub fn change(&mut self, change: impl FnOnce(&mut Temporal)) {
        let mut parts = self.parts.clone();
        change(&mut parts);
        self.show(clamp_day(parts));
    }

    /// This computer's clock.
    pub fn now(&mut self) {
        let next = now_for(self.kind, self.parts.offset);
        self.show(next);
    }

    fn show(&mut self, parts: Temporal) {
        self.text = format_temporal(&parts, self.kind, self.offset_space);
        self.parts = parts;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TemporalKind::*;

    #[test]
    fn knows_each_engine_s_date_and_time_types() {
        assert_eq!(temporal_kind_of("date"), Some(Date));
        assert_eq!(temporal_kind_of("time without time zone"), Some(Time));
        assert_eq!(temporal_kind_of("time(6)"), Some(Time));
        assert_eq!(temporal_kind_of("timestamp(3) with time zone"), Some(TimestampTz));
        assert_eq!(temporal_kind_of("timestamp without time zone"), Some(Timestamp));
        assert_eq!(temporal_kind_of("datetime(6)"), Some(Timestamp));
        assert_eq!(temporal_kind_of("datetime2(7)"), Some(Timestamp));
        assert_eq!(temporal_kind_of("datetimeoffset(7)"), Some(TimestampTz));
        assert_eq!(temporal_kind_of("text"), None);
        assert_eq!(temporal_kind_of("time with time zone"), None);
    }

    fn round_trip(text: &str, kind: TemporalKind, space: bool) -> String {
        format_temporal(&parse_temporal(text, kind).expect("a valid value"), kind, space)
    }

    #[test]
    fn reads_each_engine_s_text_and_writes_it_back_the_same_way() {
        let pg = "2026-09-16 14:00:27.123456+05:30";
        assert_eq!(round_trip(pg, TimestampTz, false), pg);
        assert_eq!(parse_temporal("2026-11-05 10:00:00+00", TimestampTz).unwrap().offset, Some(0));
        assert_eq!(parse_temporal("2026-11-05 10:00:00-03", TimestampTz).unwrap().offset, Some(-180));
        let mssql = "2026-11-05 10:00:00.1234567 +00:00";
        assert_eq!(round_trip(mssql, TimestampTz, true), mssql);
        assert_eq!(round_trip("2026-02-03T04:05", Timestamp, false), "2026-02-03 04:05:00");
        assert_eq!(round_trip("2026-02-03", Date, false), "2026-02-03");
        assert_eq!(round_trip("09:30:00.5", Time, false), "09:30:00.5");
    }

    #[test]
    fn refuses_what_isn_t_a_real_date_or_time() {
        assert_eq!(parse_temporal("2026-02-30", Date), None);
        assert!(parse_temporal("2028-02-29", Date).is_some());
        assert_eq!(parse_temporal("24:00:00", Time), None);
        assert_eq!(parse_temporal("2026-01-01 10:00:00", TimestampTz), None);
        assert_eq!(parse_temporal("yesterday", Timestamp), None);
    }

    #[test]
    fn pulls_the_day_back_into_a_shorter_month() {
        let march31 = parse_temporal("2026-03-31", Date).unwrap();
        assert_eq!(clamp_day(Temporal { month: 2, ..march31.clone() }).day, 28);
        assert_eq!(clamp_day(Temporal { month: 2, year: 2028, ..march31 }).day, 29);
    }

    #[test]
    fn takes_now_from_this_computer_s_clock_keeping_a_column_s_own_offset() {
        let current = now();
        assert!(is_valid(&current));
        assert_eq!(now_for(Timestamp, Some(90)).offset, Some(90));
        assert_eq!(now_for(TimestampTz, Some(90)).offset, current.offset);
    }

    #[test]
    fn keeps_the_typed_text_and_the_controls_in_step() {
        let mut draft = TemporalDraft::new(Some("2026-03-31 10:00:00"), Timestamp, false);
        draft.change(|parts| parts.month = 2);
        assert_eq!(draft.text, "2026-02-28 10:00:00");
        draft.type_text("2026-02-2");
        assert!(!draft.is_valid());
        assert_eq!(draft.value(), None);
        assert_eq!(draft.parts.day, 28, "the controls keep the last valid value");
        draft.type_text("2026-02-03T04:05");
        assert_eq!(draft.value().as_deref(), Some("2026-02-03 04:05:00"));
        draft.type_text("");
        assert!(draft.is_valid(), "empty is valid");
    }
}
