use sim_kernel::{CodecId, Lib, Linker, LoadCx, Result};

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
pub use sim_codec_chat::{OpenAiCodec, OpenAiCodecLib, openai_codec_symbol};
pub use streaming::{
    GatewayEventData, OpenAiSseSurface, StreamSink, encode_gateway_events_sse,
    gateway_event_data_from_packet, gateway_event_data_kind, gateway_event_data_packets,
};

const OPENAI_CODEC_RUNTIME_ID: CodecId = CodecId(0);

/// Registers the OpenAI codec with the linker, resolving its expression and
/// options shapes and installing it as a runtime codec value.
pub fn install_openai_codec(cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
    OpenAiCodecLib::new(OPENAI_CODEC_RUNTIME_ID).load(cx, linker)
}
