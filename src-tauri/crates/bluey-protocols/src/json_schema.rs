//! JSON Schema normalisation shared by the provider codecs.
//!
//! The WebView builds every structured-output schema with zod v4's
//! `toJSONSchema` (`src/modes/schemas.ts`, `src/ai/engine.ts`,
//! `src/sessions/summary.ts`). That output carries a top-level `$schema` and
//! lists only the non-optional properties in `required`. Providers differ in
//! what they accept:
//!
//! * Gemini / Cloud Code reject unknown keywords such as `$schema` but take
//!   optional properties as they are → [`strip_meta`].
//! * OpenAI's *strict* structured outputs (the Responses API `text.format`
//!   behind ChatGPT / Codex, chat-completions `response_format` on Foundry and
//!   OpenAI-compatible endpoints) additionally require **every** property to be
//!   listed in `required` and every object to carry `additionalProperties:
//!   false`; an optional property must be expressed as a required *nullable*
//!   one (`"type": ["string", "null"]`). A schema that breaks these rules is
//!   answered with HTTP 400 (`Invalid schema for response_format … 'required'
//!   is required to be supplied and to be an array including every key in
//!   properties`) — Bluey's `ai.invalid_request` → [`strict_variant`].
//!
//! The frontend parsers treat `null` like an absent field, so the nullable
//! form changes nothing downstream.

use serde_json::{json, Map, Value};

/// Remove the top-level `$schema` key; everything else is forwarded untouched.
pub fn strip_meta(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut cleaned = map.clone();
            cleaned.remove("$schema");
            Value::Object(cleaned)
        }
        other => other.clone(),
    }
}

/// The schema as OpenAI's strict mode wants it: no `$schema`, every object
/// with `additionalProperties: false` and all of its properties `required`,
/// the previously optional ones made nullable. Idempotent; leaves value
/// constraints (`enum`, `const`, `minimum`, `maximum`, …) in place.
pub fn strict_variant(schema: &Value) -> Value {
    let mut out = strip_meta(schema);
    strictify(&mut out);
    out
}

fn strictify(node: &mut Value) {
    let Value::Object(map) = node else {
        return;
    };
    for key in ["anyOf", "oneOf", "allOf", "prefixItems"] {
        if let Some(Value::Array(items)) = map.get_mut(key) {
            for item in items {
                strictify(item);
            }
        }
    }
    for key in ["items", "not", "if", "then", "else"] {
        if let Some(child) = map.get_mut(key) {
            strictify(child);
        }
    }
    for key in ["$defs", "definitions"] {
        if let Some(Value::Object(definitions)) = map.get_mut(key) {
            for definition in definitions.values_mut() {
                strictify(definition);
            }
        }
    }
    let is_object = map.get("type").is_some_and(|t| type_includes(t, "object"))
        || map.contains_key("properties");
    if !is_object {
        return;
    }
    let originally_required: Vec<String> = map
        .get("required")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut required = Vec::new();
    if let Some(Value::Object(properties)) = map.get_mut("properties") {
        for (name, property) in properties.iter_mut() {
            strictify(property);
            if !originally_required.iter().any(|r| r == name) {
                make_nullable(property);
            }
            required.push(Value::String(name.clone()));
        }
    }
    map.insert("required".into(), Value::Array(required));
    map.insert("additionalProperties".into(), Value::Bool(false));
}

fn type_includes(declared: &Value, wanted: &str) -> bool {
    match declared {
        Value::String(name) => name == wanted,
        Value::Array(names) => names.iter().any(|n| n.as_str() == Some(wanted)),
        _ => false,
    }
}

fn is_null_schema(schema: &Value) -> bool {
    schema
        .get("type")
        .is_some_and(|t| t.as_str() == Some("null"))
}

/// Let `property` accept `null` as well, in the form strict mode understands.
fn make_nullable(property: &mut Value) {
    let Value::Object(map) = property else {
        return; // `true` / non-object schemas already accept null
    };
    if map.contains_key("const") {
        // `const` pins a single value; a null alternative needs a union.
        let original = Value::Object(std::mem::take(map));
        *property = json!({ "anyOf": [original, { "type": "null" }] });
        return;
    }
    if let Some(declared) = map.get_mut("type") {
        match declared {
            Value::String(name) if name == "null" => {}
            Value::String(name) => {
                let name = name.clone();
                *declared = json!([name, "null"]);
            }
            Value::Array(names) if !names.iter().any(|n| n.as_str() == Some("null")) => {
                names.push(json!("null"));
            }
            _ => {}
        }
        if let Some(Value::Array(values)) = map.get_mut("enum") {
            if !values.iter().any(Value::is_null) {
                values.push(Value::Null);
            }
        }
        return;
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(Value::Array(variants)) = map.get_mut(key) {
            if !variants.iter().any(is_null_schema) {
                variants.push(json!({ "type": "null" }));
            }
            return;
        }
    }
    // `$ref`, `allOf`, bare `enum` without a type, …: wrap in a union.
    let original = Value::Object(std::mem::take(map));
    *property = json!({ "anyOf": [original, { "type": "null" }] });
}

/// Property names of an object schema, in declaration order (tests, logging).
pub fn property_names(schema: &Value) -> Vec<String> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(Map::keys)
        .map(|keys| keys.cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// What zod 4.5.4 emits for `outputSchemaFor("coding")` (trimmed).
    fn zod_coding_schema() -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "responseType": { "type": "string", "const": "code" },
                "title": { "type": "string" },
                "content": { "type": "string" },
                "sections": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string", "enum": ["Approach", "Solution"] },
                            "content": { "type": "string" },
                            "kind": { "type": "string", "enum": ["text", "list", "code"] },
                            "language": { "type": "string" }
                        },
                        "required": ["title", "content"],
                        "additionalProperties": false
                    }
                },
                "code": {
                    "type": "object",
                    "properties": {
                        "language": { "type": "string" },
                        "code": { "type": "string" },
                        "filename": { "type": "string" }
                    },
                    "required": ["language", "code"],
                    "additionalProperties": false
                },
                "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
            },
            "required": ["responseType", "content"],
            "additionalProperties": false
        })
    }

    #[test]
    fn strip_meta_drops_only_the_top_level_schema_keyword() {
        let cleaned = strip_meta(&json!({
            "$schema": "x",
            "type": "object",
            "properties": { "nested": { "$schema": "keep-me" } }
        }));
        assert!(cleaned.get("$schema").is_none());
        assert_eq!(cleaned["properties"]["nested"]["$schema"], "keep-me");
        assert_eq!(strip_meta(&json!(true)), json!(true));
    }

    #[test]
    fn strict_variant_requires_every_property_and_makes_optionals_nullable() {
        let strict = strict_variant(&zod_coding_schema());
        assert!(strict.get("$schema").is_none());
        // `serde_json::Map` keeps keys sorted, so `required` follows `properties` order.
        assert_eq!(
            strict["required"],
            json!([
                "code",
                "confidence",
                "content",
                "responseType",
                "sections",
                "title"
            ]),
            "every property"
        );
        assert_eq!(strict["additionalProperties"], false);
        // Required properties keep their exact shape.
        assert_eq!(
            strict["properties"]["responseType"],
            json!({ "type": "string", "const": "code" })
        );
        assert_eq!(strict["properties"]["content"], json!({ "type": "string" }));
        // Optional ones become nullable, constraints intact.
        assert_eq!(
            strict["properties"]["title"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(
            strict["properties"]["confidence"],
            json!({ "type": ["number", "null"], "minimum": 0, "maximum": 1 })
        );
        assert_eq!(
            strict["properties"]["sections"]["type"],
            json!(["array", "null"])
        );
        assert_eq!(
            strict["properties"]["code"]["type"],
            json!(["object", "null"])
        );
    }

    #[test]
    fn strict_variant_recurses_into_array_items_and_nested_objects() {
        let strict = strict_variant(&zod_coding_schema());
        let section = &strict["properties"]["sections"]["items"];
        assert_eq!(
            section["required"],
            json!(["content", "kind", "language", "title"])
        );
        assert_eq!(section["additionalProperties"], false);
        assert_eq!(
            section["properties"]["kind"],
            json!({ "type": ["string", "null"], "enum": ["text", "list", "code", null] }),
            "an optional enum gains a null member"
        );
        assert_eq!(
            section["properties"]["title"]["enum"],
            json!(["Approach", "Solution"]),
            "a required enum is untouched"
        );
        let code = &strict["properties"]["code"];
        assert_eq!(code["required"], json!(["code", "filename", "language"]));
        assert_eq!(
            code["properties"]["filename"]["type"],
            json!(["string", "null"])
        );
    }

    #[test]
    fn strict_variant_is_idempotent() {
        let once = strict_variant(&zod_coding_schema());
        assert_eq!(strict_variant(&once), once);
    }

    #[test]
    fn unions_refs_and_consts_get_a_null_alternative() {
        let strict = strict_variant(&json!({
            "type": "object",
            "properties": {
                "either": { "anyOf": [{ "type": "string" }, { "type": "number" }] },
                "already": { "anyOf": [{ "type": "string" }, { "type": "null" }] },
                "linked": { "$ref": "#/$defs/thing" },
                "fixed": { "const": "x" },
                "multi": { "type": ["string", "number"] }
            },
            "$defs": { "thing": { "type": "object", "properties": { "a": { "type": "string" } } } }
        }));
        assert_eq!(
            strict["properties"]["either"]["anyOf"],
            json!([{ "type": "string" }, { "type": "number" }, { "type": "null" }])
        );
        assert_eq!(
            strict["properties"]["already"]["anyOf"],
            json!([{ "type": "string" }, { "type": "null" }]),
            "no duplicate null variant"
        );
        assert_eq!(
            strict["properties"]["linked"],
            json!({ "anyOf": [{ "$ref": "#/$defs/thing" }, { "type": "null" }] })
        );
        assert_eq!(
            strict["properties"]["fixed"],
            json!({ "anyOf": [{ "const": "x" }, { "type": "null" }] })
        );
        assert_eq!(
            strict["properties"]["multi"]["type"],
            json!(["string", "number", "null"])
        );
        // Definitions are strict too.
        assert_eq!(strict["$defs"]["thing"]["required"], json!(["a"]));
        assert_eq!(strict["$defs"]["thing"]["additionalProperties"], false);
    }

    #[test]
    fn non_object_schemas_pass_through() {
        assert_eq!(
            strict_variant(&json!({ "type": "string", "enum": ["a", "b"] })),
            json!({ "type": "string", "enum": ["a", "b"] })
        );
        assert_eq!(
            strict_variant(&json!({ "type": "object" })),
            json!({ "type": "object", "required": [], "additionalProperties": false })
        );
        assert_eq!(property_names(&zod_coding_schema()).len(), 6);
    }
}
