use std::sync::{Arc, Mutex};

use serde_json::{Map, Number, Value as JsonValue};
use sim_citizen_derive::non_citizen;
use sim_kernel::{
    Consistency, Cx, Error, EvalFabric, EvalMode, EvalReply, EvalRequest, Expr, Object,
    ObjectCompat, Result, Symbol, Value, eval_remote_capability,
};

use crate::{
    clock::{DeterministicWallClock, SystemWallClock, WallClock, WallTimestamp},
    objects::{GatewayRequest, GatewayResponseValue},
    routes::responses::{RESPONSES_PATH, ResponseExecution, ResponseIdGenerators},
    server::GatewayRouteState,
};

/// Location-transparent eval fabric exposing the OpenAI gateway as an `EvalFabric`.
///
/// Decodes an [`EvalRequest`] into a `/v1/responses` gateway request, runs it
/// against the shared route state (store, cache, runners, federation), and
/// returns the gateway response wrapped as a runtime value. Server code targets
/// this surface rather than a transport-specific API.
#[non_citizen(
    reason = "live OpenAI gateway fabric handle; reconstruct route data via openai/GatewayRequest and openai/Plan descriptors",
    kind = "handle",
    descriptor = "openai/GatewayRequest"
)]
pub struct OpenAiGatewayFabric {
    state: GatewayRouteState,
    runtime: Mutex<GatewayFabricRuntime>,
    codecs: Vec<Symbol>,
}

struct GatewayFabricRuntime {
    ids: ResponseIdGenerators,
    clock: GatewayFabricClock,
    last_execution: Option<ResponseExecution>,
}

enum GatewayFabricClock {
    System(SystemWallClock),
    Deterministic(DeterministicWallClock),
}

impl OpenAiGatewayFabric {
    /// Returns a fabric over fresh in-memory state and the system clock.
    pub fn memory() -> Self {
        Self::with_state_system(GatewayRouteState::memory(), 1)
    }

    /// Returns a fabric with deterministic ids and a fixed-step simulated clock.
    ///
    /// Suitable for tests and replay: ids count from `id_start` and the clock
    /// starts at `clock_start_ms`, advancing `clock_step_ms` per read.
    pub fn deterministic(id_start: u64, clock_start_ms: u64, clock_step_ms: u64) -> Self {
        Self::with_state_clock(
            GatewayRouteState::memory(),
            ResponseIdGenerators::deterministic(id_start),
            GatewayFabricClock::Deterministic(DeterministicWallClock::new(
                clock_start_ms,
                clock_step_ms,
            )),
        )
    }

    /// Returns a fabric over the supplied `state` using the system clock.
    ///
    /// Response ids are seeded deterministically from `id_seed`.
    pub fn with_state_system(state: GatewayRouteState, id_seed: u64) -> Self {
        Self::with_state_clock(
            state,
            ResponseIdGenerators::deterministic(id_seed),
            GatewayFabricClock::System(SystemWallClock),
        )
    }

    fn with_state_clock(
        state: GatewayRouteState,
        ids: ResponseIdGenerators,
        clock: GatewayFabricClock,
    ) -> Self {
        Self {
            state,
            runtime: Mutex::new(GatewayFabricRuntime {
                ids,
                clock,
                last_execution: None,
            }),
            codecs: vec![
                Symbol::qualified("codec", "binary"),
                Symbol::qualified("codec", "openai"),
            ],
        }
    }

    /// Returns the local-first eval request that carries `request`'s body for realization.
    pub fn eval_request_for_gateway_request(request: &GatewayRequest) -> EvalRequest {
        EvalRequest {
            expr: Expr::Bytes(request.body().to_vec()),
            result_shape: None,
            required_capabilities: Vec::new(),
            deadline: None,
            consistency: Consistency::LocalFirst,
            mode: EvalMode::Eval,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            trace: false,
        }
    }

    /// Returns the most recent response execution, or `None` before any run.
    pub fn last_execution(&self) -> Result<Option<ResponseExecution>> {
        self.runtime
            .lock()
            .map_err(|_| Error::PoisonedLock("openai gateway fabric runtime"))
            .map(|runtime| runtime.last_execution.clone())
    }
}

impl Object for OpenAiGatewayFabric {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<openai-gateway-fabric>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for OpenAiGatewayFabric {
    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("openai-gateway"))?,
            ),
            (
                Symbol::new("address"),
                cx.factory().symbol(Symbol::new("local"))?,
            ),
            (
                Symbol::new("codecs"),
                cx.factory().list(
                    self.codecs
                        .iter()
                        .cloned()
                        .map(|codec| cx.factory().symbol(codec))
                        .collect::<Result<Vec<_>>>()?,
                )?,
            ),
        ])
    }

    fn as_eval_fabric(&self) -> Option<&dyn EvalFabric> {
        Some(self)
    }
}

impl EvalFabric for OpenAiGatewayFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        if matches!(request.consistency, Consistency::RemoteOnly) {
            return Err(Error::CapabilityDenied {
                capability: eval_remote_capability(),
            });
        }
        if !matches!(request.mode, EvalMode::Eval) {
            return Err(Error::Eval(
                "openai gateway fabric only supports eval mode".to_owned(),
            ));
        }
        for capability in &request.required_capabilities {
            cx.require(capability)?;
        }

        let gateway_request = gateway_request_from_expr(&request.expr, request.stream)?;
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| Error::PoisonedLock("openai gateway fabric runtime"))?;
        let mut store = self
            .state
            .store()
            .lock()
            .map_err(|_| Error::PoisonedLock("openai gateway fabric store"))?;
        let mut cache = self
            .state
            .cache()
            .lock()
            .map_err(|_| Error::PoisonedLock("openai gateway fabric cache"))?;
        let runtime = &mut *runtime;
        let execution =
            crate::routes::responses::execute_response_request_with_cache_runners_and_federation(
                cx,
                &mut *store,
                &mut cache,
                &mut runtime.ids,
                &mut runtime.clock,
                &gateway_request,
                crate::routes::responses::ResponseRuntimeTargets::with_federation(
                    self.state.runners(),
                    self.state.federation(),
                ),
            );
        let response = execution.response().clone();
        runtime.last_execution = Some(execution);
        Ok(EvalReply {
            value: cx
                .factory()
                .opaque(Arc::new(GatewayResponseValue::new(response)))?,
            diagnostics: cx.take_diagnostics(),
            trace: request
                .trace
                .then(|| cx.factory().symbol(Symbol::new("openai-gateway")))
                .transpose()?,
        })
    }
}

#[cfg(feature = "http")]
impl sim_lib_server::EvalSite for OpenAiGatewayFabric {
    fn site_kind(&self) -> &'static str {
        "openai-gateway"
    }

    fn address(&self) -> &sim_lib_server::ServerAddress {
        static ADDRESS: sim_lib_server::ServerAddress = sim_lib_server::ServerAddress::Local;
        &ADDRESS
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(
        &self,
        cx: &mut Cx,
        frame: sim_lib_server::ServerFrame,
    ) -> Result<sim_lib_server::ServerFrame> {
        let consistency = frame.envelope.consistency;
        let reply_codec = frame
            .envelope
            .reply_codec_hint
            .clone()
            .filter(|hint| self.codecs.iter().any(|codec| codec == hint))
            .unwrap_or_else(|| self.codecs[0].clone());
        let request = sim_lib_server::eval_request_from_frame(cx, &frame)?;
        let reply = self.realize(cx, request)?;
        sim_lib_server::server_frame_from_reply(cx, &reply_codec, reply, consistency)
    }

    fn as_eval_fabric(&self) -> Option<&dyn EvalFabric> {
        Some(self)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl WallClock for GatewayFabricClock {
    fn now(&self) -> Result<WallTimestamp> {
        match self {
            Self::System(clock) => clock.now(),
            Self::Deterministic(clock) => clock.now(),
        }
    }
}

fn gateway_request_from_expr(expr: &Expr, stream: bool) -> Result<GatewayRequest> {
    let mut body = match expr {
        Expr::Bytes(bytes) => bytes.clone(),
        Expr::String(text) => text.as_bytes().to_vec(),
        Expr::Map(_) => {
            let mut json = json_from_expr(expr)?;
            if stream && let Some(object) = json.as_object_mut() {
                object.insert("stream".to_owned(), JsonValue::Bool(true));
            }
            crate::objects::canonical_json_bytes(json)
        }
        _ => {
            return Err(Error::TypeMismatch {
                expected: "OpenAI gateway request bytes, JSON string, or map",
                found: "non-request",
            });
        }
    };
    if stream && !matches!(expr, Expr::Map(_)) {
        body = ensure_streaming_body(body)?;
    }
    Ok(GatewayRequest::new(
        "POST",
        RESPONSES_PATH,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body,
    ))
}

fn ensure_streaming_body(body: Vec<u8>) -> Result<Vec<u8>> {
    let mut json = serde_json::from_slice::<JsonValue>(&body)
        .map_err(|err| Error::Eval(format!("invalid OpenAI gateway request JSON: {err}")))?;
    let Some(object) = json.as_object_mut() else {
        return Err(Error::Eval(
            "OpenAI gateway request JSON must be an object".to_owned(),
        ));
    };
    object.insert("stream".to_owned(), JsonValue::Bool(true));
    Ok(crate::objects::canonical_json_bytes(json))
}

fn json_from_expr(expr: &Expr) -> Result<JsonValue> {
    Ok(match expr {
        Expr::Nil => JsonValue::Null,
        Expr::Bool(flag) => JsonValue::Bool(*flag),
        Expr::Number(number) => number
            .canonical
            .parse::<i64>()
            .ok()
            .map(Number::from)
            .map(JsonValue::Number)
            .unwrap_or_else(|| JsonValue::String(number.canonical.clone())),
        Expr::String(text) => JsonValue::String(text.clone()),
        Expr::Symbol(symbol) | Expr::Local(symbol) => JsonValue::String(symbol.to_string()),
        Expr::List(items) | Expr::Vector(items) => JsonValue::Array(
            items
                .iter()
                .map(json_from_expr)
                .collect::<Result<Vec<_>>>()?,
        ),
        Expr::Map(entries) => {
            let mut object = Map::new();
            for (key, value) in entries {
                object.insert(json_key(key)?, json_from_expr(value)?);
            }
            JsonValue::Object(object)
        }
        _ => {
            return Err(Error::TypeMismatch {
                expected: "JSON-compatible request expression",
                found: "non-json",
            });
        }
    })
}

fn json_key(expr: &Expr) -> Result<String> {
    match expr {
        Expr::String(text) => Ok(text.clone()),
        Expr::Symbol(symbol) | Expr::Local(symbol) if symbol.namespace.is_none() => {
            Ok(symbol.name.as_ref().to_owned())
        }
        Expr::Symbol(symbol) | Expr::Local(symbol) => Ok(symbol.to_string()),
        _ => Err(Error::TypeMismatch {
            expected: "JSON object key",
            found: "non-key",
        }),
    }
}
