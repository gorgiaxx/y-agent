//! JSON Schema validator with compiled schema caching.

use std::collections::HashMap;
use std::sync::Arc;

use jsonschema::Validator;

use crate::error::ToolRegistryError;

/// Validates tool parameters against JSON Schema Draft 7.
///
/// Compiled schemas are cached for repeated validation of the same tool.
pub struct JsonSchemaValidator {
    /// Cache of compiled validators, keyed by a schema hash.
    cache: HashMap<String, Arc<Validator>>,
}

impl JsonSchemaValidator {
    /// Create a new validator.
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Compute a cache key for a schema (JSON string hash).
    fn cache_key(schema: &serde_json::Value) -> String {
        schema.to_string()
    }

    /// Validate parameters against a JSON Schema.
    ///
    /// Returns `Ok(())` if valid, or `Err(ValidationError)` with details.
    pub fn validate(
        &mut self,
        schema: &serde_json::Value,
        params: &serde_json::Value,
    ) -> Result<(), ToolRegistryError> {
        let key = Self::cache_key(schema);

        let validator = if let Some(cached) = self.cache.get(&key) {
            cached.clone()
        } else {
            let compiled = Validator::new(schema).map_err(|e| {
                ToolRegistryError::ValidationError {
                    message: format!("invalid schema: {e}"),
                }
            })?;
            let arc = Arc::new(compiled);
            self.cache.insert(key, arc.clone());
            arc
        };

        validator.validate(params).map_err(|e| {
            ToolRegistryError::ValidationError {
                message: e.to_string(),
            }
        })
    }

    /// Check if a schema is already cached.
    pub fn is_cached(&self, schema: &serde_json::Value) -> bool {
        let key = Self::cache_key(schema);
        self.cache.contains_key(&key)
    }
}

impl Default for JsonSchemaValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            },
            "required": ["name"]
        })
    }

    fn strict_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"],
            "additionalProperties": false
        })
    }

    fn nested_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "address": {
                    "type": "object",
                    "properties": {
                        "street": { "type": "string" },
                        "city": { "type": "string" }
                    },
                    "required": ["street"]
                }
            },
            "required": ["address"]
        })
    }

    #[test]
    fn test_validate_valid_params() {
        let mut v = JsonSchemaValidator::new();
        let schema = object_schema();
        let params = serde_json::json!({"name": "Alice", "age": 30});
        assert!(v.validate(&schema, &params).is_ok());
    }

    #[test]
    fn test_validate_missing_required_field() {
        let mut v = JsonSchemaValidator::new();
        let schema = object_schema();
        let params = serde_json::json!({"age": 30});
        let err = v.validate(&schema, &params).unwrap_err();
        assert!(matches!(err, ToolRegistryError::ValidationError { .. }));
    }

    #[test]
    fn test_validate_wrong_type() {
        let mut v = JsonSchemaValidator::new();
        let schema = object_schema();
        let params = serde_json::json!({"name": "Alice", "age": "thirty"});
        let err = v.validate(&schema, &params).unwrap_err();
        assert!(matches!(err, ToolRegistryError::ValidationError { .. }));
    }

    #[test]
    fn test_validate_additional_properties_denied() {
        let mut v = JsonSchemaValidator::new();
        let schema = strict_schema();
        let params = serde_json::json!({"name": "Alice", "extra": "field"});
        let err = v.validate(&schema, &params).unwrap_err();
        assert!(matches!(err, ToolRegistryError::ValidationError { .. }));
    }

    #[test]
    fn test_validate_compiled_schema_cache() {
        let mut v = JsonSchemaValidator::new();
        let schema = object_schema();
        let params = serde_json::json!({"name": "Alice"});

        assert!(v.validate(&schema, &params).is_ok());
        assert!(v.is_cached(&schema));

        assert!(v.validate(&schema, &params).is_ok());
    }

    #[test]
    fn test_validate_empty_schema_accepts_all() {
        let mut v = JsonSchemaValidator::new();
        let schema = serde_json::json!({});
        let params = serde_json::json!({"anything": "goes", "number": 42});
        assert!(v.validate(&schema, &params).is_ok());
    }

    #[test]
    fn test_validate_nested_object() {
        let mut v = JsonSchemaValidator::new();
        let schema = nested_schema();
        let valid = serde_json::json!({
            "address": { "street": "123 Main St", "city": "Springfield" }
        });
        assert!(v.validate(&schema, &valid).is_ok());

        let invalid = serde_json::json!({
            "address": { "city": "Springfield" }
        });
        assert!(v.validate(&schema, &invalid).is_err());
    }
}
