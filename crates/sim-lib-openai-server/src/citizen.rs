use sim_citizen_derive::Citizen;
use sim_kernel::{Error, Expr, Result, Symbol};

use crate::{
    content_id::content_id_for_expr,
    objects::{GatewayEvent, GatewayRequest, GatewayResponse, GatewayRun},
    runtime::OpenAiGatewayKey,
};
use sim_kernel::CapabilitySet;

/// Citizen descriptor wrapping a validated `openai/GatewayRequest` expression.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "openai/GatewayRequest", version = 1)]
pub struct OpenAiGatewayRequestDescriptor {
    #[citizen(with = "gateway_request_expr")]
    request: Expr,
}

/// Citizen descriptor wrapping a validated `openai/GatewayResponse` expression.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "openai/GatewayResponse", version = 1)]
pub struct OpenAiGatewayResponseDescriptor {
    #[citizen(with = "gateway_response_expr")]
    response: Expr,
}

/// Citizen descriptor wrapping a validated `openai/GatewayRun` expression.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "openai/GatewayRun", version = 1)]
pub struct OpenAiGatewayRunDescriptor {
    #[citizen(with = "gateway_run_expr")]
    run: Expr,
}

/// Citizen descriptor wrapping a validated `openai/GatewayEvent` expression.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "openai/GatewayEvent", version = 1)]
pub struct OpenAiGatewayEventDescriptor {
    #[citizen(with = "gateway_event_expr")]
    event: Expr,
}

/// Citizen descriptor wrapping a parseable `openai/Plan` source string.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "openai/Plan", version = 1)]
pub struct OpenAiPlanDescriptor {
    #[citizen(with = "plan_source")]
    source: String,
}

/// Citizen descriptor wrapping a validated `openai/GatewayKey` expression.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "openai/GatewayKey", version = 1)]
pub struct OpenAiGatewayKeyDescriptor {
    #[citizen(with = "gateway_key_expr")]
    key: Expr,
}

impl OpenAiGatewayRequestDescriptor {
    /// Validates and wraps a gateway request expression.
    pub fn from_expr(request: Expr) -> Result<Self> {
        gateway_request_expr::decode(&request)?;
        Ok(Self { request })
    }

    /// Returns the wrapped gateway request expression.
    pub fn as_expr(&self) -> &Expr {
        &self.request
    }
}

impl Default for OpenAiGatewayRequestDescriptor {
    fn default() -> Self {
        Self::from_expr(GatewayRequest::get("/v1/models").to_expr())
            .expect("default OpenAI request descriptor should be valid")
    }
}

impl OpenAiGatewayResponseDescriptor {
    /// Validates and wraps a gateway response expression.
    pub fn from_expr(response: Expr) -> Result<Self> {
        gateway_response_expr::decode(&response)?;
        Ok(Self { response })
    }

    /// Returns the wrapped gateway response expression.
    pub fn as_expr(&self) -> &Expr {
        &self.response
    }
}

impl Default for OpenAiGatewayResponseDescriptor {
    fn default() -> Self {
        Self::from_expr(GatewayResponse::json(200, br#"{"ok":true}"#.to_vec()).to_expr())
            .expect("default OpenAI response descriptor should be valid")
    }
}

impl OpenAiGatewayRunDescriptor {
    /// Validates and wraps a gateway run expression.
    pub fn from_expr(run: Expr) -> Result<Self> {
        gateway_run_expr::decode(&run)?;
        Ok(Self { run })
    }

    /// Returns the wrapped gateway run expression.
    pub fn as_expr(&self) -> &Expr {
        &self.run
    }
}

impl Default for OpenAiGatewayRunDescriptor {
    fn default() -> Self {
        let request_id = content_id_for_expr(&GatewayRequest::get("/v1/models").to_expr())
            .expect("default OpenAI run content id should be valid");
        Self::from_expr(GatewayRun::new("gwrun-citizen", request_id, 1).to_expr())
            .expect("default OpenAI run descriptor should be valid")
    }
}

impl OpenAiGatewayEventDescriptor {
    /// Validates and wraps a gateway event expression.
    pub fn from_expr(event: Expr) -> Result<Self> {
        gateway_event_expr::decode(&event)?;
        Ok(Self { event })
    }

    /// Returns the wrapped gateway event expression.
    pub fn as_expr(&self) -> &Expr {
        &self.event
    }
}

impl Default for OpenAiGatewayEventDescriptor {
    fn default() -> Self {
        Self::from_expr(
            GatewayEvent::new(
                "gwevt-citizen",
                "gwrun-citizen",
                0,
                Symbol::new("final"),
                Expr::String("ok".to_owned()),
                1,
            )
            .to_expr(),
        )
        .expect("default OpenAI event descriptor should be valid")
    }
}

impl OpenAiPlanDescriptor {
    /// Builds a descriptor from plan source text, validating that it parses.
    pub fn new(source: impl Into<String>) -> Result<Self> {
        let source = source.into();
        plan_source::decode(&Expr::String(source.clone()))?;
        Ok(Self { source })
    }

    /// Returns the wrapped plan source text.
    pub fn source(&self) -> &str {
        &self.source
    }
}

impl Default for OpenAiPlanDescriptor {
    fn default() -> Self {
        Self::new("fixture").expect("default OpenAI plan descriptor should be valid")
    }
}

impl OpenAiGatewayKeyDescriptor {
    /// Validates and wraps a gateway key expression.
    pub fn from_expr(key: Expr) -> Result<Self> {
        gateway_key_expr::decode(&key)?;
        Ok(Self { key })
    }

    /// Returns the wrapped gateway key expression.
    pub fn as_expr(&self) -> &Expr {
        &self.key
    }
}

impl Default for OpenAiGatewayKeyDescriptor {
    fn default() -> Self {
        Self::from_expr(OpenAiGatewayKey::from_secret("citizen", CapabilitySet::new()).to_expr())
            .expect("default OpenAI key descriptor should be valid")
    }
}

/// Returns the class symbol `openai/GatewayRequest`.
pub fn openai_gateway_request_class_symbol() -> Symbol {
    Symbol::qualified("openai", "GatewayRequest")
}

/// Returns the class symbol `openai/GatewayResponse`.
pub fn openai_gateway_response_class_symbol() -> Symbol {
    Symbol::qualified("openai", "GatewayResponse")
}

/// Returns the class symbol `openai/GatewayRun`.
pub fn openai_gateway_run_class_symbol() -> Symbol {
    Symbol::qualified("openai", "GatewayRun")
}

/// Returns the class symbol `openai/GatewayEvent`.
pub fn openai_gateway_event_class_symbol() -> Symbol {
    Symbol::qualified("openai", "GatewayEvent")
}

/// Returns the class symbol `openai/Plan`.
pub fn openai_plan_class_symbol() -> Symbol {
    Symbol::qualified("openai", "Plan")
}

/// Returns the class symbol `openai/GatewayKey`.
pub fn openai_gateway_key_class_symbol() -> Symbol {
    Symbol::qualified("openai", "GatewayKey")
}

pub(crate) mod gateway_request_expr {
    use sim_kernel::{Expr, Result};

    use super::expect_object;

    pub fn encode(expr: &Expr) -> Expr {
        expr.clone()
    }

    pub fn decode(expr: &Expr) -> Result<Expr> {
        expect_object(expr, "openai-gateway/request")?;
        Ok(expr.clone())
    }
}

pub(crate) mod gateway_response_expr {
    use sim_kernel::{Expr, Result};

    use super::expect_object;

    pub fn encode(expr: &Expr) -> Expr {
        expr.clone()
    }

    pub fn decode(expr: &Expr) -> Result<Expr> {
        expect_object(expr, "openai-gateway/response")?;
        Ok(expr.clone())
    }
}

pub(crate) mod gateway_run_expr {
    use sim_kernel::{Expr, Result};

    use super::expect_object;

    pub fn encode(expr: &Expr) -> Expr {
        expr.clone()
    }

    pub fn decode(expr: &Expr) -> Result<Expr> {
        expect_object(expr, "openai-gateway/run")?;
        Ok(expr.clone())
    }
}

pub(crate) mod gateway_event_expr {
    use sim_kernel::{Expr, Result};

    use super::expect_object;

    pub fn encode(expr: &Expr) -> Expr {
        expr.clone()
    }

    pub fn decode(expr: &Expr) -> Result<Expr> {
        expect_object(expr, "openai-gateway/event")?;
        Ok(expr.clone())
    }
}

pub(crate) mod plan_source {
    use sim_kernel::{Error, Expr, Result};

    use crate::parse_plan;

    pub fn encode(source: &str) -> Expr {
        Expr::String(source.to_owned())
    }

    pub fn decode(expr: &Expr) -> Result<String> {
        let Expr::String(source) = expr else {
            return Err(Error::Eval(
                "OpenAI plan descriptor source must be a string".to_owned(),
            ));
        };
        parse_plan(source)?;
        Ok(source.clone())
    }
}

pub(crate) mod gateway_key_expr {
    use sim_kernel::{Expr, Result};

    use super::expect_object;

    pub fn encode(expr: &Expr) -> Expr {
        expr.clone()
    }

    pub fn decode(expr: &Expr) -> Result<Expr> {
        expect_object(expr, "openai-gateway/key")?;
        Ok(expr.clone())
    }
}

fn expect_object(expr: &Expr, expected: &str) -> Result<()> {
    let Expr::Map(entries) = expr else {
        return Err(Error::Eval(format!("{expected} descriptor must be a map")));
    };
    let object = entries.iter().find_map(|(key, value)| {
        field_name(key)
            .as_deref()
            .filter(|name| *name == "object")
            .map(|_| value)
    });
    match object {
        Some(Expr::String(value)) if value == expected => Ok(()),
        _ => Err(Error::Eval(format!(
            "{expected} descriptor has wrong object field"
        ))),
    }
}

fn field_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Symbol(symbol) if symbol.namespace.is_none() => Some(symbol.name.to_string()),
        Expr::String(value) => Some(value.clone()),
        _ => None,
    }
}
