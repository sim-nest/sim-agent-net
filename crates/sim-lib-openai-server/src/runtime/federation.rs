use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use serde_json::{Map, Number, Value as JsonValue};
use sim_kernel::{
    Consistency, Cx, Error, EvalFabricRef, EvalMode, EvalRequest, Expr, Result, Symbol,
};
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse, ModelUsage};
use sim_lib_net_core::hex_encode;

use sim_codec_chat::text_part;
use sim_codec_json::json_number_to_u64;

use crate::{
    capabilities::openai_gateway_federate_capability, objects::GatewayResponseValue,
    plan::fixtures::request_text,
};

/// Registry of remote gateways reachable for federated inference, keyed by address.
#[derive(Clone, Default)]
pub struct OpenAiFederation {
    gateways: Arc<Mutex<BTreeMap<String, OpenAiFederatedGateway>>>,
}

/// A single remote gateway: its `gateway/`-prefixed address, default model, and
/// the eval fabric used to reach it.
#[derive(Clone)]
pub struct OpenAiFederatedGateway {
    address: String,
    default_model: String,
    fabric: EvalFabricRef,
}

/// Policy applied to a federated request: optional privacy tier and budget entries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenAiFederationPolicy {
    privacy: Option<String>,
    budget: Vec<(String, Expr)>,
}

impl OpenAiFederation {
    /// Returns an empty federation registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a remote gateway under its normalized address.
    pub fn insert_gateway(
        &self,
        address: impl AsRef<str>,
        default_model: impl Into<String>,
        fabric: EvalFabricRef,
    ) -> Result<()> {
        let gateway = OpenAiFederatedGateway::new(address.as_ref(), default_model, fabric)?;
        self.gateways
            .lock()
            .map_err(|_| Error::PoisonedLock("openai federation registry"))?
            .insert(gateway.address.clone(), gateway);
        Ok(())
    }

    /// Runs `request` against the federated gateway at `address` and returns its response.
    ///
    /// Requires the federation capability, encodes the request (with `policy`)
    /// as a `/v1/responses` body, realizes it through the gateway's eval fabric,
    /// and decodes the reply; errors if the gateway is unknown, denies the
    /// capability, or returns a non-200 / non-response value.
    pub fn infer(
        &self,
        cx: &mut Cx,
        address: &str,
        request: &ModelRequest,
        policy: &OpenAiFederationPolicy,
    ) -> Result<ModelResponse> {
        cx.require(&openai_gateway_federate_capability())?;
        let gateway = self.gateway(address)?;
        let body = federated_request_body(&gateway.default_model, request, policy)?;
        let reply = gateway.fabric.realize(
            cx,
            EvalRequest {
                expr: Expr::Bytes(body),
                result_shape: None,
                required_capabilities: Vec::new(),
                deadline: None,
                consistency: Consistency::LocalFirst,
                mode: EvalMode::Eval,
                answer_limit: None,
                stream_buffer: None,
                stream: false,
                trace: false,
            },
        )?;
        let Some(response) = reply.value.object().downcast_ref::<GatewayResponseValue>() else {
            return Err(Error::Eval(format!(
                "federated gateway {address} returned a non-response value"
            )));
        };
        if response.response().status() != 200 {
            return Err(Error::Eval(format!(
                "federated gateway {address} returned status {}",
                response.response().status()
            )));
        }
        responses_body_to_model_response(address, response.response().body())
    }

    fn gateway(&self, address: &str) -> Result<OpenAiFederatedGateway> {
        let address = normalize_gateway_address(address)?;
        self.gateways
            .lock()
            .map_err(|_| Error::PoisonedLock("openai federation registry"))?
            .get(&address)
            .cloned()
            .ok_or_else(|| Error::Eval(format!("model_not_found: {address}")))
    }
}

impl OpenAiFederatedGateway {
    /// Returns a federated gateway after validating and normalizing `address`.
    pub fn new(
        address: impl AsRef<str>,
        default_model: impl Into<String>,
        fabric: EvalFabricRef,
    ) -> Result<Self> {
        Ok(Self {
            address: normalize_gateway_address(address.as_ref())?,
            default_model: default_model.into(),
            fabric,
        })
    }
}

impl OpenAiFederationPolicy {
    /// Returns a federation policy with the given privacy tier and budget entries.
    pub fn new(privacy: Option<String>, budget: Vec<(String, Expr)>) -> Self {
        Self { privacy, budget }
    }

    /// Returns a copy of the policy with `entries` appended to its budget.
    pub fn with_budget_entries(&self, entries: Vec<(String, Expr)>) -> Self {
        let mut budget = self.budget.clone();
        budget.extend(entries);
        Self {
            privacy: self.privacy.clone(),
            budget,
        }
    }
}

fn normalize_gateway_address(address: &str) -> Result<String> {
    let trimmed = address.trim();
    let Some(target) = trimmed.strip_prefix("gateway/") else {
        return Err(Error::Eval(format!(
            "federated gateway address must start with gateway/, found {address}"
        )));
    };
    if target.is_empty() {
        return Err(Error::Eval("federated gateway address is empty".to_owned()));
    }
    Ok(format!("gateway/{target}"))
}

fn federated_request_body(
    model: &str,
    request: &ModelRequest,
    policy: &OpenAiFederationPolicy,
) -> Result<Vec<u8>> {
    let mut object = Map::new();
    object.insert("model".to_owned(), JsonValue::String(model.to_owned()));
    object.insert("input".to_owned(), JsonValue::String(request_text(request)));
    object.insert("store".to_owned(), JsonValue::Bool(false));
    if let Some(privacy) = &policy.privacy {
        object.insert("privacy".to_owned(), JsonValue::String(privacy.clone()));
    }
    let budget = budget_json(&policy.budget);
    if !budget.is_empty() {
        object.insert("budget".to_owned(), JsonValue::Object(budget));
    }
    serde_json::to_vec(&JsonValue::Object(object))
        .map_err(|err| Error::Eval(format!("failed to encode federated gateway request: {err}")))
}

fn budget_json(entries: &[(String, Expr)]) -> Map<String, JsonValue> {
    let mut object = Map::new();
    for (key, value) in entries {
        object.insert(key.clone(), expr_to_json(value));
    }
    object
}

// Intentionally-divergent untagged Expr->JSON projection: this module's wire
// form differs from sim_codec_json::project_expr_to_json (and from the sibling
// forks), so it is kept local on purpose -- not a deletable fork. See OVERLAP_5.
fn expr_to_json(expr: &Expr) -> JsonValue {
    match expr {
        Expr::Nil => JsonValue::Null,
        Expr::Bool(value) => JsonValue::Bool(*value),
        Expr::String(value) => JsonValue::String(value.clone()),
        Expr::Symbol(value) if value.namespace.is_none() => {
            JsonValue::String(value.name.as_ref().to_owned())
        }
        Expr::Number(value) => number_literal_json(&value.canonical),
        Expr::List(items) => match items.as_slice() {
            [Expr::Symbol(head), Expr::String(value)]
                if head.namespace.is_none() && head.name.as_ref() == "plan/atom" =>
            {
                atom_literal_json(value)
            }
            _ => JsonValue::Array(items.iter().map(expr_to_json).collect()),
        },
        Expr::Map(entries) => JsonValue::Object(
            entries
                .iter()
                .map(|(key, value)| (json_key(key), expr_to_json(value)))
                .collect(),
        ),
        Expr::Bytes(bytes) => JsonValue::String(hex_encode(bytes)),
        _ => JsonValue::String(format!("{expr:?}")),
    }
}

fn json_key(expr: &Expr) -> String {
    match expr {
        Expr::String(value) => value.clone(),
        Expr::Symbol(value) if value.namespace.is_none() => value.name.as_ref().to_owned(),
        Expr::Symbol(value) => format!(
            "{}/{}",
            value.namespace.as_deref().unwrap_or(""),
            value.name
        ),
        _ => format!("{expr:?}"),
    }
}

fn atom_literal_json(value: &str) -> JsonValue {
    match value {
        "true" => JsonValue::Bool(true),
        "false" => JsonValue::Bool(false),
        _ => number_literal_json(value),
    }
}

fn number_literal_json(value: &str) -> JsonValue {
    if let Ok(unsigned) = value.parse::<u64>() {
        return JsonValue::Number(Number::from(unsigned));
    }
    value
        .parse::<f64>()
        .ok()
        .and_then(Number::from_f64)
        .map(JsonValue::Number)
        .unwrap_or_else(|| JsonValue::String(value.to_owned()))
}

fn responses_body_to_model_response(address: &str, body: &[u8]) -> Result<ModelResponse> {
    let value: JsonValue = serde_json::from_slice(body)
        .map_err(|err| Error::Eval(format!("federated gateway returned invalid json: {err}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| Error::Eval("federated gateway response must be an object".to_owned()))?;
    let output_text = object
        .get("output_text")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::Eval("federated gateway response missing output_text".to_owned()))?;
    let mut response = ModelResponse::new(
        Symbol::new(address.to_owned()),
        address,
        vec![text_part(output_text)],
        Symbol::new("stop"),
    );
    response.usage = object.get("usage").map(usage_from_json).transpose()?;
    if let Some(model) = object.get("model").and_then(JsonValue::as_str) {
        response.extra.push((
            Expr::Symbol(Symbol::new("federated-model")),
            Expr::String(model.to_owned()),
        ));
    }
    Ok(response)
}

fn usage_from_json(value: &JsonValue) -> Result<ModelUsage> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::Eval("federated gateway usage must be an object".to_owned()))?;
    Ok(ModelUsage {
        input_tokens: object.get("prompt_tokens").and_then(json_number_to_u64),
        output_tokens: object.get("completion_tokens").and_then(json_number_to_u64),
        latency_ms: None,
        cost_usd: None,
        extra: Vec::new(),
    })
}
