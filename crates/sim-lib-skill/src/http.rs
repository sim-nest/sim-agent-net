//! HTTP/OpenAPI skill transport.
//!
//! Provides [`FixtureHttpTransport`], a deterministic transport that derives
//! skill cards from a subset of an OpenAPI document and serves canned JSON
//! responses, for skill discovery and tests. Gated behind the `http` feature.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use serde_json::{Map as JsonMap, Value as JsonValue};
use sim_codec_json::{JsonProjectionMode, project_json_to_expr};
use sim_kernel::{Cx, Error, Result, ShapeRef, Symbol, Value};
use sim_shape::{
    AnyShape, ExprKind, ExprKindShape, FieldShape, FieldSpec, ListShape, NumberValueShape, Shape,
    shape_value,
};

use crate::{
    SkillCard, SkillPolicy, SkillRole, SkillTransport, skill_specific_call_capability,
    skill_transport_value,
};

const HTTP_FIXTURE_KIND: &str = "http-fixture";

#[derive(Clone, Debug)]
struct HttpOperation {
    operation_id: String,
    title: String,
    description: String,
    request_schema: JsonValue,
    response_schema: JsonValue,
}

/// Deterministic HTTP/OpenAPI fixture transport for skill tests and discovery.
pub struct FixtureHttpTransport {
    id: String,
    operations: Vec<HttpOperation>,
    responses: BTreeMap<String, JsonValue>,
    calls: AtomicUsize,
}

impl FixtureHttpTransport {
    /// Builds a fixture transport from the supported OpenAPI JSON subset.
    pub fn from_openapi_value(id: impl Into<String>, document: JsonValue) -> Result<Self> {
        let id = id.into();
        Ok(Self {
            id,
            operations: operations_from_openapi(&document)?,
            responses: BTreeMap::new(),
            calls: AtomicUsize::new(0),
        })
    }

    /// Registers a deterministic JSON response for one operation id.
    pub fn with_response(mut self, operation_id: impl Into<String>, response: JsonValue) -> Self {
        self.responses.insert(operation_id.into(), response);
        self
    }

    /// Returns the number of fixture calls that reached the transport.
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Wraps this transport as a normal skill transport value.
    pub fn value(self: Arc<Self>, cx: &mut Cx) -> Result<Value> {
        skill_transport_value(cx, self)
    }
}

impl SkillTransport for FixtureHttpTransport {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &str {
        HTTP_FIXTURE_KIND
    }

    fn discover(&self, _cx: &mut Cx) -> Result<Vec<SkillCard>> {
        self.operations
            .iter()
            .map(|operation| card_for_operation(&self.id, operation))
            .collect()
    }

    fn call(
        &self,
        cx: &mut Cx,
        card: &SkillCard,
        _args: Value,
        _events: Option<&mut dyn crate::SkillEventSink>,
    ) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let response = self.responses.get(&card.operation).ok_or_else(|| {
            Error::Eval(format!(
                "http fixture {} has no response for operation {}",
                self.id, card.operation
            ))
        })?;
        cx.factory().expr(project_json_to_expr(
            response,
            JsonProjectionMode::UntaggedInterop,
        ))
    }

    fn health(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("skill/http-health"))?,
            ),
            (Symbol::new("id"), cx.factory().string(self.id.clone())?),
            (
                Symbol::new("transport-kind"),
                cx.factory().string(HTTP_FIXTURE_KIND.to_owned())?,
            ),
            (
                Symbol::new("operations"),
                cx.factory().string(self.operations.len().to_string())?,
            ),
            (
                Symbol::new("calls"),
                cx.factory().string(self.call_count().to_string())?,
            ),
        ])
    }
}

fn card_for_operation(transport_id: &str, operation: &HttpOperation) -> Result<SkillCard> {
    let id = format!("{transport_id}.{}", operation.operation_id);
    Ok(SkillCard {
        id: id.clone(),
        symbol: Symbol::qualified("skill", id.clone()),
        aliases: vec![Symbol::qualified("http", operation.operation_id.clone())],
        origin: Symbol::new("openapi"),
        title: operation.title.clone(),
        description: operation.description.clone(),
        input_shape: operation_input_shape(&id, &operation.request_schema)?,
        output_shape: operation_output_shape(&id, &operation.response_schema)?,
        roles: vec![SkillRole::Tool],
        capabilities: vec![skill_specific_call_capability(&id)],
        policy: SkillPolicy::default(),
        transport_id: transport_id.to_owned(),
        transport_kind: HTTP_FIXTURE_KIND.to_owned(),
        operation: operation.operation_id.clone(),
    })
}

fn operation_input_shape(id: &str, schema: &JsonValue) -> Result<ShapeRef> {
    let shape = json_schema_shape(schema)?;
    Ok(shape_value(
        Symbol::qualified("skill-http", format!("{id}-args")),
        Arc::new(ListShape::new(vec![shape])),
    ))
}

fn operation_output_shape(id: &str, schema: &JsonValue) -> Result<ShapeRef> {
    Ok(shape_value(
        Symbol::qualified("skill-http", format!("{id}-result")),
        json_schema_shape(schema)?,
    ))
}

fn operations_from_openapi(document: &JsonValue) -> Result<Vec<HttpOperation>> {
    let paths = object_field(document, "paths", "OpenAPI document paths")?;
    let mut operations = Vec::new();
    for (path, path_item) in paths {
        let path_item = path_item
            .as_object()
            .ok_or_else(|| Error::Eval(format!("OpenAPI path item {path} must be an object")))?;
        for method in ["get", "post", "put", "patch", "delete"] {
            let Some(operation) = path_item.get(method) else {
                continue;
            };
            operations.push(operation_from_json(path, method, operation)?);
        }
    }
    if operations.is_empty() {
        return Err(Error::Eval(
            "OpenAPI fixture contains no supported operations".to_owned(),
        ));
    }
    Ok(operations)
}

fn operation_from_json(path: &str, method: &str, operation: &JsonValue) -> Result<HttpOperation> {
    let operation = operation.as_object().ok_or_else(|| {
        Error::Eval(format!(
            "OpenAPI {method} {path} operation must be an object"
        ))
    })?;
    let operation_id = string_field(operation, "operationId", "OpenAPI operation id")?;
    let summary = optional_string_field(operation, "summary")
        .or_else(|| optional_string_field(operation, "description"))
        .unwrap_or_else(|| operation_id.clone());
    let description =
        optional_string_field(operation, "description").unwrap_or_else(|| summary.clone());
    Ok(HttpOperation {
        operation_id,
        title: summary,
        description: format!("{method} {path}: {description}"),
        request_schema: request_schema(operation),
        response_schema: response_schema(operation)?,
    })
}

fn request_schema(operation: &JsonMap<String, JsonValue>) -> JsonValue {
    operation
        .get("requestBody")
        .and_then(|body| body.get("content"))
        .and_then(|content| content.get("application/json"))
        .and_then(|json| json.get("schema"))
        .cloned()
        .unwrap_or(JsonValue::Object(JsonMap::new()))
}

fn response_schema(operation: &JsonMap<String, JsonValue>) -> Result<JsonValue> {
    operation
        .get("responses")
        .and_then(|responses| responses.get("200"))
        .and_then(|response| response.get("content"))
        .and_then(|content| content.get("application/json"))
        .and_then(|json| json.get("schema"))
        .cloned()
        .ok_or_else(|| Error::Eval("OpenAPI operation missing JSON 200 response schema".to_owned()))
}

fn json_schema_shape(schema: &JsonValue) -> Result<Arc<dyn Shape>> {
    match schema.get("type").and_then(JsonValue::as_str) {
        Some("object") => object_schema_shape(schema),
        Some("array") => array_schema_shape(schema),
        Some("string") => Ok(Arc::new(ExprKindShape::new(ExprKind::String))),
        Some("number" | "integer") => Ok(Arc::new(NumberValueShape)),
        Some("boolean") => Ok(Arc::new(ExprKindShape::new(ExprKind::Bool))),
        Some("null") => Ok(Arc::new(ExprKindShape::new(ExprKind::Nil))),
        Some(_) | None => Ok(Arc::new(AnyShape)),
    }
}

fn object_schema_shape(schema: &JsonValue) -> Result<Arc<dyn Shape>> {
    let properties = schema
        .get("properties")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let required = schema
        .get("required")
        .and_then(JsonValue::as_array)
        .map(|values| required_names(values))
        .transpose()?
        .unwrap_or_default();
    let specs = required
        .iter()
        .map(|name| {
            Ok(FieldSpec::required(
                Symbol::new(name.clone()),
                json_schema_shape(properties.get(name).unwrap_or(&JsonValue::Null))?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Arc::new(FieldShape::anonymous(specs)))
}

fn array_schema_shape(schema: &JsonValue) -> Result<Arc<dyn Shape>> {
    let item_shape = schema
        .get("items")
        .map(json_schema_shape)
        .transpose()?
        .unwrap_or_else(|| Arc::new(AnyShape) as Arc<dyn Shape>);
    Ok(Arc::new(ListShape::with_rest(Vec::new(), item_shape)))
}

fn required_names(values: &[JsonValue]) -> Result<Vec<String>> {
    let mut seen = BTreeSet::new();
    let mut names = Vec::new();
    for value in values {
        let Some(name) = value.as_str() else {
            return Err(Error::Eval(
                "OpenAPI schema required entries must be strings".to_owned(),
            ));
        };
        if seen.insert(name.to_owned()) {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

fn object_field<'a>(
    value: &'a JsonValue,
    field: &str,
    expected: &'static str,
) -> Result<&'a JsonMap<String, JsonValue>> {
    value
        .get(field)
        .and_then(JsonValue::as_object)
        .ok_or_else(|| Error::Eval(format!("{expected} must be an object at field {field}")))
}

fn string_field(
    fields: &JsonMap<String, JsonValue>,
    field: &str,
    expected: &'static str,
) -> Result<String> {
    fields
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error::Eval(format!("{expected} missing field {field}")))
}

fn optional_string_field(fields: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    fields
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests;
