//! A binary value read as MessagePack, pretty-printed as JSON the way the
//! web UI's `asMsgpack` (`@msgpack/msgpack` with 64-bit integers as
//! BigInt) did: bytes become `0x…` strings, 64-bit integers decimal
//! strings, timestamps ISO dates, other extensions `{"type", "data"}`.

use dbforge_core::drivers::redis::Text;
use serde_json::{Map, Number, Value};

use super::format::js_numbers;

/// The value pretty-printed as JSON when its bytes are exactly one
/// MessagePack value, else `None`. Text is never read as MessagePack: maps,
/// arrays and strings use bytes that aren't valid UTF-8, so only a lone
/// small number could pass, and `"5"` would show as 53.
pub fn as_msgpack(value: &Text) -> Option<String> {
    if !value.binary {
        return None;
    }
    let bytes = from_hex(&value.text)?;
    if bytes.is_empty() {
        return None;
    }
    let mut reader = Reader { bytes: &bytes, at: 0 };
    let decoded = reader.value(0)?;
    // Extra bytes: ordinary data rarely passes for MessagePack.
    if reader.at != bytes.len() {
        return None;
    }
    serde_json::to_string_pretty(&js_numbers(decoded)).ok()
}

fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    hex.as_bytes()
        .chunks(2)
        .map(|pair| std::str::from_utf8(pair).ok().and_then(|pair| u8::from_str_radix(pair, 16).ok()))
        .collect()
}

/// Nesting deeper than this is refused rather than risking the stack.
const MAX_DEPTH: usize = 512;

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.at.checked_add(n)?;
        let taken = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(taken)
    }

    fn uint(&mut self, n: usize) -> Option<u64> {
        Some(self.take(n)?.iter().fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte)))
    }

    fn int(&mut self, n: usize) -> Option<i64> {
        let raw = self.uint(n)?;
        let shift = 64 - 8 * n as u32;
        Some(((raw << shift) as i64) >> shift)
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        if depth > MAX_DEPTH {
            return None;
        }
        let head = *self.take(1)?.first()?;
        Some(match head {
            0x00..=0x7f => Value::from(head),
            0x80..=0x8f => self.map(usize::from(head & 0x0f), depth)?,
            0x90..=0x9f => self.array(usize::from(head & 0x0f), depth)?,
            0xa0..=0xbf => self.string(usize::from(head & 0x1f))?,
            0xc0 => Value::Null,
            0xc2 => Value::Bool(false),
            0xc3 => Value::Bool(true),
            0xc4 => self.binary(1)?,
            0xc5 => self.binary(2)?,
            0xc6 => self.binary(4)?,
            0xc7 => self.extension(1, None)?,
            0xc8 => self.extension(2, None)?,
            0xc9 => self.extension(4, None)?,
            0xca => float(f64::from(f32::from_bits(self.uint(4)? as u32))),
            0xcb => float(f64::from_bits(self.uint(8)?)),
            0xcc => Value::from(self.uint(1)?),
            0xcd => Value::from(self.uint(2)?),
            0xce => Value::from(self.uint(4)?),
            // 64-bit integers are BigInts, which the web UI wrote as decimal strings.
            0xcf => Value::String(self.uint(8)?.to_string()),
            0xd0 => Value::from(self.int(1)?),
            0xd1 => Value::from(self.int(2)?),
            0xd2 => Value::from(self.int(4)?),
            0xd3 => Value::String(self.int(8)?.to_string()),
            0xd4 => self.extension(0, Some(1))?,
            0xd5 => self.extension(0, Some(2))?,
            0xd6 => self.extension(0, Some(4))?,
            0xd7 => self.extension(0, Some(8))?,
            0xd8 => self.extension(0, Some(16))?,
            0xd9 => {
                let n = self.uint(1)? as usize;
                self.string(n)?
            }
            0xda => {
                let n = self.uint(2)? as usize;
                self.string(n)?
            }
            0xdb => {
                let n = self.uint(4)? as usize;
                self.string(n)?
            }
            0xdc => {
                let n = self.uint(2)? as usize;
                self.array(n, depth)?
            }
            0xdd => {
                let n = self.uint(4)? as usize;
                self.array(n, depth)?
            }
            0xde => {
                let n = self.uint(2)? as usize;
                self.map(n, depth)?
            }
            0xdf => {
                let n = self.uint(4)? as usize;
                self.map(n, depth)?
            }
            0xe0..=0xff => Value::from(head as i8),
            // 0xc1 is never used.
            _ => return None,
        })
    }

    fn string(&mut self, n: usize) -> Option<Value> {
        Some(Value::String(String::from_utf8_lossy(self.take(n)?).into_owned()))
    }

    fn binary(&mut self, size: usize) -> Option<Value> {
        let n = self.uint(size)? as usize;
        Some(Value::String(hex(self.take(n)?)))
    }

    fn array(&mut self, n: usize, depth: usize) -> Option<Value> {
        // Each item takes a byte at least: a count beyond the bytes left is refused before allocating.
        if n > self.bytes.len() - self.at {
            return None;
        }
        (0..n).map(|_| self.value(depth + 1)).collect::<Option<Vec<_>>>().map(Value::Array)
    }

    fn map(&mut self, n: usize, depth: usize) -> Option<Value> {
        if n > self.bytes.len() - self.at {
            return None;
        }
        let mut fields = Map::new();
        for _ in 0..n {
            // Keys are strings or numbers, as the web UI's decoder allows.
            let key = match self.value(depth + 1)? {
                Value::String(key) => key,
                Value::Number(number) => number.to_string(),
                _ => return None,
            };
            let value = self.value(depth + 1)?;
            fields.insert(key, value);
        }
        Some(Value::Object(fields))
    }

    /// An extension of `fixed` bytes, or with its length in `size` bytes.
    fn extension(&mut self, size: usize, fixed: Option<usize>) -> Option<Value> {
        let n = match fixed {
            Some(n) => n,
            None => self.uint(size)? as usize,
        };
        let kind = self.int(1)?;
        let data = self.take(n)?;
        if kind == -1 {
            return timestamp(data).map(Value::String);
        }
        let mut fields = Map::new();
        fields.insert("type".into(), Value::from(kind));
        fields.insert("data".into(), Value::String(hex(data)));
        Some(Value::Object(fields))
    }
}

/// Bytes as `0x…`.
fn hex(bytes: &[u8]) -> String {
    format!("0x{}", bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>())
}

/// A float as JSON writes one: NaN and the infinities as `null`.
fn float(value: f64) -> Value {
    Number::from_f64(value).map_or(Value::Null, Value::Number)
}

/// A MessagePack timestamp (extension -1) as JavaScript's `toISOString`
/// writes a date: milliseconds, UTC.
fn timestamp(data: &[u8]) -> Option<String> {
    let number = |bytes: &[u8]| bytes.iter().fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte));
    let (seconds, nanos) = match data.len() {
        4 => (number(data) as i64, 0),
        8 => {
            let both = number(data);
            ((both & 0x3_ffff_ffff) as i64, (both >> 34) as u32)
        }
        12 => (number(&data[4..]) as i64, number(&data[..4]) as u32),
        _ => return None,
    };
    let millis = seconds.checked_mul(1000)?.checked_add(i64::from(nanos / 1_000_000))?;
    let date = chrono::DateTime::from_timestamp_millis(millis)?;
    Some(date.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary(hex: &str) -> Text {
        Text { text: hex.into(), binary: true }
    }

    /// `{user: "u_88412", roles: ["admin"], seen: 1730718422, ok: true, blob: Uint8Array [1, 255]}`
    /// as `@msgpack/msgpack` encodes it.
    const PACKED: &str =
        "85a475736572a7755f3838343132a5726f6c657391a561646d696ea47365656ece6728aad6a26f6bc3a4626c6f62c40201ff";

    #[test]
    fn decodes_binary_values_the_backend_sent_as_hex() {
        assert_eq!(
            as_msgpack(&binary(PACKED)).as_deref(),
            Some(
                "{\n  \"user\": \"u_88412\",\n  \"roles\": [\n    \"admin\"\n  ],\n  \"seen\": 1730718422,\n  \"ok\": true,\n  \"blob\": \"0x01ff\"\n}"
            )
        );
    }

    #[test]
    fn leaves_text_alone_messagepack_is_binary_and_5_would_read_as_53() {
        assert_eq!(as_msgpack(&Text::from("5")), None);
    }

    #[test]
    fn refuses_anything_that_is_not_one_whole_messagepack_value() {
        assert_eq!(as_msgpack(&Text::from("hello")), None);
        assert_eq!(as_msgpack(&binary("ff01")), None, "extra bytes");
        assert_eq!(as_msgpack(&binary("")), None);
        assert_eq!(as_msgpack(&binary("dc ff")), None, "not hex");
        assert_eq!(as_msgpack(&binary("dd7fffffff")), None, "a count beyond the bytes");
    }

    #[test]
    fn writes_64_bit_integers_floats_and_timestamps_as_the_web_ui_did() {
        assert_eq!(as_msgpack(&binary("cf0000000000000001")).as_deref(), Some("\"1\""));
        assert_eq!(as_msgpack(&binary("d3ffffffffffffffff")).as_deref(), Some("\"-1\""));
        assert_eq!(as_msgpack(&binary("cb3ff8000000000000")).as_deref(), Some("1.5"));
        assert_eq!(as_msgpack(&binary("cb4000000000000000")).as_deref(), Some("2"));
        assert_eq!(as_msgpack(&binary("e0")).as_deref(), Some("-32"));
        // 2024-11-04T11:04:54Z, a 32-bit timestamp.
        assert_eq!(as_msgpack(&binary("d6ff6728aa56")).as_deref(), Some("\"2024-11-04T11:04:54.000Z\""));
        assert_eq!(as_msgpack(&binary("d40501")).as_deref(), Some("{\n  \"type\": 5,\n  \"data\": \"0x01\"\n}"));
    }
}
