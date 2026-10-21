//! Template data that Tera 2 can compare and count.
//!
//! The workspace turns on `serde_json`'s `arbitrary_precision` (the token verifier needs it), and with that on
//! a `serde_json::Number` serialises as a one-field map. Tera 2 then sees a map where a number should be and
//! fails on `{% if w.occupied > 0 %}`. [`Plain`] writes each number as an integer or float instead.

use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use serde_json::Value;

/// A JSON value that serialises its numbers as plain numbers.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Plain(pub PlainValue);

#[derive(Debug, Clone, PartialEq)]
pub struct PlainValue(pub Value);

impl Serialize for PlainValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match &self.0 {
            Value::Null => s.serialize_unit(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
                (Some(i), _, _) => s.serialize_i64(i),
                (_, Some(u), _) => s.serialize_u64(u),
                (_, _, Some(f)) => s.serialize_f64(f),
                _ => s.serialize_str(&n.to_string()),
            },
            Value::String(t) => s.serialize_str(t),
            Value::Array(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for v in a {
                    seq.serialize_element(&PlainValue(v.clone()))?;
                }
                seq.end()
            }
            Value::Object(o) => {
                let mut map = s.serialize_map(Some(o.len()))?;
                for (k, v) in o {
                    map.serialize_entry(k, &PlainValue(v.clone()))?;
                }
                map.end()
            }
        }
    }
}

/// `json!` for template data: the same syntax, but the result renders numbers Tera can use.
macro_rules! json {
    ($($t:tt)*) => { $crate::tpl::Plain($crate::tpl::PlainValue(serde_json::json!($($t)*))) };
}
pub(crate) use json;
