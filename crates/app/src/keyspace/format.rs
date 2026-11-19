//! How the key browser writes and reads values: TTLs, sizes, the hit rate,
//! a value as JSON or hex, scores and `field=value` lines, and the
//! redis-cli commands it copies. A port of the former web UI's `keyspace.ts`.

use dbforge_core::drivers::redis::{Entry, KeyEdit, NewKey, Text};
use serde_json::{Number, Value};

use crate::i18n::KeyspaceMessages;

const UNITS: [(i64, &str); 4] = [(86_400_000, "d"), (3_600_000, "h"), (60_000, "m"), (1_000, "s")];

/// `23m`, or with `precise` `23m 41s`; `no ttl` for keys that never expire.
pub fn format_ttl(ms: Option<i64>, m: &KeyspaceMessages, precise: bool) -> String {
    let Some(ms) = ms else { return m.no_ttl.to_owned() };
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let index = UNITS.iter().position(|(size, _)| ms >= *size).unwrap_or(UNITS.len() - 1);
    let (size, unit) = UNITS[index];
    let whole = format!("{}{unit}", ms / size);
    match UNITS.get(index + 1) {
        Some((next, next_unit)) if precise => {
            let rest = (ms % size) / next;
            if rest > 0 { format!("{whole} {rest}{next_unit}") } else { whole }
        }
        _ => whole,
    }
}

/// `412 B`, `1.5 KB`, `412 MB`: one decimal only for a fraction under 100.
pub fn format_bytes(bytes: i64) -> String {
    const SIZES: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit < SIZES.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    let whole = unit == 0 || value >= 100. || value.fract() == 0.;
    format!("{} {}", to_fixed(value, if whole { 0 } else { 1 }), SIZES[unit])
}

/// Hits over lookups, `97.4%`; `—` before any lookup.
pub fn hit_rate(hits: i64, misses: i64) -> String {
    let total = hits + misses;
    if total == 0 { "—".to_owned() } else { format!("{}%", to_fixed(hits as f64 / total as f64 * 100., 1)) }
}

/// `value` with `digits` decimals as JavaScript's `toFixed` writes it: a
/// half rounds up (`2.5` → `3`), where Rust's formatting rounds it to even.
fn to_fixed(value: f64, digits: usize) -> String {
    let scale = 10f64.powi(digits as i32);
    format!("{:.digits$}", (value * scale).round() / scale)
}

/// The text pretty-printed when it is a JSON object or array, else `None`.
pub fn as_json(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return None;
    }
    let value: Value = serde_json::from_str(trimmed).ok()?;
    serde_json::to_string_pretty(&js_numbers(value)).ok()
}

/// Numbers as JavaScript writes them: a whole float without its `.0`.
pub(super) fn js_numbers(value: Value) -> Value {
    match value {
        Value::Number(number) => Value::Number(js_number(number)),
        Value::Array(items) => Value::Array(items.into_iter().map(js_numbers).collect()),
        Value::Object(fields) => {
            Value::Object(fields.into_iter().map(|(key, value)| (key, js_numbers(value))).collect())
        }
        other => other,
    }
}

fn js_number(number: Number) -> Number {
    match number.as_f64() {
        Some(float) if number.is_f64() && float.fract() == 0. && float.abs() < 9_007_199_254_740_992. => {
            Number::from(float as i64)
        }
        _ => number,
    }
}

/// The text's UTF-8 bytes in hex, space-separated.
pub fn as_hex(text: &str) -> String {
    text.bytes().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(" ")
}

/// A binary value's hex, which the core sends unspaced, in pairs.
pub fn spaced_hex(hex: &str) -> String {
    hex.as_bytes().chunks(2).map(|pair| String::from_utf8_lossy(pair).into_owned()).collect::<Vec<_>>().join(" ")
}

/// The label the browser shows for a type: `HASH`, `ZSET`, …
pub fn kind_label(kind: &str) -> String {
    kind.to_uppercase()
}

/// A number as JavaScript's `Number()` reads one, or `None`.
fn number(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(0.);
    }
    trimmed.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// A sorted set's score.
pub fn parse_score(text: &str, m: &KeyspaceMessages) -> Result<f64, String> {
    match number(text) {
        Some(score) if !text.trim().is_empty() => Ok(score),
        _ => Err((m.not_a_number)(text)),
    }
}

/// `field=value` lines; the value may contain `=`.
pub fn parse_fields(text: &str, m: &KeyspaceMessages) -> Result<Vec<(String, String)>, String> {
    text.split('\n')
        .filter(|line| !line.trim().is_empty())
        .map(|line| match line.find('=') {
            Some(cut) if cut > 0 => Ok((line[..cut].trim().to_owned(), line[cut + 1..].to_owned())),
            _ => Err((m.not_field_value)(line)),
        })
        .collect()
}

/// A time to live typed in seconds, in milliseconds; more than 0.
pub fn ttl_ms(seconds: &str, m: &KeyspaceMessages) -> Result<i64, String> {
    match number(seconds) {
        Some(ttl) if ttl > 0. => Ok((ttl * 1000.).round() as i64),
        _ => Err(m.seconds_greater_than_zero.to_owned()),
    }
}

/// What the TTL field starts with: the whole seconds left, rounded up.
pub fn ttl_seconds(ttl_ms: Option<i64>) -> String {
    ttl_ms.map(|ms| ((ms + 999) / 1000).to_string()).unwrap_or_default()
}

/// An argument as redis-cli reads it: bare when it can be, else in double
/// quotes with `\` and `"` escaped.
pub fn cli_arg(text: &str) -> String {
    let bare = !text.is_empty() && !text.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\'));
    if bare { text.to_owned() } else { format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"")) }
}

/// The redis-cli command that reads a whole key of `kind`.
pub fn read_command(name: &str, kind: &str) -> String {
    let key = cli_arg(name);
    match kind {
        "string" => format!("GET {key}"),
        "hash" => format!("HGETALL {key}"),
        "list" => format!("LRANGE {key} 0 -1"),
        "set" => format!("SMEMBERS {key}"),
        "zset" => format!("ZRANGE {key} 0 -1 WITHSCORES"),
        "stream" => format!("XRANGE {key} - +"),
        _ => format!("TYPE {key}"),
    }
}

/// `host:port` for the CLI's header, or the host of a connection given as a
/// URL (`rediss://user@cache.example.com:6380/0` → `cache.example.com:6380`).
pub fn address_of(host: &str, port: u16) -> String {
    match host.split_once("://") {
        Some((scheme, rest)) if !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_lowercase()) => {
            let rest = rest.split_once('@').map_or(rest, |(_, after)| after);
            rest.split('/').next().unwrap_or_default().to_owned()
        }
        _ => format!("{host}:{port}"),
    }
}

/// How a type names its elements and the columns of its table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    /// The elements, plural: `fields`, `items`…
    pub noun: &'static str,
    /// The button adding one; strings have none.
    pub add: Option<&'static str>,
    /// The column naming an element (field, index, id); sets have none.
    pub key: Option<&'static str>,
    pub value: &'static str,
    /// Whether the server filters the elements by a glob (`HSCAN`, `SSCAN`, `ZSCAN`).
    pub filter: bool,
}

/// The shape of a key of `kind`; `None` for a module's type.
pub fn shape(kind: &str, m: &KeyspaceMessages) -> Option<Shape> {
    let (noun, add, key, value, filter) = match kind {
        "hash" => (m.noun_fields, Some(m.add_field), Some(m.field), m.value, true),
        "list" => (m.noun_items, Some(m.add_item), Some(m.index), m.value, false),
        "set" | "zset" => (m.noun_members, Some(m.add_member), None, m.member, true),
        "stream" => (m.noun_entries, Some(m.add_entry), Some(m.id), m.fields, false),
        "string" => (m.noun_bytes, None, None, m.value, false),
        _ => return None,
    };
    Some(Shape { noun, add, key, value, filter })
}

/// The edit removing `entry` from a key of `kind`.
pub fn remove_edit(kind: &str, entry: &Entry) -> KeyEdit {
    let key = entry.key.clone();
    match (kind, key) {
        ("hash", Some(field)) => KeyEdit::RemoveField { field },
        ("list", Some(index)) => {
            KeyEdit::RemoveItem { index: index.text.parse().unwrap_or_default(), old: entry.value.clone() }
        }
        ("stream", Some(id)) => KeyEdit::RemoveEntry { id: id.text },
        _ => KeyEdit::RemoveMember { member: entry.value.clone() },
    }
}

/// The edit saving `text` over `entry` of a key of `kind`: a hash field's
/// or list item's value, or a sorted-set member's score.
pub fn save_edit(kind: &str, entry: &Entry, text: &str, m: &KeyspaceMessages) -> Result<KeyEdit, String> {
    Ok(match (kind, entry.key.clone()) {
        ("string", _) => KeyEdit::SetString { value: text.to_owned() },
        ("hash", Some(field)) => KeyEdit::SetField { field, value: text.to_owned() },
        ("list", Some(index)) => KeyEdit::SetItem {
            index: index.text.parse().unwrap_or_default(),
            old: entry.value.clone(),
            value: text.to_owned(),
        },
        _ => KeyEdit::AddMember { member: entry.value.clone(), score: Some(parse_score(text, m)?) },
    })
}

/// Whether the value viewer offers editing `entry` of a key of `kind`.
pub fn editable(kind: &str, length: i64, entry: &Entry) -> bool {
    if entry.value.binary {
        return false;
    }
    match kind {
        // Only a string read whole: a longer one shows its first 1 MB.
        "string" => length == entry.size as i64,
        "hash" | "list" | "zset" => true,
        _ => false,
    }
}

/// A field of an add dialog: what it asks for and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldName {
    Name,
    Kind,
    Field,
    Value,
    Member,
    Score,
    Ttl,
    End,
    Fields,
}

/// The fields of the dialog adding an element to a key of `kind`, in order.
pub fn add_fields(kind: &str) -> &'static [FieldName] {
    use FieldName::*;
    match kind {
        "hash" => &[Field, Value],
        "list" => &[Value, End],
        "zset" => &[Member, Score],
        "stream" => &[Fields],
        _ => &[Member],
    }
}

/// The fields of the dialog adding a key, for the type picked.
pub fn new_key_fields(kind: &str) -> Vec<FieldName> {
    use FieldName::*;
    let mut fields = vec![Name, Kind];
    if kind == "hash" || kind == "stream" {
        fields.push(Field);
    }
    fields.push(Value);
    if kind == "zset" {
        fields.push(Score);
    }
    fields.push(Ttl);
    fields
}

/// The values an add dialog holds, by field.
pub trait FormValues {
    fn get(&self, field: FieldName) -> String;
}

/// The edit adding what the dialog for a key of `kind` holds.
pub fn add_edit(kind: &str, form: &dyn FormValues, m: &KeyspaceMessages) -> Result<KeyEdit, String> {
    let text = |field| form.get(field);
    Ok(match kind {
        "hash" => {
            let field = text(FieldName::Field);
            if field.is_empty() {
                return Err(m.enter_field_name.to_owned());
            }
            KeyEdit::SetField { field: Text::from(field.as_str()), value: text(FieldName::Value) }
        }
        "list" => KeyEdit::PushItem { value: text(FieldName::Value), head: text(FieldName::End) == "head" },
        "zset" => KeyEdit::AddMember {
            member: Text::from(text(FieldName::Member).as_str()),
            score: Some(parse_score(&text(FieldName::Score), m)?),
        },
        "stream" => KeyEdit::AddEntry { fields: parse_fields(&text(FieldName::Fields), m)? },
        _ => KeyEdit::AddMember { member: Text::from(text(FieldName::Member).as_str()), score: None },
    })
}

/// The key the add-key dialog describes.
pub fn new_key(form: &dyn FormValues, m: &KeyspaceMessages) -> Result<NewKey, String> {
    let kind = form.get(FieldName::Kind);
    let ttl = form.get(FieldName::Ttl);
    let ttl_ms = if ttl.trim().is_empty() { None } else { Some(ttl_ms(&ttl, m)?) };
    Ok(NewKey {
        name: form.get(FieldName::Name),
        field: (kind == "hash" || kind == "stream").then(|| form.get(FieldName::Field)),
        value: form.get(FieldName::Value),
        score: if kind == "zset" { Some(parse_score(&form.get(FieldName::Score), m)?) } else { None },
        ttl_ms,
        kind,
    })
}

/// How the value viewer shows a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueMode {
    Text,
    Json,
    Hex,
    Msgpack,
}

/// The mode a value opens in: binary values have no text to show, so
/// MessagePack when they decode, else hex; text as JSON when it is.
pub fn first_mode(binary: bool, json: bool, msgpack: bool) -> ValueMode {
    match (binary, json, msgpack) {
        (true, _, true) => ValueMode::Msgpack,
        (true, _, false) => ValueMode::Hex,
        (false, true, _) => ValueMode::Json,
        (false, false, _) => ValueMode::Text,
    }
}

/// What the viewer shows of `value` in `mode`, given its JSON and
/// MessagePack readings when it has them.
pub fn shown_value(value: &Text, mode: ValueMode, json: Option<&str>, msgpack: Option<&str>) -> String {
    match (mode, json, msgpack) {
        (ValueMode::Msgpack, _, Some(msgpack)) => msgpack.to_owned(),
        _ if value.binary => spaced_hex(&value.text),
        (ValueMode::Json, Some(json), _) => json.to_owned(),
        (ValueMode::Hex, _, _) => as_hex(&value.text),
        _ => value.text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::i18n::Language;

    fn en() -> &'static KeyspaceMessages {
        &Language::En.messages().keyspace
    }

    fn es() -> &'static KeyspaceMessages {
        &Language::Es.messages().keyspace
    }

    #[test]
    fn writes_ttls_in_their_largest_unit() {
        assert_eq!(format_ttl(None, en(), false), "no ttl");
        assert_eq!(format_ttl(Some(41_000), en(), false), "41s");
        assert_eq!(format_ttl(Some(23 * 60_000 + 41_000), en(), false), "23m");
        assert_eq!(format_ttl(Some(2 * 3_600_000 + 5), en(), false), "2h");
        assert_eq!(format_ttl(Some(3 * 86_400_000), en(), false), "3d");
        assert_eq!(format_ttl(Some(23 * 60_000 + 41_000), en(), true), "23m 41s");
        assert_eq!(format_ttl(Some(450), en(), false), "450ms");
    }

    #[test]
    fn writes_the_ttl_and_errors_in_spanish() {
        assert_eq!(format_ttl(None, es(), false), "sin ttl");
        assert_eq!(parse_score("abc", es()), Err("\"abc\" no es un número.".to_owned()));
    }

    #[test]
    fn writes_sizes_and_the_hit_rate() {
        assert_eq!(format_bytes(412), "412 B");
        assert_eq!(format_bytes(412 * 1024 * 1024), "412 MB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(102_810), "100 KB", "no decimal from 100 up");
        assert_eq!(format_bytes(154_112), "151 KB", "a half rounds up, as toFixed does");
        assert_eq!(hit_rate(1, 7), "12.5%");
        assert_eq!(hit_rate(974, 26), "97.4%");
        assert_eq!(hit_rate(0, 0), "—");
    }

    #[test]
    fn shows_values_as_json_or_hex_when_they_can_be() {
        assert_eq!(as_json(r#"{"a":[1,2]}"#).as_deref(), Some("{\n  \"a\": [\n    1,\n    2\n  ]\n}"));
        assert_eq!(as_json("Mozilla/5.0"), None);
        assert_eq!(as_json("[1.0, 2.5]").as_deref(), Some("[\n  1,\n  2.5\n]"), "numbers as JavaScript writes them");
        assert_eq!(as_hex("AB"), "41 42");
        assert_eq!(as_hex("é"), "c3 a9");
        assert_eq!(spaced_hex("ff01a"), "ff 01 a");
    }

    #[test]
    fn reads_each_kind_of_key_with_its_command_quoting_names_redis_cli_would_split() {
        assert_eq!(read_command("user:1", "hash"), "HGETALL user:1");
        assert_eq!(read_command("my list", "list"), "LRANGE \"my list\" 0 -1");
        assert_eq!(read_command("say \"hi\"\\", "string"), "GET \"say \\\"hi\\\"\\\\\"");
        assert_eq!(read_command("board", "zset"), "ZRANGE board 0 -1 WITHSCORES");
        assert_eq!(read_command("events", "stream"), "XRANGE events - +");
        assert_eq!(read_command("x", "ReJSON-RL"), "TYPE x");
        assert_eq!(cli_arg(""), "\"\"", "an empty name is quoted");
    }

    #[test]
    fn writes_the_address_the_cli_names() {
        assert_eq!(address_of("127.0.0.1", 6379), "127.0.0.1:6379");
        assert_eq!(address_of("rediss://user:pw@cache.example.com:6380/0", 6379), "cache.example.com:6380");
        assert_eq!(address_of("redis://h", 6379), "h");
    }

    #[test]
    fn reads_scores_fields_and_seconds() {
        assert_eq!(parse_score(" 2.5 ", en()), Ok(2.5));
        assert_eq!(parse_score("", en()), Err("\"\" isn't a number.".to_owned()));
        assert_eq!(parse_score("inf", en()), Err("\"inf\" isn't a number.".to_owned()));
        assert_eq!(
            parse_fields("event=login\n\nnote=a=b\n", en()),
            Ok(vec![("event".into(), "login".into()), ("note".into(), "a=b".into())])
        );
        assert_eq!(parse_fields("=x", en()), Err("\"=x\" isn't field=value.".to_owned()));
        assert_eq!(ttl_ms("1.5", en()), Ok(1500));
        assert_eq!(ttl_ms("0", en()), Err(en().seconds_greater_than_zero.to_owned()));
        assert_eq!(ttl_ms("", en()), Err(en().seconds_greater_than_zero.to_owned()));
        assert_eq!(ttl_seconds(Some(41_001)), "42");
        assert_eq!(ttl_seconds(None), "");
    }

    fn entry(key: Option<&str>, value: &str, binary: bool) -> Entry {
        Entry { key: key.map(Text::from), value: Text { text: value.into(), binary }, score: None, size: value.len() }
    }

    #[test]
    fn names_each_type_s_elements() {
        let hash = shape("hash", en()).expect("a hash shape");
        assert_eq!((hash.noun, hash.add, hash.key, hash.filter), ("fields", Some("Add field"), Some("Field"), true));
        assert_eq!(shape("string", en()).and_then(|shape| shape.add), None);
        assert_eq!(shape("zset", en()).and_then(|shape| shape.key), None);
        assert!(shape("ReJSON-RL", en()).is_none(), "a module's keys are read in the CLI");
    }

    #[test]
    fn removes_and_saves_each_type_s_elements() {
        let field = entry(Some("name"), "Ada", false);
        assert_eq!(remove_edit("hash", &field), KeyEdit::RemoveField { field: Text::from("name") });
        let item = entry(Some("3"), "x", false);
        assert_eq!(remove_edit("list", &item), KeyEdit::RemoveItem { index: 3, old: Text::from("x") });
        assert_eq!(remove_edit("stream", &entry(Some("1-0"), "{}", false)), KeyEdit::RemoveEntry { id: "1-0".into() });
        assert_eq!(remove_edit("set", &entry(None, "m", false)), KeyEdit::RemoveMember { member: Text::from("m") });
        assert_eq!(
            save_edit("list", &item, "y", en()),
            Ok(KeyEdit::SetItem { index: 3, old: Text::from("x"), value: "y".into() })
        );
        assert_eq!(
            save_edit("zset", &entry(None, "m", false), "4", en()),
            Ok(KeyEdit::AddMember { member: Text::from("m"), score: Some(4.) })
        );
        assert!(save_edit("zset", &entry(None, "m", false), "four", en()).is_err());
        assert!(!editable("hash", 1, &entry(Some("f"), "00ff", true)), "binary values aren't edited");
        assert!(!editable("string", 2_000_000, &entry(None, "abc", false)), "a cut string isn't edited");
        assert!(!editable("set", 1, &entry(None, "m", false)), "a set member has nothing to edit");
    }

    struct Form(HashMap<&'static str, &'static str>);

    impl FormValues for Form {
        fn get(&self, field: FieldName) -> String {
            let name = match field {
                FieldName::Name => "name",
                FieldName::Kind => "kind",
                FieldName::Field => "field",
                FieldName::Value => "value",
                FieldName::Member => "member",
                FieldName::Score => "score",
                FieldName::Ttl => "ttl",
                FieldName::End => "end",
                FieldName::Fields => "fields",
            };
            self.0.get(name).copied().unwrap_or_default().to_owned()
        }
    }

    #[test]
    fn adds_elements_as_each_type_takes_them() {
        let form = |pairs: &[(&'static str, &'static str)]| Form(pairs.iter().copied().collect());
        assert_eq!(add_edit("hash", &form(&[("value", "v")]), en()), Err("Enter a field name.".to_owned()));
        assert_eq!(
            add_edit("list", &form(&[("value", "v"), ("end", "head")]), en()),
            Ok(KeyEdit::PushItem { value: "v".into(), head: true })
        );
        assert_eq!(
            add_edit("stream", &form(&[("fields", "a=1")]), en()),
            Ok(KeyEdit::AddEntry { fields: vec![("a".into(), "1".into())] })
        );
        assert_eq!(add_fields("list"), [FieldName::Value, FieldName::End]);
        assert_eq!(
            new_key_fields("zset"),
            [FieldName::Name, FieldName::Kind, FieldName::Value, FieldName::Score, FieldName::Ttl]
        );
        assert_eq!(new_key_fields("hash")[2], FieldName::Field);
    }

    #[test]
    fn describes_a_new_key_with_its_first_element() {
        let form = |pairs: &[(&'static str, &'static str)]| Form(pairs.iter().copied().collect());
        let key =
            new_key(&form(&[("name", "s:1"), ("kind", "hash"), ("field", "f"), ("value", "v"), ("ttl", "2")]), en())
                .expect("a key");
        assert_eq!(
            key,
            NewKey {
                name: "s:1".into(),
                kind: "hash".into(),
                field: Some("f".into()),
                value: "v".into(),
                score: None,
                ttl_ms: Some(2000)
            }
        );
        let plain = new_key(&form(&[("name", "n"), ("kind", "string"), ("field", "ignored"), ("ttl", " ")]), en());
        assert_eq!(plain.map(|key| (key.field, key.ttl_ms)), Ok((None, None)));
        assert!(new_key(&form(&[("kind", "string"), ("ttl", "-1")]), en()).is_err());
    }

    #[test]
    fn opens_values_in_the_mode_that_reads_best() {
        assert_eq!(first_mode(true, false, true), ValueMode::Msgpack);
        assert_eq!(first_mode(true, false, false), ValueMode::Hex);
        assert_eq!(first_mode(false, true, false), ValueMode::Json);
        assert_eq!(first_mode(false, false, false), ValueMode::Text);
        let binary = Text { text: "c3a9ff".into(), binary: true };
        assert_eq!(shown_value(&binary, ValueMode::Text, None, None), "c3 a9 ff");
        let text = Text::from("é");
        assert_eq!(shown_value(&text, ValueMode::Hex, None, None), "c3 a9");
        assert_eq!(shown_value(&text, ValueMode::Json, None, None), "é", "no JSON reading: the text");
    }
}
