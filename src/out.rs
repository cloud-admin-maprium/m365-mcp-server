//! Tool-result plumbing plus the JSON helpers that keep response shapes
//! byte-compatible with the JS original.
//!
//! The distinction that matters: JS omits a key when the value is `undefined`
//! but emits `null` when the source explicitly wrote `x || null`. Every field
//! goes through exactly one of the helpers below, matching its origin.

use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Map, Value};

pub fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

pub fn tool_ok(value: Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(pretty(&value))])
}

/// Failure shape is unchanged (`{"error": "..."}`) but the call is now flagged,
/// so an expired secret or a 401 doesn't look like a successful tool call.
pub fn tool_err(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(pretty(&serde_json::json!({
        "error": message.into(),
    })))])
}

pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn put_str(map: &mut Map<String, Value>, key: &str, value: impl Into<String>) {
    map.insert(key.to_string(), Value::String(value.into()));
}

/// `key: value` when present, key absent when `None` — the `undefined` case.
pub fn put_opt_str(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        map.insert(key.to_string(), Value::String(value));
    }
}

pub fn put_opt_bool(map: &mut Map<String, Value>, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        map.insert(key.to_string(), Value::Bool(value));
    }
}

/// `value || ''` — always present, never null.
pub fn put_or_empty(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    map.insert(key.to_string(), Value::String(value.unwrap_or_default()));
}

/// `value || null` — always present, null when missing.
pub fn put_null_str(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    map.insert(key.to_string(), value.map_or(Value::Null, Value::String));
}

pub fn put_json(map: &mut Map<String, Value>, key: &str, value: Value) {
    map.insert(key.to_string(), value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_and_omitted_are_distinct() {
        let mut map = Map::new();
        put_opt_str(&mut map, "a", None);
        put_null_str(&mut map, "b", None);
        put_or_empty(&mut map, "c", None);
        let value = Value::Object(map);
        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            r#"{"b":null,"c":""}"#
        );
    }

    #[test]
    fn error_result_is_flagged_and_keeps_the_shape() {
        let result = tool_err("boom");
        assert_eq!(result.is_error, Some(true));
        let text = result.content[0].as_text().unwrap();
        assert_eq!(text.text.trim(), "{\n  \"error\": \"boom\"\n}");
    }
}
