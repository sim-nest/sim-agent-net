use std::sync::Arc;

use sim_codec::{
    CodecDefaultDecode, CodecRuntime, Decoder, Encoder, Input, Output, ReadCx, codec_value,
};
use sim_kernel::{CodecId, DefaultFactory, Error, Factory, Linker, Result, Symbol, WriteCx};

/// Decoding of OpenAI request/response JSON into SIM transcript expressions.
pub mod decode;
/// Encoding of SIM transcript expressions into OpenAI request/response JSON.
pub mod encode;
/// Validated transcript and codec-option shapes for the OpenAI codec.
pub mod shapes;
/// Server-sent-event streaming of gateway events on OpenAI surfaces.
pub mod streaming;

pub use decode::{decode_openai_request, decode_openai_response};
pub use encode::{encode_openai_request, encode_openai_response, encode_openai_responses_response};
pub use shapes::{ChatTranscript, OpenAiCodecOptions, OpenAiRequestOptions};
pub use streaming::{
    GatewayEventData, OpenAiSseSurface, StreamSink, encode_gateway_events_sse,
    gateway_event_data_from_packet, gateway_event_data_kind, gateway_event_data_packets,
};

const OPENAI_CODEC_RUNTIME_ID: CodecId = CodecId(0);

/// Runtime codec for OpenAI-compatible chat-completion JSON fixtures.
pub struct OpenAiCodec;

impl Decoder for OpenAiCodec {
    fn decode(&self, cx: &mut ReadCx<'_>, input: Input) -> Result<sim_kernel::Expr> {
        decode::decode_openai_request_for_codec(cx.codec, input)
    }
}

impl Encoder for OpenAiCodec {
    fn encode(&self, cx: &mut WriteCx<'_>, expr: &sim_kernel::Expr) -> Result<Output> {
        encode::encode_openai_response_for_codec(cx.codec, expr).map(Output::Text)
    }
}

/// Returns the codec symbol `codec/openai`.
pub fn openai_codec_symbol() -> Symbol {
    Symbol::qualified("codec", "openai")
}

/// Registers the OpenAI codec with the linker, resolving its expression and
/// options shapes and installing it as a runtime codec value.
pub fn install_openai_codec(linker: &mut Linker<'_>) -> Result<()> {
    let factory = DefaultFactory;
    let expr_shape = linker
        .registry()
        .shape_by_symbol(&Symbol::qualified("codec", "ChatTranscript"))
        .or_else(|| {
            linker
                .registry()
                .shape_by_symbol(&Symbol::qualified("core", "Expr"))
        })
        .or_else(|| {
            linker
                .registry()
                .shape_by_symbol(&Symbol::qualified("core", "Any"))
        })
        .cloned()
        .unwrap_or(factory.nil()?);
    let options_shape = linker
        .registry()
        .shape_by_symbol(&Symbol::qualified("codec", "OpenAiCodecOptions"))
        .or_else(|| {
            linker
                .registry()
                .shape_by_symbol(&Symbol::qualified("core", "EncodeOptions"))
        })
        .or_else(|| {
            linker
                .registry()
                .shape_by_symbol(&Symbol::qualified("core", "Any"))
        })
        .cloned()
        .unwrap_or(factory.nil()?);

    let symbol = openai_codec_symbol();
    linker.codec_value(
        symbol.clone(),
        codec_value(CodecRuntime {
            id: OPENAI_CODEC_RUNTIME_ID,
            symbol,
            decoder: Some(Arc::new(OpenAiCodec)),
            located_decoder: None,
            tree_decoder: None,
            encoder: Some(Arc::new(OpenAiCodec)),
            located_encoder: None,
            tree_encoder: None,
            expr_shape,
            options_shape,
            default_decode: CodecDefaultDecode::Datum,
        }),
    )?;
    Ok(())
}

pub(crate) fn input_text(codec: CodecId, input: Input) -> Result<String> {
    match input {
        Input::Text(text) => Ok(text),
        Input::Bytes(bytes) => String::from_utf8(bytes).map_err(|err| codec_error(codec, err)),
    }
}

pub(crate) fn codec_error(codec: CodecId, message: impl ToString) -> Error {
    Error::CodecError {
        codec,
        message: message.to_string(),
    }
}
