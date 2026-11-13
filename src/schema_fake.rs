// Schema-valid fake JSON.
//
// Generates a value that validates against a JSON Schema, for structured
// outputs (`response_format: json_schema`, Anthropic `output_config.format`)
// and for tool-call arguments when a request forces a tool.
//
// Decisions:
// - Deterministic: the same schema and seed give the same value, so tests can
//   assert on it. Callers seed from the prompt.
// - Covers the subset structured-output APIs accept: `type` (single or list),
//   `properties`/`required`/`additionalProperties`, `items`/`prefixItems`,
//   `enum`, `const`, `anyOf`/`oneOf` (first branch), `allOf` (merged
//   properties), local `$ref` (`#/$defs/...`, `#/definitions/...`), string
//   `format` (date-time, date, email, uri, uuid), `minLength`/`maxLength`,
//   `pattern` is ignored, numeric bounds, `minItems`/`maxItems`.
// - Optional properties are included (strict mode makes every property
//   required anyway), so clients see the full shape.
// - Recursion is cut at depth 8 with the cheapest valid value (null when
//   allowed, otherwise an empty container).

use serde_json::{json, Map, Value};

const MAX_DEPTH: usize = 8;
const WORDS: &[&str] = &[
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
    "kilo", "lima", "mike", "november", "oscar", "papa",
];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
}

/// FNV-1a over `bytes`, for seeding from prompt text.
pub fn seed_from(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A value valid against `schema`, chosen deterministically from `seed`.
pub fn fake_from_schema(schema: &Value, seed: u64) -> Value {
    let mut rng = Rng(seed);
    gen(schema, schema, &mut rng, 0)
}

fn resolve<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    let mut current = schema;
    for _ in 0..16 {
        let Some(r) = current.get("$ref").and_then(Value::as_str) else {
            return current;
        };
        let Some(path) = r.strip_prefix("#/") else {
            return current;
        };
        let mut target = root;
        for part in path.split('/') {
            match target.get(part) {
                Some(t) => target = t,
                None => return current,
            }
        }
        current = target;
    }
    current
}

fn types(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn gen(root: &Value, schema: &Value, rng: &mut Rng, depth: usize) -> Value {
    let schema = resolve(root, schema);
    if let Some(c) = schema.get("const") {
        return c.clone();
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        if !values.is_empty() {
            return values[rng.below(values.len() as u64) as usize].clone();
        }
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(branches) = schema.get(key).and_then(Value::as_array) {
            // Prefer a non-null branch so nullable fields still show data.
            let pick = branches
                .iter()
                .find(|b| resolve(root, b).get("type") != Some(&json!("null")))
                .or_else(|| branches.first());
            if let Some(b) = pick {
                return gen(root, b, rng, depth + 1);
            }
        }
    }
    if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
        let mut merged = Map::new();
        let mut properties = Map::new();
        let mut required = Vec::new();
        for p in parts {
            let p = resolve(root, p);
            if let Some(props) = p.get("properties").and_then(Value::as_object) {
                properties.extend(props.clone());
            }
            if let Some(r) = p.get("required").and_then(Value::as_array) {
                required.extend(r.clone());
            }
        }
        merged.insert("type".into(), json!("object"));
        merged.insert("properties".into(), Value::Object(properties));
        merged.insert("required".into(), Value::Array(required));
        return gen(root, &Value::Object(merged), rng, depth + 1);
    }

    let ts = types(schema);
    let t = ts
        .iter()
        .copied()
        .find(|t| *t != "null")
        .or_else(|| ts.first().copied())
        .unwrap_or_else(|| {
            if schema.get("properties").is_some() {
                "object"
            } else if schema.get("items").is_some() {
                "array"
            } else {
                "string"
            }
        });
    if depth >= MAX_DEPTH {
        return if ts.contains(&"null") {
            Value::Null
        } else {
            match t {
                "object" => json!({}),
                "array" => json!([]),
                _ => scalar(t, schema, rng),
            }
        };
    }
    match t {
        "object" => {
            let mut out = Map::new();
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (name, sub) in props {
                    out.insert(name.clone(), gen(root, sub, rng, depth + 1));
                }
            }
            Value::Object(out)
        }
        "array" => {
            let min = schema.get("minItems").and_then(Value::as_u64).unwrap_or(1);
            let max = schema
                .get("maxItems")
                .and_then(Value::as_u64)
                .unwrap_or(min.max(3))
                .max(min);
            let mut items = Vec::new();
            if let Some(prefix) = schema.get("prefixItems").and_then(Value::as_array) {
                for p in prefix {
                    items.push(gen(root, p, rng, depth + 1));
                }
            }
            let n = (min + rng.below(max - min + 1)).max(items.len() as u64);
            let item_schema = schema.get("items").cloned().unwrap_or(json!({}));
            while (items.len() as u64) < n {
                items.push(gen(root, &item_schema, rng, depth + 1));
            }
            Value::Array(items)
        }
        other => scalar(other, schema, rng),
    }
}

fn scalar(t: &str, schema: &Value, rng: &mut Rng) -> Value {
    match t {
        "null" => Value::Null,
        "boolean" => Value::Bool(rng.below(2) == 1),
        "integer" => {
            let lo = schema.get("minimum").and_then(Value::as_i64).unwrap_or(0);
            let hi = schema
                .get("maximum")
                .and_then(Value::as_i64)
                .unwrap_or(lo.saturating_add(100))
                .max(lo);
            json!(lo + rng.below((hi - lo) as u64 + 1) as i64)
        }
        "number" => {
            let lo = schema.get("minimum").and_then(Value::as_f64).unwrap_or(0.0);
            let hi = schema
                .get("maximum")
                .and_then(Value::as_f64)
                .unwrap_or(lo + 100.0)
                .max(lo);
            let frac = rng.below(10_000) as f64 / 10_000.0;
            json!(((lo + (hi - lo) * frac) * 100.0).round() / 100.0)
        }
        _ => Value::String(string(schema, rng)),
    }
}

fn string(schema: &Value, rng: &mut Rng) -> String {
    let n = rng.next();
    let base = match schema.get("format").and_then(Value::as_str) {
        Some("date-time") => format!("2026-{:02}-{:02}T12:00:00Z", 1 + n % 12, 1 + n % 28),
        Some("date") => format!("2026-{:02}-{:02}", 1 + n % 12, 1 + n % 28),
        Some("time") => format!("{:02}:{:02}:00", n % 24, n % 60),
        Some("email") => format!("{}@example.com", WORDS[(n % WORDS.len() as u64) as usize]),
        Some("uri") | Some("url") => {
            format!(
                "https://example.com/{}",
                WORDS[(n % WORDS.len() as u64) as usize]
            )
        }
        Some("uuid") => format!(
            "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
            n as u32,
            (n >> 32) as u16,
            (n >> 48) & 0xfff,
            (n >> 20) & 0xfff,
            n & 0xffff_ffff_ffff
        ),
        _ => {
            let words = 1 + rng.below(3);
            (0..words)
                .map(|_| WORDS[rng.below(WORDS.len() as u64) as usize])
                .collect::<Vec<_>>()
                .join(" ")
        }
    };
    let min = schema.get("minLength").and_then(Value::as_u64).unwrap_or(0) as usize;
    let max = schema
        .get("maxLength")
        .and_then(Value::as_u64)
        .map(|m| m as usize);
    let mut s = base;
    while s.chars().count() < min {
        s.push('x');
    }
    if let Some(max) = max {
        s = s.chars().take(max).collect();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_with_required_fields_and_types() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer", "minimum": 18, "maximum": 30},
                "score": {"type": "number"},
                "tags": {"type": "array", "items": {"type": "string"}, "minItems": 2, "maxItems": 2},
                "kind": {"enum": ["a", "b"]},
                "when": {"type": "string", "format": "date-time"},
                "note": {"type": ["string", "null"]}
            },
            "required": ["name", "age", "score", "tags", "kind", "when", "note"],
            "additionalProperties": false
        });
        let v = fake_from_schema(&schema, 7);
        assert!(v["name"].is_string());
        let age = v["age"].as_i64().unwrap();
        assert!((18..=30).contains(&age));
        assert!(v["score"].is_number());
        assert_eq!(v["tags"].as_array().unwrap().len(), 2);
        assert!(["a", "b"].contains(&v["kind"].as_str().unwrap()));
        assert!(v["when"].as_str().unwrap().ends_with('Z'));
        assert!(v["note"].is_string());
        // Deterministic.
        assert_eq!(v, fake_from_schema(&schema, 7));
    }

    #[test]
    fn refs_anyof_and_recursion() {
        let schema = json!({
            "$defs": {"node": {"type": "object", "properties": {
                "value": {"type": "integer"},
                "children": {"type": "array", "items": {"$ref": "#/$defs/node"}}
            }}},
            "type": "object",
            "properties": {
                "root": {"$ref": "#/$defs/node"},
                "either": {"anyOf": [{"type": "null"}, {"type": "boolean"}]},
                "fixed": {"const": 42}
            }
        });
        let v = fake_from_schema(&schema, 1);
        assert!(v["root"]["value"].is_i64());
        assert!(v["either"].is_boolean());
        assert_eq!(v["fixed"], 42);
    }

    #[test]
    fn string_lengths() {
        let v = fake_from_schema(
            &json!({"type": "string", "minLength": 40, "maxLength": 45}),
            3,
        );
        let n = v.as_str().unwrap().len();
        assert!((40..=45).contains(&n));
    }
}
