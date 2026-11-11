//! Tool parameter schemas trimmed to the JSON Schema subset every major MCP client accepts.
//! Gemini-based clients (Antigravity) reject `$ref`/`$defs`, `oneOf`, `const`, nullable type
//! lists and most `format`s; OpenAI-based clients (Codex) are fine with the simpler form too.
//! Defaults are dropped as keywords; each parameter's description states its default.
use serde_json::{Map, Value};

/// A portable copy of `schema`: references inlined, string choices flattened to one `enum`,
/// `const` → single-value `enum`, `["T", "null"]` → `T`, and `$schema`/`format`/`default` removed.
pub fn portable(schema: &Map<String, Value>) -> Map<String, Value> {
    let defs = schema
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    simplify_object(schema, &defs)
}

fn simplify(value: &Value, defs: &Map<String, Value>) -> Value {
    match value {
        Value::Object(m) => Value::Object(simplify_object(m, defs)),
        Value::Array(items) => Value::Array(items.iter().map(|v| simplify(v, defs)).collect()),
        other => other.clone(),
    }
}

fn simplify_object(schema: &Map<String, Value>, defs: &Map<String, Value>) -> Map<String, Value> {
    let mut m = schema.clone();
    if let Some(Value::String(reference)) = m.remove("$ref")
        && let Some(Value::Object(def)) = reference
            .strip_prefix("#/$defs/")
            .and_then(|name| defs.get(name))
    {
        // Keywords next to the reference (e.g. the property's own description) win.
        for (k, v) in def {
            m.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    for key in ["$defs", "$schema", "format", "default"] {
        m.remove(key);
    }
    if let Some(c) = m.remove("const") {
        m.insert("enum".into(), Value::Array(vec![c]));
    }
    if let Some(Value::Array(types)) = m.get("type") {
        let non_null: Vec<&Value> = types
            .iter()
            .filter(|t| t.as_str() != Some("null"))
            .collect();
        if let [only] = non_null.as_slice() {
            let only = (*only).clone();
            m.insert("type".into(), only);
        }
    }

    // Recurse: `properties` maps names to schemas; every other object/array value is a schema.
    let mut out = Map::new();
    for (k, v) in m {
        let v = match (k.as_str(), &v) {
            ("properties", Value::Object(props)) => Value::Object(
                props
                    .iter()
                    .map(|(name, s)| (name.clone(), simplify(s, defs)))
                    .collect(),
            ),
            _ => simplify(&v, defs),
        };
        out.insert(k, v);
    }

    for key in ["oneOf", "anyOf"] {
        if let Some(Value::Array(branches)) = out.get(key)
            && let Some(values) = string_choices(branches)
        {
            out.remove(key);
            out.insert("type".into(), Value::String("string".into()));
            out.insert("enum".into(), Value::Array(values));
        }
    }
    out
}

/// All the strings allowed by branches that are each a string `enum`, or `None`.
fn string_choices(branches: &[Value]) -> Option<Vec<Value>> {
    let mut values = Vec::new();
    for branch in branches {
        let b = branch.as_object()?;
        if b.get("type").and_then(Value::as_str) != Some("string") {
            return None;
        }
        values.extend(b.get("enum")?.as_array()?.iter().cloned());
    }
    Some(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inlines_refs_and_flattens_choices() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": { "Scope": { "oneOf": [
                { "type": "string", "enum": ["unread", "sent"] },
                { "type": "string", "const": "inbox", "description": "Your inbox." }
            ] } },
            "type": "object",
            "properties": {
                "scope": { "$ref": "#/$defs/Scope", "description": "Which folder." },
                "limit": { "type": "integer", "format": "uint", "minimum": 0, "default": 30 },
                "course": { "type": ["string", "null"], "default": null },
                "format": { "type": "string", "description": "a property that happens to be named format" }
            },
            "required": ["limit"]
        });
        let out = Value::Object(portable(schema.as_object().unwrap()));
        assert_eq!(
            out,
            json!({
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "enum": ["unread", "sent", "inbox"], "description": "Which folder." },
                    "limit": { "type": "integer", "minimum": 0 },
                    "course": { "type": "string" },
                    "format": { "type": "string", "description": "a property that happens to be named format" }
                },
                "required": ["limit"]
            })
        );
    }

    #[test]
    fn leaves_mixed_unions_alone() {
        let schema = json!({ "oneOf": [{ "type": "string" }, { "type": "integer" }] });
        let out = Value::Object(portable(schema.as_object().unwrap()));
        assert_eq!(
            out,
            json!({ "oneOf": [{ "type": "string" }, { "type": "integer" }] })
        );
    }
}
