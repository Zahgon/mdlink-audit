//! JSON documents written the way the reports print them: two-space indentation with `", "` and
//! `": "` separators, keys in insertion order, and non-ASCII characters kept verbatim.

use crate::output::json_string;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn str(text: impl Into<String>) -> Value {
        Value::Str(text.into())
    }

    pub fn object(pairs: Vec<(&str, Value)>) -> Value {
        Value::Object(pairs.into_iter().map(|(key, value)| (key.to_string(), value)).collect())
    }

    /// The document with two-space indentation, without a final line break.
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        self.write_pretty(&mut out, 0);
        out
    }

    fn write_pretty(&self, out: &mut String, depth: usize) {
        let pad = |depth: usize| "  ".repeat(depth);
        match self {
            Value::Array(items) if !items.is_empty() => {
                out.push_str("[\n");
                for (index, item) in items.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    item.write_pretty(out, depth + 1);
                    out.push_str(if index + 1 == items.len() { "\n" } else { ",\n" });
                }
                out.push_str(&pad(depth));
                out.push(']');
            }
            Value::Object(pairs) if !pairs.is_empty() => {
                out.push_str("{\n");
                for (index, (key, value)) in pairs.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    out.push_str(&json_string(key));
                    out.push_str(": ");
                    value.write_pretty(out, depth + 1);
                    out.push_str(if index + 1 == pairs.len() { "\n" } else { ",\n" });
                }
                out.push_str(&pad(depth));
                out.push('}');
            }
            other => other.write_compact(out, false),
        }
    }

    /// The document on one line; with `sort_keys`, object keys in code point order.
    pub fn compact(&self, sort_keys: bool) -> String {
        let mut out = String::new();
        self.write_compact(&mut out, sort_keys);
        out
    }

    fn write_compact(&self, out: &mut String, sort_keys: bool) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
            Value::Int(number) => out.push_str(&number.to_string()),
            Value::Str(text) => out.push_str(&json_string(text)),
            Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    item.write_compact(out, sort_keys);
                }
                out.push(']');
            }
            Value::Object(pairs) => {
                let mut ordered: Vec<&(String, Value)> = pairs.iter().collect();
                if sort_keys {
                    ordered.sort_by(|a, b| a.0.cmp(&b.0));
                }
                out.push('{');
                for (index, (key, value)) in ordered.into_iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&json_string(key));
                    out.push_str(": ");
                    value.write_compact(out, sort_keys);
                }
                out.push('}');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts() {
        let value = Value::object(vec![
            ("a", Value::Array(vec![])),
            ("b", Value::object(vec![("z", Value::Int(1)), ("y", Value::Null)])),
            ("c", Value::Array(vec![Value::Bool(true), Value::str("é\n")])),
        ]);
        assert_eq!(
            value.pretty(),
            "{\n  \"a\": [],\n  \"b\": {\n    \"z\": 1,\n    \"y\": null\n  },\n  \"c\": [\n    true,\n    \"é\\n\"\n  ]\n}"
        );
        assert_eq!(
            value.compact(true),
            "{\"a\": [], \"b\": {\"y\": null, \"z\": 1}, \"c\": [true, \"é\\n\"]}"
        );
    }
}
