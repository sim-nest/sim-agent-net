use std::collections::BTreeSet;

use serde_json::Value;

use crate::ClientError;

/// Bounded JSON Schema contract used for imported callable input and output.
#[derive(Clone, Debug)]
pub struct SchemaContract {
    document: Value,
    maximum_depth: usize,
}

impl SchemaContract {
    /// Validates and retains a bounded schema document.
    pub fn new(
        document: Value,
        maximum_bytes: usize,
        maximum_depth: usize,
    ) -> Result<Self, ClientError> {
        if maximum_bytes == 0 || maximum_depth == 0 {
            return Err(ClientError::Schema("schema limits must be non-zero".into()));
        }
        let bytes = serde_json::to_vec(&document)
            .map_err(|error| ClientError::Schema(error.to_string()))?;
        if bytes.len() > maximum_bytes {
            return Err(ClientError::Schema("schema byte limit exceeded".into()));
        }
        validate_schema(&document, 0, maximum_depth)?;
        Ok(Self {
            document,
            maximum_depth,
        })
    }

    /// Validates one JSON value against the supported closed subset.
    pub fn validate(&self, value: &Value) -> Result<(), ClientError> {
        validate_instance(&self.document, value, 0, self.maximum_depth)
    }

    /// Original validated schema document.
    pub fn document(&self) -> &Value {
        &self.document
    }
}

fn validate_schema(schema: &Value, depth: usize, cap: usize) -> Result<(), ClientError> {
    if depth > cap {
        return Err(ClientError::Schema("schema depth limit exceeded".into()));
    }
    let object = schema
        .as_object()
        .ok_or_else(|| ClientError::Schema("schema must be an object".into()))?;
    if let Some(kind) = object.get("type") {
        let supported = [
            "object", "array", "string", "number", "integer", "boolean", "null",
        ];
        if !kind.as_str().is_some_and(|kind| supported.contains(&kind)) {
            return Err(ClientError::Schema("unsupported schema type".into()));
        }
    }
    if let Some(required) = object.get("required") {
        let values = required
            .as_array()
            .ok_or_else(|| ClientError::Schema("required must be an array".into()))?;
        let mut names = BTreeSet::new();
        for name in values {
            let name = name
                .as_str()
                .ok_or_else(|| ClientError::Schema("required names must be strings".into()))?;
            if !names.insert(name) {
                return Err(ClientError::Schema("duplicate required property".into()));
            }
        }
    }
    if let Some(properties) = object.get("properties") {
        for child in properties
            .as_object()
            .ok_or_else(|| ClientError::Schema("properties must be an object".into()))?
            .values()
        {
            validate_schema(child, depth + 1, cap)?;
        }
    }
    if let Some(items) = object.get("items") {
        validate_schema(items, depth + 1, cap)?;
    }
    if let Some(values) = object.get("enum") {
        if values.as_array().is_none_or(Vec::is_empty) {
            return Err(ClientError::Schema("enum must be a non-empty array".into()));
        }
    }
    Ok(())
}

fn validate_instance(
    schema: &Value,
    value: &Value,
    depth: usize,
    cap: usize,
) -> Result<(), ClientError> {
    if depth > cap {
        return Err(ClientError::Schema("instance depth limit exceeded".into()));
    }
    let object = schema.as_object().expect("validated schema");
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        if !values.contains(value) {
            return Err(ClientError::Schema("value is outside schema enum".into()));
        }
    }
    if let Some(kind) = object.get("type").and_then(Value::as_str) {
        let valid = match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        };
        if !valid {
            return Err(ClientError::Schema(format!("expected JSON {kind}")));
        }
    }
    if let Some(instance) = value.as_object() {
        if let Some(required) = object.get("required").and_then(Value::as_array) {
            for name in required.iter().filter_map(Value::as_str) {
                if !instance.contains_key(name) {
                    return Err(ClientError::Schema(format!(
                        "missing required property {name}"
                    )));
                }
            }
        }
        if let Some(properties) = object.get("properties").and_then(Value::as_object) {
            for (name, child) in properties {
                if let Some(value) = instance.get(name) {
                    validate_instance(child, value, depth + 1, cap)?;
                }
            }
        }
    }
    if let (Some(items), Some(values)) = (object.get("items"), value.as_array()) {
        for value in values {
            validate_instance(items, value, depth + 1, cap)?;
        }
    }
    Ok(())
}
