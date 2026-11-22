//! JSON helpers that cannot panic. `value["key"] = x` panics when the value is not an object (or null), so
//! the code that builds a FHIR resource field by field goes through [`set`] instead.

use serde_json::Value;

static NULL: Value = Value::Null;

/// `v[key]` without the panic: the member, or JSON null when `v` is not an object or has no such member. Read
/// chains (`at(at(v, "a"), "b")`) end in null, as `Value`'s own indexing does for a missing key.
pub fn at<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or(&NULL)
}

/// Set `key` to `val` when `v` is an object; do nothing when it is not (a resource built from `json!({..})`
/// always is).
pub fn set(v: &mut Value, key: &str, val: Value) {
    if let Value::Object(map) = v {
        map.insert(key.to_string(), val);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_key_is_set_on_an_object_and_a_non_object_is_left_alone() {
        let mut o = json!({ "a": 1 });
        set(&mut o, "b", json!(2));
        set(&mut o, "a", json!(3));
        assert_eq!(o, json!({ "a": 3, "b": 2 }));
        let mut not_object = json!([1]);
        set(&mut not_object, "k", json!(1));
        assert_eq!(not_object, json!([1]));
    }

    #[test]
    fn a_missing_member_reads_as_null_all_the_way_down() {
        let v = json!({ "a": { "b": 1 } });
        assert_eq!(at(at(&v, "a"), "b"), &json!(1));
        assert!(at(at(&v, "x"), "y").is_null());
        assert!(at(&json!([1]), "k").is_null());
    }
}
