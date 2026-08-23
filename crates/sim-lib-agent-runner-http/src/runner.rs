use crate::ProviderAuth;
use crate::client::{HttpRunnerRequest, post_json, post_json_stream};
use crate::config::{ProviderConfig, compatibility_secret};
use crate::model_params::attach_bridge_model_params_to_body;
use crate::redact::redact_text;
use crate::stream::HttpStreamDecoder;

// conformance: direct and split HTTP provider paths are identical and thread-safe.
use sim_codec_chat::{
    AnthropicRequestOptions, LemonadeRequestOptions, LmStudioRequestOptions, OllamaRequestOptions,
    OpenAiRequestOptions, decode_anthropic_response, decode_anthropic_stream,
    decode_lemonade_response, decode_lemonade_stream, decode_lm_studio_response,
    decode_lm_studio_stream, decode_ollama_response, decode_ollama_stream, decode_openai_response,
    decode_openai_responses_response, encode_anthropic_request, encode_lemonade_request,
    encode_lm_studio_request, encode_ollama_request, encode_openai_request,
    encode_openai_responses_request, model_error_expr,
};
use sim_kernel::{
    CapabilityName, Cx, Datum, DatumStore, Effect, Error, Expr, Ref, Result, Symbol, core_any_ref,
    effect, value_from_ref,
};
use sim_lib_agent_runner_core::{
    ModelCard, ModelEvent, ModelEventSink, ModelRequest, ModelResponse, ModelRunner,
    OUTPUT_GRAMMAR_DIALECT_EXTRA, OUTPUT_GRAMMAR_EXTRA, OUTPUT_GRAMMAR_REQUIRED_EXTRA,
    RETURN_CODEC_EXTRA, RETURN_SHAPE_EXTRA, grammar_dialect_symbol,
};
use sim_lib_provider::{
    ProviderCall, ProviderDispatch, ProviderOutcome, ProviderSeatExecution, Secret,
};
use sim_shape::GrammarDialect;
use std::time::Duration;

/// HTTP-backed [`ModelRunner`] for OpenAI-compatible and Ollama endpoints.
#[derive(Clone, Debug)]
pub struct HttpRunner {
    runner: Symbol,
    model: String,
    provider: Symbol,
    locality: Symbol,
    runner_label: &'static str,
    request_path: &'static str,
    endpoint: String,
    secret: Option<Secret>,
    auth: ProviderAuth,
    codec: Symbol,
    timeout: Duration,
    stream: bool,
    tools: bool,
    max_response_bytes: usize,
    grammar_dialects: Vec<GrammarDialect>,
}

/// Owned HTTP dispatch payload produced by [`HttpRunner::plan`].
#[derive(Clone, Debug)]
pub struct HttpProviderCall {
    request: HttpRunnerRequest,
    secret: Option<Secret>,
    include_raw: bool,
}

/// Owned HTTP outcome consumed by [`HttpRunner::land`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpProviderOutcome {
    response: crate::client::HttpRunnerResponse,
    include_raw: bool,
}

impl HttpRunner {
    /// Builds a runner from an open provider config without changing the HTTP
    /// transport path.
    pub fn new_provider(config: ProviderConfig) -> Self {
        let auth = config.profile.auth.clone();
        Self {
            runner: config.runner,
            model: config.model,
            provider: config.profile.provider,
            locality: config.locality,
            runner_label: "runner/provider",
            request_path: config.profile.chat_path,
            endpoint: config.endpoint,
            secret: config.secret,
            auth,
            codec: config.codec,
            timeout: config.timeout,
            stream: config.stream,
            tools: config.tools,
            max_response_bytes: config.max_output_bytes,
            grammar_dialects: config.grammar_dialects,
        }
    }

    /// Builds a runner targeting an OpenAI-compatible `/chat/completions`
    /// endpoint, reading its API key from the `api_key_env` environment
    /// variable.
    #[allow(clippy::too_many_arguments)]
    pub fn new_openai_compatible(
        runner: Symbol,
        model: impl Into<String>,
        endpoint: impl Into<String>,
        api_key_env: impl Into<String>,
        codec: Symbol,
        timeout: Duration,
        stream: bool,
        tools: bool,
        max_response_bytes: usize,
    ) -> Self {
        let api_key_env = api_key_env.into();
        let secret = compatibility_secret(&api_key_env).expect(
            "historical OpenAI-compatible constructor requires its credential at construction time",
        );
        Self {
            runner,
            model: model.into(),
            provider: Symbol::new("openai-compatible"),
            locality: Symbol::new("network"),
            runner_label: "runner/openai-compatible",
            request_path: "/chat/completions",
            endpoint: endpoint.into(),
            secret: Some(secret),
            auth: ProviderAuth::BearerEnv { env: api_key_env },
            codec,
            timeout,
            stream,
            tools,
            max_response_bytes,
            grammar_dialects: Vec::new(),
        }
    }

    /// Builds a runner targeting an Ollama endpoint at the given `locality`.
    #[allow(clippy::too_many_arguments)]
    pub fn new_ollama(
        runner: Symbol,
        model: impl Into<String>,
        locality: Symbol,
        endpoint: impl Into<String>,
        codec: Symbol,
        timeout: Duration,
        stream: bool,
        tools: bool,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            runner,
            model: model.into(),
            provider: Symbol::new("ollama"),
            locality,
            runner_label: "runner/ollama",
            request_path: "/api/chat",
            endpoint: endpoint.into(),
            secret: None,
            auth: ProviderAuth::None,
            codec,
            timeout,
            stream,
            tools,
            max_response_bytes,
            grammar_dialects: vec![GrammarDialect::Gbnf],
        }
    }

    fn infer_inner(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        let include_raw = self.include_raw(cx, &request);
        let api_key = self.secret.as_ref().map(Secret::expose);
        let headers = self.request_headers(api_key);
        let body = self.encode_request(request, self.stream)?;
        let response = post_json(
            HttpRunnerRequest {
                runner_label: self.runner_label,
                endpoint: self.endpoint.clone(),
                path: self.request_path,
                headers,
                timeout: self.timeout,
                body,
                max_response_bytes: self.max_response_bytes,
            },
            api_key,
        )?;
        self.decode_response(&response.body, include_raw)
    }

    fn infer_stream_inner(
        &self,
        cx: &mut Cx,
        request: ModelRequest,
        sink: &mut dyn ModelEventSink,
    ) -> Result<ModelResponse> {
        if !self.stream {
            let response = self.infer_inner(cx, request)?;
            sink.emit(ModelEvent::final_of(&response))?;
            return Ok(response);
        }
        let include_raw = self.include_raw(cx, &request);
        let api_key = self.secret.as_ref().map(Secret::expose);
        let headers = self.request_headers(api_key);
        let body = self.encode_request(request, true)?;
        let mut decoder = self.stream_decoder(include_raw)?;
        sink.emit(decoder.start_event())?;
        let response = post_json_stream(
            HttpRunnerRequest {
                runner_label: self.runner_label,
                endpoint: self.endpoint.clone(),
                path: self.request_path,
                headers,
                timeout: self.timeout,
                body,
                max_response_bytes: self.max_response_bytes,
            },
            api_key,
            &mut |chunk| decoder.feed(chunk, sink),
        )?;
        let model_response = if decoder.has_stream_output() {
            decoder.finish(sink)?
        } else {
            self.decode_response(&response.body, include_raw)?
        };
        sink.emit(ModelEvent::final_of(&model_response))?;
        Ok(model_response)
    }

    fn encode_request(&self, request: ModelRequest, stream: bool) -> Result<Vec<u8>> {
        let openai_codec = Symbol::qualified("codec", "openai");
        let openai_responses_codec = Symbol::qualified("codec", "openai-responses");
        let anthropic_codec = Symbol::qualified("codec", "anthropic");
        let ollama_codec = Symbol::qualified("codec", "ollama");
        let lm_studio_codec = Symbol::qualified("codec", "lm-studio");
        let lemonade_codec = Symbol::qualified("codec", "lemonade");
        let request = self.prepare_output_grammar(request);
        let request_extra = request.extra.clone();
        let request_expr: Expr = request.into();
        let body = if self.codec == openai_responses_codec {
            encode_openai_responses_request(
                &request_expr,
                &OpenAiRequestOptions::new(self.model.clone(), stream, self.tools),
            )
        } else if self.codec == openai_codec {
            encode_openai_request(
                &request_expr,
                &OpenAiRequestOptions::new(self.model.clone(), stream, self.tools),
            )
        } else if self.codec == anthropic_codec {
            encode_anthropic_request(
                &request_expr,
                &AnthropicRequestOptions::new(
                    self.model.clone(),
                    DEFAULT_ANTHROPIC_MAX_TOKENS,
                    stream,
                    self.tools,
                ),
            )
        } else if self.codec == ollama_codec {
            encode_ollama_request(
                &request_expr,
                &OllamaRequestOptions::new(self.model.clone(), stream, self.tools),
            )
        } else if self.codec == lm_studio_codec {
            encode_lm_studio_request(
                &request_expr,
                &LmStudioRequestOptions::new(self.model.clone(), stream, self.tools),
            )
        } else if self.codec == lemonade_codec {
            encode_lemonade_request(
                &request_expr,
                &LemonadeRequestOptions::new(self.model.clone(), stream, self.tools),
            )
        } else {
            Err(Error::Eval(format!(
                "{} unsupported codec {}",
                self.runner_label, self.codec
            )))
        }?;
        attach_bridge_model_params_to_body(&self.codec, &request_extra, body, self.runner_label)
    }

    fn request_headers(&self, secret: Option<&str>) -> Vec<(String, Secret)> {
        if self.provider == Symbol::new("anthropic")
            && matches!(self.auth, ProviderAuth::HeaderEnv { .. })
            && let Some(secret) = secret
        {
            return anthropic_headers(secret);
        }

        let mut headers = vec![content_type_header()];
        match (&self.auth, secret) {
            (
                ProviderAuth::BearerEnv { .. } | ProviderAuth::OptionalBearerEnv { .. },
                Some(secret),
            ) => {
                headers.push((
                    "Authorization".to_owned(),
                    Secret::new(format!("Bearer {secret}")).expect("valid bearer header"),
                ));
            }
            (ProviderAuth::HeaderEnv { header, .. }, Some(secret)) => {
                headers.push((
                    header.clone(),
                    Secret::new(secret).expect("validated provider secret"),
                ));
            }
            _ => {}
        }
        if self.provider == Symbol::new("anthropic") {
            headers.push((
                "anthropic-version".to_owned(),
                Secret::new(ANTHROPIC_VERSION).expect("valid fixed header"),
            ));
        }
        headers
    }

    fn decode_response(&self, body: &[u8], include_raw: bool) -> Result<ModelResponse> {
        let openai_codec = Symbol::qualified("codec", "openai");
        let openai_responses_codec = Symbol::qualified("codec", "openai-responses");
        let anthropic_codec = Symbol::qualified("codec", "anthropic");
        let ollama_codec = Symbol::qualified("codec", "ollama");
        let lm_studio_codec = Symbol::qualified("codec", "lm-studio");
        let lemonade_codec = Symbol::qualified("codec", "lemonade");
        let expr = if self.codec == openai_responses_codec {
            decode_openai_responses_response(self.runner.clone(), &self.model, body, include_raw)?
        } else if self.codec == openai_codec {
            decode_openai_response(self.runner.clone(), &self.model, body, include_raw)?
        } else if self.codec == anthropic_codec {
            if self.stream {
                decode_anthropic_stream(self.runner.clone(), &self.model, body, include_raw)?
            } else {
                decode_anthropic_response(self.runner.clone(), &self.model, body, include_raw)?
            }
        } else if self.codec == ollama_codec {
            if self.stream {
                decode_ollama_stream(self.runner.clone(), &self.model, body, include_raw)?
            } else {
                decode_ollama_response(self.runner.clone(), &self.model, body, include_raw)?
            }
        } else if self.codec == lm_studio_codec {
            if self.stream {
                decode_lm_studio_stream(self.runner.clone(), &self.model, body, include_raw)?
            } else {
                decode_lm_studio_response(self.runner.clone(), &self.model, body, include_raw)?
            }
        } else if self.codec == lemonade_codec {
            if self.stream {
                decode_lemonade_stream(self.runner.clone(), &self.model, body, include_raw)?
            } else {
                decode_lemonade_response(self.runner.clone(), &self.model, body, include_raw)?
            }
        } else {
            unreachable!("codec checked above")
        };
        ModelResponse::try_from(expr)
    }

    fn include_raw(&self, cx: &mut Cx, request: &ModelRequest) -> bool {
        cx.require(&CapabilityName::new("ai-runner-raw-log"))
            .is_ok()
            && !request_privacy_no_raw(request)
    }

    fn direct_capabilities(&self) -> Vec<CapabilityName> {
        let mut capabilities = vec![CapabilityName::new(AI_RUNNER_CAPABILITY)];
        if self.locality == Symbol::new("local") {
            capabilities.push(CapabilityName::new(AI_RUNNER_LOCAL_CAPABILITY));
        } else {
            capabilities.push(CapabilityName::new(AI_RUNNER_NETWORK_CAPABILITY));
        }
        if self.secret.is_some() {
            capabilities.push(CapabilityName::new(AI_RUNNER_SECRET_CAPABILITY));
        }
        capabilities
    }

    fn stream_decoder(&self, include_raw: bool) -> Result<HttpStreamDecoder> {
        let openai_codec = Symbol::qualified("codec", "openai");
        let openai_responses_codec = Symbol::qualified("codec", "openai-responses");
        let anthropic_codec = Symbol::qualified("codec", "anthropic");
        let ollama_codec = Symbol::qualified("codec", "ollama");
        let lm_studio_codec = Symbol::qualified("codec", "lm-studio");
        let lemonade_codec = Symbol::qualified("codec", "lemonade");
        if self.codec == openai_responses_codec {
            Err(Error::Eval(
                "streaming OpenAI Responses seats are not yet supported".to_owned(),
            ))
        } else if self.codec == openai_codec {
            Ok(HttpStreamDecoder::openai(
                self.runner.clone(),
                self.model.clone(),
                include_raw,
            ))
        } else if self.codec == anthropic_codec {
            Ok(HttpStreamDecoder::anthropic(
                self.runner.clone(),
                self.model.clone(),
                include_raw,
            ))
        } else if self.codec == ollama_codec {
            Ok(HttpStreamDecoder::ollama(
                self.runner.clone(),
                self.model.clone(),
                include_raw,
            ))
        } else if self.codec == lm_studio_codec || self.codec == lemonade_codec {
            Ok(HttpStreamDecoder::openai(
                self.runner.clone(),
                self.model.clone(),
                include_raw,
            ))
        } else {
            Err(Error::Eval(format!(
                "{} unsupported codec {}",
                self.runner_label, self.codec
            )))
        }
    }

    fn error_response(&self, message: impl Into<String>) -> Result<ModelResponse> {
        ModelResponse::try_from(model_error_expr(
            self.runner.clone(),
            self.model.clone(),
            message.into(),
        ))
    }

    fn prepare_output_grammar(&self, mut request: ModelRequest) -> ModelRequest {
        let Some(dialect) = self.preferred_grammar_dialect() else {
            strip_output_grammar(&mut request.extra);
            return request;
        };
        if extra_field(&request.extra, RETURN_SHAPE_EXTRA).is_none()
            && !explicit_output_grammar_matches(&request.extra, dialect)
        {
            strip_output_grammar(&mut request.extra);
            return request;
        }
        let return_codec = extra_symbol(&request.extra, RETURN_CODEC_EXTRA);
        if return_codec.as_ref() != Some(&Symbol::qualified("codec", "json")) {
            strip_output_grammar(&mut request.extra);
            return request;
        }
        if !explicit_output_grammar_matches(&request.extra, dialect) {
            remove_extra(&mut request.extra, OUTPUT_GRAMMAR_EXTRA);
        }
        normalize_return_shape_for_output_grammar(&mut request.extra);
        upsert_extra(
            &mut request.extra,
            OUTPUT_GRAMMAR_DIALECT_EXTRA,
            Expr::Symbol(grammar_dialect_symbol(dialect)),
        );
        request
    }

    fn preferred_grammar_dialect(&self) -> Option<GrammarDialect> {
        if self.grammar_dialects.contains(&GrammarDialect::JsonSchema) {
            Some(GrammarDialect::JsonSchema)
        } else if self.grammar_dialects.contains(&GrammarDialect::Gbnf) {
            Some(GrammarDialect::Gbnf)
        } else {
            None
        }
    }
}

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_ANTHROPIC_MAX_TOKENS: u64 = 1024;
const AI_RUNNER_CAPABILITY: &str = "ai-runner";
const AI_RUNNER_NETWORK_CAPABILITY: &str = "ai-runner-network";
const AI_RUNNER_LOCAL_CAPABILITY: &str = "ai-runner-local";
const AI_RUNNER_SECRET_CAPABILITY: &str = "ai-runner-secret";

fn anthropic_headers(secret: &str) -> Vec<(String, Secret)> {
    vec![
        (
            "x-api-key".to_owned(),
            Secret::new(secret).expect("validated provider secret"),
        ),
        (
            "anthropic-version".to_owned(),
            Secret::new(ANTHROPIC_VERSION).expect("valid fixed header"),
        ),
        content_type_header(),
    ]
}

fn content_type_header() -> (String, Secret) {
    (
        "content-type".to_owned(),
        Secret::new("application/json").expect("valid fixed header"),
    )
}

fn request_privacy_no_raw(request: &ModelRequest) -> bool {
    request
        .extra
        .iter()
        .find_map(|(key, value)| is_field(key, "privacy").then_some(value))
        .is_some_and(privacy_expr_no_raw)
}

fn privacy_expr_no_raw(expr: &Expr) -> bool {
    match expr {
        Expr::Symbol(symbol) => symbol.name.as_ref() == "no-raw",
        Expr::String(text) => text == "no-raw",
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) => {
            items.iter().any(privacy_expr_no_raw)
        }
        Expr::Map(entries) => entries.iter().any(|(key, value)| {
            is_field(key, "no-raw") && !matches!(value, Expr::Bool(false) | Expr::Nil)
        }),
        _ => false,
    }
}

fn is_field(expr: &Expr, name: &str) -> bool {
    matches!(
        expr,
        Expr::Symbol(symbol) if symbol.namespace.is_none() && symbol.name.as_ref() == name
    )
}

fn extra_field<'a>(entries: &'a [(Expr, Expr)], name: &str) -> Option<&'a Expr> {
    entries.iter().find_map(|(key, value)| {
        if is_field(key, name) {
            Some(value)
        } else {
            None
        }
    })
}

fn extra_field_mut<'a>(entries: &'a mut [(Expr, Expr)], name: &str) -> Option<&'a mut Expr> {
    entries.iter_mut().find_map(|(key, value)| {
        if is_field(key, name) {
            Some(value)
        } else {
            None
        }
    })
}

fn extra_symbol(entries: &[(Expr, Expr)], name: &str) -> Option<Symbol> {
    match extra_field(entries, name) {
        Some(Expr::Symbol(symbol)) => Some(symbol.clone()),
        _ => None,
    }
}

fn upsert_extra(entries: &mut Vec<(Expr, Expr)>, name: &str, value: Expr) {
    if let Some((_, existing)) = entries.iter_mut().find(|(key, _)| is_field(key, name)) {
        *existing = value;
        return;
    }
    entries.push((Expr::Symbol(Symbol::new(name)), value));
}

fn strip_output_grammar(entries: &mut Vec<(Expr, Expr)>) {
    entries.retain(|(key, _)| {
        !is_field(key, OUTPUT_GRAMMAR_EXTRA)
            && !is_field(key, OUTPUT_GRAMMAR_DIALECT_EXTRA)
            && !is_field(key, OUTPUT_GRAMMAR_REQUIRED_EXTRA)
            && !is_field(key, RETURN_SHAPE_EXTRA)
    });
}

fn remove_extra(entries: &mut Vec<(Expr, Expr)>, name: &str) {
    entries.retain(|(key, _)| !is_field(key, name));
}

fn normalize_return_shape_for_output_grammar(entries: &mut [(Expr, Expr)]) {
    let Some(shape_expr) = extra_field_mut(entries, RETURN_SHAPE_EXTRA) else {
        return;
    };
    let Expr::Symbol(symbol) = shape_expr else {
        return;
    };
    if symbol.namespace.as_deref() != Some("core") {
        return;
    }
    if matches!(
        symbol.name.as_ref(),
        "Any" | "Bool" | "List" | "Map" | "Nil" | "Number" | "String" | "Symbol"
    ) {
        *shape_expr = Expr::Symbol(Symbol::new(symbol.name.to_string()));
    }
}

fn explicit_output_grammar_matches(entries: &[(Expr, Expr)], dialect: GrammarDialect) -> bool {
    matches!(
        extra_field(entries, OUTPUT_GRAMMAR_EXTRA),
        Some(Expr::String(_))
    ) && extra_field(entries, OUTPUT_GRAMMAR_DIALECT_EXTRA)
        .and_then(|expr| match expr {
            Expr::Symbol(symbol) => grammar_dialect_from_symbol_local(symbol),
            _ => None,
        })
        .unwrap_or(GrammarDialect::JsonSchema)
        == dialect
}

fn grammar_dialect_from_symbol_local(symbol: &Symbol) -> Option<GrammarDialect> {
    match symbol.name.as_ref() {
        "json-schema" if symbol.namespace.is_none() => Some(GrammarDialect::JsonSchema),
        "gbnf" if symbol.namespace.is_none() => Some(GrammarDialect::Gbnf),
        "sexpr" if symbol.namespace.is_none() => Some(GrammarDialect::SExpr),
        _ => None,
    }
}

impl ModelRunner for HttpRunner {
    fn card(&self) -> ModelCard {
        let mut card = ModelCard::new(
            self.runner.clone(),
            self.model.clone(),
            self.provider.clone(),
            self.locality.clone(),
        );
        if !self.grammar_dialects.is_empty() {
            card.extra.push((
                Expr::Symbol(Symbol::new("output-grammar-dialects")),
                Expr::Vector(
                    self.grammar_dialects
                        .iter()
                        .copied()
                        .map(grammar_dialect_symbol)
                        .map(Expr::Symbol)
                        .collect(),
                ),
            ));
        }
        card
    }

    fn infer(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        match self.resolve_network_effect(cx, request, |runner, cx, request| {
            if runner.stream {
                runner.infer_inner(cx, request)
            } else {
                runner.execute(cx, request)
            }
        }) {
            Ok(response) => Ok(response),
            Err(error) => self.error_response(redact_text(&error.to_string(), &[])),
        }
    }

    fn infer_stream(
        &self,
        cx: &mut Cx,
        request: ModelRequest,
        sink: &mut dyn ModelEventSink,
    ) -> Result<ModelResponse> {
        match self.resolve_network_effect(cx, request, {
            let sink = &mut *sink;
            |runner, cx, request| runner.infer_stream_inner(cx, request, sink)
        }) {
            Ok(response) => Ok(response),
            Err(error) => {
                let message = redact_text(&error.to_string(), &[]);
                sink.emit(ModelEvent::error_text(
                    self.runner.clone(),
                    self.model.clone(),
                    Expr::String("http-stream-error".to_owned()),
                    message.clone(),
                ))?;
                let response = self.error_response(message)?;
                sink.emit(ModelEvent::final_of(&response))?;
                Ok(response)
            }
        }
    }
}

impl ProviderDispatch for HttpRunner {
    type Call = HttpProviderCall;
    type Outcome = HttpProviderOutcome;

    fn dispatch(call: ProviderCall<Self::Call>) -> Result<ProviderOutcome<Self::Outcome>> {
        let HttpProviderCall {
            request,
            secret,
            include_raw,
        } = call.payload;
        let response = post_json(request, secret.as_ref().map(Secret::expose))?;
        Ok(ProviderOutcome::new(HttpProviderOutcome {
            response,
            include_raw,
        }))
    }
}

impl ProviderSeatExecution for HttpRunner {
    fn plan(&self, cx: &mut Cx, request: ModelRequest) -> Result<ProviderCall<Self::Call>> {
        if self.stream {
            return Err(Error::Eval(format!(
                "{} split-mode streaming is unsupported",
                self.runner_label
            )));
        }
        let include_raw = self.include_raw(cx, &request);
        let secret = self.secret.clone();
        let headers = self.request_headers(secret.as_ref().map(Secret::expose));
        let body = self.encode_request(request, false)?;
        Ok(ProviderCall::new(HttpProviderCall {
            request: HttpRunnerRequest {
                runner_label: self.runner_label,
                endpoint: self.endpoint.clone(),
                path: self.request_path,
                headers,
                timeout: self.timeout,
                body,
                max_response_bytes: self.max_response_bytes,
            },
            secret,
            include_raw,
        }))
    }

    fn land(&self, _cx: &mut Cx, outcome: ProviderOutcome<Self::Outcome>) -> Result<ModelResponse> {
        self.decode_response(&outcome.payload.response.body, outcome.payload.include_raw)
    }
}

impl HttpRunner {
    fn resolve_network_effect<F>(
        &self,
        cx: &mut Cx,
        request: ModelRequest,
        perform: F,
    ) -> Result<ModelResponse>
    where
        F: FnOnce(&Self, &mut Cx, ModelRequest) -> Result<ModelResponse>,
    {
        let effect = self.network_effect(cx, &request)?;
        let result = effect::resolve_effect(cx, effect, |cx, _effect| {
            let response = perform(self, cx, request)?;
            response_ref(cx, response)
        })?;
        response_from_ref(cx, &result)
    }

    fn network_effect(&self, cx: &mut Cx, request: &ModelRequest) -> Result<Effect> {
        let input = Datum::Node {
            tag: Symbol::qualified("agent", "HttpRunnerInput"),
            fields: vec![
                (Symbol::new("runner"), Datum::Symbol(self.runner.clone())),
                (Symbol::new("model"), Datum::String(self.model.clone())),
                (
                    Symbol::new("provider"),
                    Datum::Symbol(self.provider.clone()),
                ),
                (
                    Symbol::new("endpoint"),
                    Datum::String(self.endpoint.clone()),
                ),
                (
                    Symbol::new("request"),
                    Datum::try_from(Expr::from(request.clone()))?,
                ),
            ],
        };
        let input = Ref::Content(cx.datum_store_mut().intern(input)?);
        Effect::new(
            cx.fresh_handle(),
            network_effect_kind(),
            Ref::Symbol(self.runner.clone()),
            input,
            core_any_ref(),
            effect::effect_resume_op_key(),
            effect::effect_abort_op_key(),
        )
        .with_requirements(self.direct_capabilities())
        .with_replay_key(Some(Ref::Symbol(Symbol::qualified(
            "agent",
            "http-runner-v1",
        ))))
    }
}

fn network_effect_kind() -> Symbol {
    Symbol::qualified("effect", "network")
}

fn response_ref(cx: &mut Cx, response: ModelResponse) -> Result<Ref> {
    Ok(Ref::Content(
        cx.datum_store_mut()
            .intern(Datum::try_from(Expr::from(response))?)?,
    ))
}

fn response_from_ref(cx: &mut Cx, reference: &Ref) -> Result<ModelResponse> {
    ModelResponse::try_from(value_from_ref(cx, reference)?.object().as_expr(cx)?)
}

#[cfg(test)]
mod tests;
