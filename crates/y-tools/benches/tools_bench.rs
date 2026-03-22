//! Benchmarks for tool dispatch and JSON schema validation.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_json_schema_validation(c: &mut Criterion) {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "text": {"type": "string"},
            "count": {"type": "integer", "minimum": 0}
        },
        "required": ["text"]
    });

    let valid_input = serde_json::json!({
        "text": "hello world",
        "count": 42
    });

    let compiled = jsonschema::validator_for(&schema).unwrap();

    c.bench_function("json_schema_validate", |b| {
        b.iter(|| {
            let _result = compiled.validate(black_box(&valid_input));
        });
    });
}

fn bench_tool_dispatch(c: &mut Criterion) {
    use std::collections::HashMap;
    use y_core::runtime::RuntimeCapability;
    use y_core::tool::{ToolCategory, ToolDefinition, ToolType};
    use y_core::types::ToolName;

    // Simulate a tool registry with 50 tools
    let mut registry: HashMap<String, ToolDefinition> = HashMap::new();
    for i in 0..50 {
        let name = format!("tool_{i}");
        registry.insert(
            name.clone(),
            ToolDefinition {
                name: ToolName::from_string(&name),
                description: format!("Tool {i} description"),
                help: None,
                parameters: serde_json::json!({"type": "object"}),
                result_schema: None,
                category: ToolCategory::Custom,
                tool_type: ToolType::BuiltIn,
                capabilities: RuntimeCapability::default(),
                is_dangerous: false,
            },
        );
    }

    c.bench_function("tool_dispatch_lookup_50", |b| {
        b.iter(|| {
            // Simulate lookup + validation cycle
            let tool = registry.get(black_box("tool_25")).unwrap();
            let _name = &tool.name;
        });
    });
}

criterion_group!(benches, bench_json_schema_validation, bench_tool_dispatch);
criterion_main!(benches);
