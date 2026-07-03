use crate::client::{HttpRunnerRequest, post_json, post_json_stream};
use crate::redact::redact_text;
use crate::stream::HttpStreamDecoder;
use sim_codec_chat::{
    OllamaRequestOptions, decode_ollama_response, decode_ollama_stream, encode_ollama_request,
    model_error_expr,
};
use sim_kernel::{
    CapabilityName, Cx, Datum, DatumStore, Effect, Error, Expr, Ref, Result, Symbol, core_any_ref,
    effect, value_from_ref,
};
use sim_lib_agent_runner_core::{
    ModelCard, ModelEvent, ModelEventSink, ModelRequest, ModelResponse, ModelRunner,
};
use sim_lib_openai_server::{OpenAiRequestOptions, decode_openai_response, encode_openai_request};
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
    api_key_env: Option<String>,
    codec: Symbol,
    timeout: Duration,
    stream: bool,
    tools: bool,
    max_response_bytes: usize,
}

impl HttpRunner {
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
        Self {
            runner,
            model: model.into(),
            provider: Symbol::new("openai-compatible"),
            locality: Symbol::new("network"),
            runner_label: "runner/openai-compatible",
            request_path: "/chat/completions",
            endpoint: endpoint.into(),
            api_key_env: Some(api_key_env.into()),
            codec,
            timeout,
            stream,
            tools,
            max_response_bytes,
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
            api_key_env: None,
            codec,
            timeout,
            stream,
            tools,
            max_response_bytes,
        }
    }

    fn infer_inner(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        let include_raw = self.include_raw(cx, &request);
        let body = self.encode_request(request, self.stream)?;
        let api_key = self.api_key()?;
        let response = post_json(
            HttpRunnerRequest {
                runner_label: self.runner_label,
                endpoint: self.endpoint.as_str(),
                path: self.request_path,
                bearer_token: api_key.as_deref(),
                timeout: self.timeout,
                body,
                max_response_bytes: self.max_response_bytes,
            },
            api_key.as_deref(),
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
        let body = self.encode_request(request, true)?;
        let api_key = self.api_key()?;
        let mut decoder = self.stream_decoder(include_raw)?;
        sink.emit(decoder.start_event())?;
        let response = post_json_stream(
            HttpRunnerRequest {
                runner_label: self.runner_label,
                endpoint: self.endpoint.as_str(),
                path: self.request_path,
                bearer_token: api_key.as_deref(),
                timeout: self.timeout,
                body,
                max_response_bytes: self.max_response_bytes,
            },
            api_key.as_deref(),
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
        let ollama_codec = Symbol::qualified("codec", "ollama");
        let request_expr: Expr = request.into();
        if self.codec == openai_codec {
            encode_openai_request(
                &request_expr,
                &OpenAiRequestOptions::new(self.model.clone(), stream, self.tools),
            )
        } else if self.codec == ollama_codec {
            encode_ollama_request(
                &request_expr,
                &OllamaRequestOptions::new(self.model.clone(), stream, self.tools),
            )
        } else {
            Err(Error::Eval(format!(
                "{} unsupported codec {}",
                self.runner_label, self.codec
            )))
        }
    }

    fn api_key(&self) -> Result<Option<String>> {
        match &self.api_key_env {
            Some(api_key_env) => Ok(Some(std::env::var(api_key_env).map_err(|_| {
                Error::Eval(format!(
                    "{} missing env var {}",
                    self.runner_label, api_key_env
                ))
            })?)),
            None => Ok(None),
        }
    }

    fn decode_response(&self, body: &[u8], include_raw: bool) -> Result<ModelResponse> {
        let openai_codec = Symbol::qualified("codec", "openai");
        let ollama_codec = Symbol::qualified("codec", "ollama");
        let expr = if self.codec == openai_codec {
            decode_openai_response(self.runner.clone(), &self.model, body, include_raw)?
        } else if self.codec == ollama_codec {
            if self.stream {
                decode_ollama_stream(self.runner.clone(), &self.model, body, include_raw)?
            } else {
                decode_ollama_response(self.runner.clone(), &self.model, body, include_raw)?
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

    fn stream_decoder(&self, include_raw: bool) -> Result<HttpStreamDecoder> {
        let openai_codec = Symbol::qualified("codec", "openai");
        let ollama_codec = Symbol::qualified("codec", "ollama");
        if self.codec == openai_codec {
            Ok(HttpStreamDecoder::openai(
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

impl ModelRunner for HttpRunner {
    fn card(&self) -> ModelCard {
        ModelCard::new(
            self.runner.clone(),
            self.model.clone(),
            self.provider.clone(),
            self.locality.clone(),
        )
    }

    fn infer(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        match self.resolve_network_effect(cx, request, |runner, cx, request| {
            runner.infer_inner(cx, request)
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
            effect::effect_network_kind(),
            Ref::Symbol(self.runner.clone()),
            input,
            core_any_ref(),
            effect::effect_resume_op_key(),
            effect::effect_abort_op_key(),
        )
        .with_replay_key(Some(Ref::Symbol(Symbol::qualified(
            "agent",
            "http-runner-v1",
        ))))
    }
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
