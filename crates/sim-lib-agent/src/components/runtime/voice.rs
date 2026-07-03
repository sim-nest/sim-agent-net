use super::super::model::{AgentComponent, VoiceBackend};
use super::process::{capture_child_output, io_error_to_host, shell_child};
use super::stream_transform::transform_stream_expr;
use crate::{VOICE_CAPABILITY, memory::flatten_expr_text, util::expr_to_value};
use sim_kernel::CapabilityName;
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_server::{FrameKind, ServerFrame, eval_request_from_frame};
use sim_lib_stream_core::{PcmPacket, StreamDiagnostic, StreamPacket};
use std::time::Duration;

pub(in crate::components) fn answer_voice(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &VoiceBackend,
    frame: ServerFrame,
) -> Result<ServerFrame> {
    if frame.kind != FrameKind::Request {
        return answer_voice_stream(cx, component, backend, frame);
    }
    let consistency = frame.envelope.consistency;
    let request = eval_request_from_frame(cx, &frame)?;
    let response = voice_result_expr(cx, backend, request.expr)?;
    let value = expr_to_value(cx, &response)?;
    crate::reply::reply_frame(cx, &frame, value, consistency)
}

fn answer_voice_stream(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &VoiceBackend,
    frame: ServerFrame,
) -> Result<ServerFrame> {
    transform_stream_expr(cx, frame, |cx, expr| {
        Ok(voice_stream_packet(cx, backend, expr).to_expr())
    })
    .map_err(|err| {
        Error::Eval(format!(
            "{} only answers request or stream frames: {err}",
            component.symbol
        ))
    })
}

fn voice_result_expr(cx: &mut Cx, backend: &VoiceBackend, expr: Expr) -> Result<Expr> {
    match backend {
        VoiceBackend::Tts { voice, command } => voice_tts_expr(cx, voice, command.as_deref(), expr),
        VoiceBackend::Stt { locale, command } => {
            voice_stt_expr(cx, locale, command.as_deref(), expr)
        }
    }
}

fn voice_tts_expr(cx: &mut Cx, voice: &str, command: Option<&str>, expr: Expr) -> Result<Expr> {
    let text = flatten_expr_text(&expr);
    let (audio, synthetic) = match command {
        Some(command) => {
            cx.require(&CapabilityName::new(VOICE_CAPABILITY))?;
            (
                run_voice_command(command, text.into_bytes(), "voice/tts")?,
                false,
            )
        }
        None => (text.into_bytes(), true),
    };
    Ok(Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("voice")),
            Expr::String(voice.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("synthetic")),
            Expr::Bool(synthetic),
        ),
        (Expr::Symbol(Symbol::new("audio")), Expr::Bytes(audio)),
    ]))
}

fn voice_stt_expr(cx: &mut Cx, locale: &str, command: Option<&str>, expr: Expr) -> Result<Expr> {
    let input = match expr {
        Expr::Bytes(bytes) => bytes,
        Expr::String(text) => text.into_bytes(),
        _ => return Err(Error::Eval("voice/stt expects bytes or string".to_owned())),
    };
    let (text, synthetic) = match command {
        Some(command) => {
            cx.require(&CapabilityName::new(VOICE_CAPABILITY))?;
            let stdout = run_voice_command(command, input, "voice/stt")?;
            (
                String::from_utf8(stdout).map_err(|_| {
                    Error::Eval("voice/stt command returned non-utf-8 bytes".to_owned())
                })?,
                false,
            )
        }
        None => (
            String::from_utf8(input)
                .map_err(|_| Error::Eval("voice/stt expects utf-8 bytes".to_owned()))?,
            true,
        ),
    };
    Ok(Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("locale")),
            Expr::String(locale.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("synthetic")),
            Expr::Bool(synthetic),
        ),
        (Expr::Symbol(Symbol::new("text")), Expr::String(text)),
    ]))
}

fn run_voice_command(command: &str, input: Vec<u8>, label: &str) -> Result<Vec<u8>> {
    let child = shell_child(command).spawn().map_err(io_error_to_host)?;
    capture_child_output(child, input, label, Duration::from_secs(2), 64 * 1024)
}

fn voice_stream_packet(cx: &mut Cx, backend: &VoiceBackend, expr: Expr) -> StreamPacket {
    let packet = StreamPacket::try_from(expr.clone()).ok();
    match backend {
        VoiceBackend::Tts { command, .. } => {
            tts_stream_packet(cx, command.as_deref(), packet, expr)
        }
        VoiceBackend::Stt { locale, command } => {
            stt_stream_packet(cx, locale, command.as_deref(), packet, expr)
        }
    }
}

fn tts_stream_packet(
    cx: &mut Cx,
    command: Option<&str>,
    packet: Option<StreamPacket>,
    expr: Expr,
) -> StreamPacket {
    if let Some(StreamPacket::Pcm(packet)) = packet {
        return StreamPacket::Pcm(packet);
    }
    let text = packet_text(packet, expr);
    let bytes = match command {
        Some(command) => match cx
            .require(&CapabilityName::new(VOICE_CAPABILITY))
            .and_then(|_| run_voice_command(command, text.clone().into_bytes(), "voice/tts"))
        {
            Ok(bytes) => bytes,
            Err(err) => return voice_diagnostic_packet(err.to_string()),
        },
        None => text.into_bytes(),
    };
    StreamPacket::Pcm(bytes_to_pcm(bytes))
}

fn stt_stream_packet(
    cx: &mut Cx,
    locale: &str,
    command: Option<&str>,
    packet: Option<StreamPacket>,
    expr: Expr,
) -> StreamPacket {
    let input = match packet {
        Some(StreamPacket::Pcm(packet)) => pcm_bytes(&packet),
        Some(StreamPacket::Data(packet)) => flatten_expr_text(&packet.payload).into_bytes(),
        Some(packet) => flatten_expr_text(&packet.to_expr()).into_bytes(),
        None => flatten_expr_text(&expr).into_bytes(),
    };
    let text = match command {
        Some(command) => match cx
            .require(&CapabilityName::new(VOICE_CAPABILITY))
            .and_then(|_| run_voice_command(command, input, "voice/stt"))
            .and_then(|bytes| {
                String::from_utf8(bytes).map_err(|_| {
                    Error::Eval("voice/stt command returned non-utf-8 bytes".to_owned())
                })
            }) {
            Ok(text) => text,
            Err(err) => return voice_diagnostic_packet(err.to_string()),
        },
        None => String::from_utf8(input).unwrap_or_else(|_| "pcm transcript".to_owned()),
    };
    StreamPacket::data(
        Symbol::qualified("stream/data", "voice-transcript"),
        Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("locale")),
                Expr::String(locale.to_owned()),
            ),
            (Expr::Symbol(Symbol::new("text")), Expr::String(text)),
        ]),
    )
}

fn packet_text(packet: Option<StreamPacket>, expr: Expr) -> String {
    match packet {
        Some(StreamPacket::Data(packet)) => flatten_expr_text(&packet.payload),
        Some(packet) => flatten_expr_text(&packet.to_expr()),
        None => flatten_expr_text(&expr),
    }
}

fn bytes_to_pcm(bytes: Vec<u8>) -> PcmPacket {
    let samples = if bytes.is_empty() {
        vec![0]
    } else {
        bytes
            .into_iter()
            .map(|byte| (i16::from(byte) - 128) * 256)
            .collect::<Vec<_>>()
    };
    PcmPacket::i16(1, samples.len(), samples).expect("synthetic voice PCM shape is valid")
}

fn pcm_bytes(packet: &PcmPacket) -> Vec<u8> {
    packet
        .samples_i16()
        .iter()
        .map(|sample| sample.to_be_bytes())
        .flat_map(|bytes| bytes.into_iter())
        .collect()
}

fn voice_diagnostic_packet(message: String) -> StreamPacket {
    StreamPacket::Diagnostic(StreamDiagnostic::new(
        Symbol::qualified("voice", "StreamError"),
        message,
    ))
}
