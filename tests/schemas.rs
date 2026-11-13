//! Every tool's parameter schema must stay inside the JSON Schema subset that Gemini-based
//! (Antigravity) and OpenAI-based (Codex) clients accept.
mod common;

use common::Harness;
use serde_json::Value;

const UNPORTABLE_KEYS: [&str; 8] = [
    "$ref", "$defs", "$schema", "oneOf", "anyOf", "allOf", "const", "format",
];

fn problems(value: &Value, path: &str, out: &mut Vec<String>) {
    match value {
        Value::Object(m) => {
            for (k, v) in m {
                // Property names are data, not keywords.
                let keyword = !path.ends_with("/properties");
                if keyword && UNPORTABLE_KEYS.contains(&k.as_str()) {
                    out.push(format!("{path}/{k}"));
                }
                if keyword && k == "type" && v.is_array() {
                    out.push(format!("{path}/type is a list"));
                }
                if keyword && k == "additionalProperties" && v.is_object() {
                    out.push(format!("{path}/additionalProperties (free-form map)"));
                }
                problems(v, &format!("{path}/{k}"), out);
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                problems(v, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn every_tool_schema_is_portable() {
    let h = Harness::new().await;
    let tools = h.tools().await;
    assert_eq!(tools.len(), 47);
    let mut found = Vec::new();
    for tool in &tools {
        let schema = Value::Object((*tool.input_schema).clone());
        assert_eq!(schema["type"], "object", "{}", tool.name);
        problems(&schema, &tool.name, &mut found);
    }
    assert!(
        found.is_empty(),
        "unportable schema parts:\n{}",
        found.join("\n")
    );
}
