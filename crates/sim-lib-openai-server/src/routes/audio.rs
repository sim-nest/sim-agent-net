use serde_json::{Map, Value, json};
use sim_kernel::Expr;

use crate::{
    clock::{GatewayClock, SystemGatewayClock},
    objects::{GatewayRequest, GatewayResponse},
    routes::{
        request_json::{optional_string, record_execution, request_object, required_string},
        run_record::{
            RouteEventInput, RouteRunExecution, RouteRunIdGenerators, RouteRunRecord, field,
            record_route_execution,
        },
    },
    server::GatewayRouteState,
    storage::GatewayStore,
};

use super::errors::OpenAiRouteError;

/// Route path for audio transcription (`POST /v1/audio/transcriptions`).
pub const AUDIO_TRANSCRIPTIONS_PATH: &str = "/v1/audio/transcriptions";
/// Route path for text-to-speech synthesis (`POST /v1/audio/speech`).
pub const AUDIO_SPEECH_PATH: &str = "/v1/audio/speech";

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

/// Handles `POST /v1/audio/transcriptions`, returning a deterministic fixture
/// transcript for the supplied JSON audio input.
pub fn handle_audio_transcriptions(
    request: &GatewayRequest,
    state: &GatewayRouteState,
) -> GatewayResponse {
    let mut clock = SystemGatewayClock;
    let seed = clock.now_ms().unwrap_or(1);
    let mut ids = RouteRunIdGenerators::deterministic(seed);
    match state.store().lock() {
        Ok(mut store) => {
            execute_audio_transcription_request(&mut *store, &mut ids, &mut clock, request)
                .response()
                .clone()
        }
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

/// Handles `POST /v1/audio/speech`, returning deterministic fixture audio bytes
/// for the requested JSON text input.
pub fn handle_audio_speech(request: &GatewayRequest, state: &GatewayRouteState) -> GatewayResponse {
    let mut clock = SystemGatewayClock;
    let seed = clock.now_ms().unwrap_or(1);
    let mut ids = RouteRunIdGenerators::deterministic(seed);
    match state.store().lock() {
        Ok(mut store) => execute_audio_speech_request(&mut *store, &mut ids, &mut clock, request)
            .response()
            .clone(),
        Err(err) => OpenAiRouteError::internal_message(format!("gateway store lock failed: {err}"))
            .into_response(),
    }
}

pub(crate) fn execute_audio_transcription_request<S, C>(
    store: &mut S,
    ids: &mut RouteRunIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
) -> RouteRunExecution
where
    S: GatewayStore,
    C: GatewayClock,
{
    match try_execute_audio_transcription_request(store, ids, clock, request) {
        Ok(execution) => execution,
        Err(error) => RouteRunExecution::error(error),
    }
}

pub(crate) fn execute_audio_speech_request<S, C>(
    store: &mut S,
    ids: &mut RouteRunIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
) -> RouteRunExecution
where
    S: GatewayStore,
    C: GatewayClock,
{
    match try_execute_audio_speech_request(store, ids, clock, request) {
        Ok(execution) => execution,
        Err(error) => RouteRunExecution::error(error),
    }
}

fn try_execute_audio_transcription_request<S, C>(
    store: &mut S,
    ids: &mut RouteRunIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
) -> RouteResult<RouteRunExecution>
where
    S: GatewayStore,
    C: GatewayClock,
{
    let object = request_object(request.body())?;
    let model = optional_string(&object, "model", "sim/audio/transcribe-fixture");
    let input = audio_input(&object)?;
    let text = deterministic_transcript(input);
    let response = GatewayResponse::json(
        200,
        json!({
            "object": "audio.transcription",
            "model": model,
            "text": text,
            "duration_ms": fixture_duration_ms(input),
        })
        .to_string()
        .into_bytes(),
    );
    record_route_execution(
        store,
        ids,
        clock,
        request,
        RouteRunRecord::new(
            response,
            AUDIO_TRANSCRIPTIONS_PATH,
            vec![RouteEventInput::new(
                "transcription",
                audio_event_payload(model, input, "transcription"),
            )],
            record_execution(&object),
        ),
    )
}

fn try_execute_audio_speech_request<S, C>(
    store: &mut S,
    ids: &mut RouteRunIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
) -> RouteResult<RouteRunExecution>
where
    S: GatewayStore,
    C: GatewayClock,
{
    let object = request_object(request.body())?;
    let model = optional_string(&object, "model", "sim/audio/speech-fixture");
    let voice = optional_string(&object, "voice", "alloy");
    let format = optional_string(&object, "response_format", "wav");
    let input = required_string(&object, "input")?;
    let bytes = deterministic_speech_bytes(model, voice, format, input);
    let response = GatewayResponse::new(
        200,
        vec![
            (
                "Content-Type".to_owned(),
                speech_content_type(format).to_owned(),
            ),
            ("X-SIM-Audio-Object".to_owned(), "audio.speech".to_owned()),
        ],
        bytes,
    );
    record_route_execution(
        store,
        ids,
        clock,
        request,
        RouteRunRecord::new(
            response,
            AUDIO_SPEECH_PATH,
            vec![RouteEventInput::new(
                "speech",
                speech_event_payload(model, voice, format, input),
            )],
            record_execution(&object),
        ),
    )
}

fn audio_input(object: &Map<String, Value>) -> RouteResult<&str> {
    match (
        object.get("input").and_then(Value::as_str),
        object.get("file").and_then(Value::as_str),
    ) {
        (Some(input), _) | (None, Some(input)) => Ok(input),
        _ => Err(OpenAiRouteError::missing_required("input")),
    }
}

fn deterministic_transcript(input: &str) -> String {
    let compact = input.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("transcribed: {compact}")
}

fn fixture_duration_ms(input: &str) -> u64 {
    input.len() as u64 * 20
}

fn deterministic_speech_bytes(model: &str, voice: &str, format: &str, input: &str) -> Vec<u8> {
    format!("SIM-SPEECH\nmodel={model}\nvoice={voice}\nformat={format}\ninput={input}\n")
        .into_bytes()
}

fn speech_content_type(format: &str) -> &'static str {
    match format {
        "mp3" => "audio/mpeg",
        "opus" => "audio/ogg",
        _ => "audio/wav",
    }
}

fn audio_event_payload(model: &str, input: &str, operation: &str) -> Expr {
    Expr::Map(vec![
        field("operation", Expr::String(operation.to_owned())),
        field("model", Expr::String(model.to_owned())),
        field("input-bytes", Expr::String(input.len().to_string())),
    ])
}

fn speech_event_payload(model: &str, voice: &str, format: &str, input: &str) -> Expr {
    Expr::Map(vec![
        field("operation", Expr::String("speech".to_owned())),
        field("model", Expr::String(model.to_owned())),
        field("voice", Expr::String(voice.to_owned())),
        field("format", Expr::String(format.to_owned())),
        field("input-bytes", Expr::String(input.len().to_string())),
    ])
}
