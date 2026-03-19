//! Parameter resolution engine for scheduled workflows.
//!
//! Resolves parameter values through a three-step chain:
//! 1. Defaults from a parameter schema
//! 2. Static overrides from the schedule's `parameter_values`
//! 3. Dynamic expressions evaluated at trigger time (e.g. `{{ trigger.time }}`)

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::trigger::TriggerType;

/// Context available during parameter resolution.
#[derive(Debug, Clone)]
pub struct ResolutionContext {
    /// When the trigger fired.
    pub trigger_time: DateTime<Utc>,
    /// Type of trigger.
    pub trigger_type: TriggerType,
    /// Execution sequence number.
    pub execution_sequence: u64,
    /// Optional event payload (for event-driven triggers).
    pub event_payload: Option<Value>,
}

/// Resolve parameter values for a scheduled workflow execution.
///
/// Resolution chain:
/// 1. Start with `defaults`
/// 2. Override with `static_values` (schedule's `parameter_values`)
/// 3. Resolve expression strings (`{{ expr }}`) using `context`
pub fn resolve_parameters(
    defaults: &Value,
    static_values: &Value,
    context: &ResolutionContext,
) -> Value {
    let mut result = merge_values(defaults, static_values);
    resolve_expressions(&mut result, context);
    result
}

/// Merge two JSON objects. Values in `overlay` override those in `base`.
/// Non-object values in `overlay` replace `base` entirely.
fn merge_values(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            let mut merged = base_map.clone();
            for (key, value) in overlay_map {
                if let Some(existing) = merged.get(key) {
                    merged.insert(key.clone(), merge_values(existing, value));
                } else {
                    merged.insert(key.clone(), value.clone());
                }
            }
            Value::Object(merged)
        }
        (_, overlay) => overlay.clone(),
    }
}

/// Resolve `{{ expression }}` strings in a JSON value tree.
fn resolve_expressions(value: &mut Value, ctx: &ResolutionContext) {
    match value {
        Value::String(s) => {
            if let Some(resolved) = try_resolve_expression(s, ctx) {
                *value = resolved;
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                resolve_expressions(v, ctx);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                resolve_expressions(v, ctx);
            }
        }
        _ => {}
    }
}

/// Try to resolve a single `{{ expr }}` expression.
///
/// Supported expressions:
/// - `{{ trigger.time }}` → ISO 8601 timestamp
/// - `{{ trigger.type }}` → trigger type string
/// - `{{ execution.sequence }}` → sequence number
/// - `{{ event.payload.FIELD }}` → field from event payload
fn try_resolve_expression(s: &str, ctx: &ResolutionContext) -> Option<Value> {
    let trimmed = s.trim();
    if !trimmed.starts_with("{{") || !trimmed.ends_with("}}") {
        return None;
    }

    let expr = trimmed[2..trimmed.len() - 2].trim();

    match expr {
        "trigger.time" => Some(Value::String(ctx.trigger_time.to_rfc3339())),
        "trigger.type" => Some(Value::String(ctx.trigger_type.to_string())),
        "execution.sequence" => Some(Value::Number(ctx.execution_sequence.into())),
        _ if expr.starts_with("event.payload.") => {
            let field = &expr["event.payload.".len()..];
            ctx.event_payload
                .as_ref()
                .and_then(|payload| resolve_json_path(payload, field))
        }
        _ => None, // Unknown expression — leave as-is.
    }
}

/// Resolve a dot-path (e.g. `"nested.field"`) in a JSON value.
fn resolve_json_path(value: &Value, path: &str) -> Option<Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;
    for part in parts {
        match current {
            Value::Object(map) => {
                current = map.get(part)?;
            }
            _ => return None,
        }
    }
    Some(current.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_context() -> ResolutionContext {
        ResolutionContext {
            trigger_time: "2026-03-11T09:00:00Z".parse().unwrap(),
            trigger_type: TriggerType::Cron,
            execution_sequence: 42,
            event_payload: Some(json!({
                "path": "/workspace/test.md",
                "nested": { "value": 123 }
            })),
        }
    }

    #[test]
    fn test_param_resolution_defaults_only() {
        let defaults = json!({"key": "default_value", "count": 10});
        let static_values = json!({});
        let ctx = test_context();

        let result = resolve_parameters(&defaults, &static_values, &ctx);
        assert_eq!(result["key"], "default_value");
        assert_eq!(result["count"], 10);
    }

    #[test]
    fn test_param_resolution_static_override() {
        let defaults = json!({"key": "default", "count": 10});
        let static_values = json!({"key": "overridden"});
        let ctx = test_context();

        let result = resolve_parameters(&defaults, &static_values, &ctx);
        assert_eq!(result["key"], "overridden");
        assert_eq!(result["count"], 10); // Not overridden.
    }

    #[test]
    fn test_param_resolution_trigger_time() {
        let defaults = json!({});
        let static_values = json!({"fired_at": "{{ trigger.time }}"});
        let ctx = test_context();

        let result = resolve_parameters(&defaults, &static_values, &ctx);
        assert!(result["fired_at"].as_str().unwrap().contains("2026-03-11"));
    }

    #[test]
    fn test_param_resolution_trigger_type() {
        let defaults = json!({});
        let static_values = json!({"type": "{{ trigger.type }}"});
        let ctx = test_context();

        let result = resolve_parameters(&defaults, &static_values, &ctx);
        assert_eq!(result["type"], "cron");
    }

    #[test]
    fn test_param_resolution_execution_sequence() {
        let defaults = json!({});
        let static_values = json!({"seq": "{{ execution.sequence }}"});
        let ctx = test_context();

        let result = resolve_parameters(&defaults, &static_values, &ctx);
        assert_eq!(result["seq"], 42);
    }

    #[test]
    fn test_param_resolution_event_payload() {
        let defaults = json!({});
        let static_values = json!({"file": "{{ event.payload.path }}"});
        let ctx = test_context();

        let result = resolve_parameters(&defaults, &static_values, &ctx);
        assert_eq!(result["file"], "/workspace/test.md");
    }

    #[test]
    fn test_param_resolution_nested_event_payload() {
        let defaults = json!({});
        let static_values = json!({"val": "{{ event.payload.nested.value }}"});
        let ctx = test_context();

        let result = resolve_parameters(&defaults, &static_values, &ctx);
        assert_eq!(result["val"], 123);
    }

    #[test]
    fn test_param_resolution_unknown_expression_unchanged() {
        let defaults = json!({});
        let static_values = json!({"x": "{{ unknown.expr }}"});
        let ctx = test_context();

        let result = resolve_parameters(&defaults, &static_values, &ctx);
        // Unknown expressions remain as string.
        assert_eq!(result["x"], "{{ unknown.expr }}");
    }

    #[test]
    fn test_param_resolution_non_expression_unchanged() {
        let defaults = json!({});
        let static_values = json!({"msg": "Hello world"});
        let ctx = test_context();

        let result = resolve_parameters(&defaults, &static_values, &ctx);
        assert_eq!(result["msg"], "Hello world");
    }

    #[test]
    fn test_merge_nested_objects() {
        let base = json!({"a": {"x": 1, "y": 2}});
        let overlay = json!({"a": {"y": 3, "z": 4}});

        let merged = merge_values(&base, &overlay);
        assert_eq!(merged["a"]["x"], 1);
        assert_eq!(merged["a"]["y"], 3);
        assert_eq!(merged["a"]["z"], 4);
    }
}
