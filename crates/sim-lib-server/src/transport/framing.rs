use std::io::{ErrorKind, Read, Write};
use std::sync::Arc;

use sim_codec_binary::{decode_frame, encode_frame};
use sim_kernel::{Cx, Error, Expr, Result, Symbol};

use crate::{EvalSite, FrameEnvelope, FrameKind, ServerFrame};

mod header;

pub(crate) use header::{endpoint_key, frame_from_header_expr, frame_header_expr};

const FRAME_PREFIX_BYTES: usize = 8;

pub(crate) fn negotiate_codec(
    cx: &mut Cx,
    transport: &mut dyn super::ConnectionTransport,
    offered_codecs: &[Symbol],
) -> Result<Symbol> {
    let _ = select_default_codec(offered_codecs)?;
    let request = ServerFrame {
        version: 1,
        codec: Symbol::qualified("codec", "binary"),
        msg_id: Some(1),
        correlate: None,
        kind: FrameKind::Negotiate {
            codecs: offered_codecs.to_vec(),
        },
        envelope: FrameEnvelope::default(),
        payload: Vec::new(),
    };
    transport.send_frame(cx, request)?;
    let reply = transport
        .recv_frame(cx, None)?
        .ok_or_else(|| Error::HostError("transport negotiation returned no reply".to_owned()))?;
    match reply.kind {
        FrameKind::Negotiate { codecs } => codecs
            .first()
            .cloned()
            .ok_or_else(|| Error::HostError("transport negotiation returned no codec".to_owned())),
        other => Err(Error::HostError(format!(
            "transport negotiation expected negotiate reply, found {}",
            other.as_symbol()
        ))),
    }
}

pub(crate) fn select_default_codec(offered_codecs: &[Symbol]) -> Result<Symbol> {
    if offered_codecs.is_empty() {
        return Err(Error::Eval(
            "transport negotiation requires at least one offered codec".to_owned(),
        ));
    }
    offered_codecs
        .iter()
        .find(|codec| **codec == Symbol::qualified("codec", "binary"))
        .cloned()
        .or_else(|| offered_codecs.first().cloned())
        .ok_or_else(|| {
            Error::Eval("transport negotiation requires at least one offered codec".to_owned())
        })
}

/// Encodes a frame into its length-prefixed transport byte form.
///
/// Lays out a fixed prefix of big-endian header and payload lengths followed by
/// the encoded header and the raw payload; errors if either part exceeds the
/// transport size limit.
pub fn encode_transport_frame(frame: &ServerFrame) -> Result<Vec<u8>> {
    let header = encode_frame(&frame_header_expr(frame))?.0;
    if header.len() > super::MAX_TRANSPORT_FRAME_BYTES
        || frame.payload.len() > super::MAX_TRANSPORT_FRAME_BYTES
    {
        return Err(Error::HostError(
            "transport frame exceeds size limit".to_owned(),
        ));
    }

    let header_len = u32::try_from(header.len())
        .map_err(|_| Error::HostError("transport header exceeds u32".to_owned()))?;
    let payload_len = u32::try_from(frame.payload.len())
        .map_err(|_| Error::HostError("transport payload exceeds u32".to_owned()))?;

    let mut bytes = Vec::with_capacity(FRAME_PREFIX_BYTES + header.len() + frame.payload.len());
    bytes.extend_from_slice(&header_len.to_be_bytes());
    bytes.extend_from_slice(&payload_len.to_be_bytes());
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&frame.payload);
    Ok(bytes)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn write_frame_to<W: Write>(writer: &mut W, frame: &ServerFrame) -> Result<()> {
    let bytes = encode_transport_frame(frame)?;
    writer.write_all(&bytes).map_err(io_to_host)?;
    writer.flush().map_err(io_to_host)
}

/// Decodes a length-prefixed transport frame from `bytes`.
///
/// Reads the header and payload lengths from the prefix, validates them against
/// the transport size limit, and reconstructs the [`ServerFrame`]; errors on a
/// truncated or oversized buffer.
pub fn decode_transport_frame(bytes: &[u8]) -> Result<ServerFrame> {
    if bytes.len() < FRAME_PREFIX_BYTES {
        return Err(Error::HostError(
            "truncated transport frame prefix".to_owned(),
        ));
    }

    let header_len = u32::from_be_bytes(bytes[0..4].try_into().expect("prefix slice")) as usize;
    let payload_len = u32::from_be_bytes(bytes[4..8].try_into().expect("prefix slice")) as usize;
    if header_len > super::MAX_TRANSPORT_FRAME_BYTES
        || payload_len > super::MAX_TRANSPORT_FRAME_BYTES
    {
        return Err(Error::HostError(
            "transport frame exceeds size limit".to_owned(),
        ));
    }

    let total_len = FRAME_PREFIX_BYTES
        .checked_add(header_len)
        .and_then(|value| value.checked_add(payload_len))
        .ok_or_else(|| Error::HostError("transport frame length overflow".to_owned()))?;
    if bytes.len() < total_len {
        return Err(Error::HostError(
            "truncated transport frame body".to_owned(),
        ));
    }

    let header_bytes = &bytes[FRAME_PREFIX_BYTES..FRAME_PREFIX_BYTES + header_len];
    let payload = bytes
        [FRAME_PREFIX_BYTES + header_len..FRAME_PREFIX_BYTES + header_len + payload_len]
        .to_vec();
    let (_, header) = decode_frame(sim_kernel::CodecId(0), header_bytes)?;
    frame_from_header_expr(&header, payload)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn read_frame_from<R: Read>(reader: &mut R) -> Result<Option<ServerFrame>> {
    let mut prefix = [0u8; FRAME_PREFIX_BYTES];
    match read_exact_or_eof(reader, &mut prefix)? {
        ReadOutcome::Eof => return Ok(None),
        ReadOutcome::Filled => {}
    }

    let header_len = u32::from_be_bytes(prefix[0..4].try_into().expect("prefix slice")) as usize;
    let payload_len = u32::from_be_bytes(prefix[4..8].try_into().expect("prefix slice")) as usize;
    if header_len > super::MAX_TRANSPORT_FRAME_BYTES
        || payload_len > super::MAX_TRANSPORT_FRAME_BYTES
    {
        return Err(Error::HostError(
            "transport frame exceeds size limit".to_owned(),
        ));
    }

    let body_len = header_len
        .checked_add(payload_len)
        .ok_or_else(|| Error::HostError("transport frame length overflow".to_owned()))?;
    let mut body = vec![0u8; body_len];
    reader.read_exact(&mut body).map_err(io_to_host)?;

    let mut frame = Vec::with_capacity(FRAME_PREFIX_BYTES + body.len());
    frame.extend_from_slice(&prefix);
    frame.extend_from_slice(&body);
    decode_transport_frame(&frame).map(Some)
}

pub(crate) fn answer_or_negotiate(
    cx: &mut Cx,
    site: &Arc<dyn EvalSite>,
    frame: ServerFrame,
) -> Result<ServerFrame> {
    match &frame.kind {
        FrameKind::Negotiate { codecs } => {
            let selected = codecs
                .iter()
                .find(|codec| site.codecs().iter().any(|installed| installed == *codec))
                .cloned()
                .ok_or_else(|| {
                    Error::Eval("transport negotiation found no shared codec".to_owned())
                })?;
            Ok(ServerFrame {
                version: frame.version,
                codec: selected.clone(),
                msg_id: None,
                correlate: frame.msg_id,
                kind: FrameKind::Negotiate {
                    codecs: vec![selected],
                },
                envelope: FrameEnvelope::default(),
                payload: Vec::new(),
            })
        }
        _ => site.answer(cx, frame),
    }
}

pub(crate) fn route_frame_bytes(
    cx: &mut Cx,
    site: &Arc<dyn EvalSite>,
    bytes: &[u8],
) -> Result<Vec<u8>> {
    let frame = decode_transport_frame(bytes)?;
    let reply = answer_or_negotiate(cx, site, frame)?;
    encode_transport_frame(&reply)
}

pub(crate) fn update_negotiated_codec_from_reply(
    runtime: &Arc<crate::ServerRuntime>,
    session_id: u64,
    frame: &ServerFrame,
    reply: &ServerFrame,
) -> Result<()> {
    if !matches!(frame.kind, FrameKind::Negotiate { .. }) {
        return Ok(());
    }
    let FrameKind::Negotiate { codecs } = &reply.kind else {
        return Ok(());
    };
    let Some(selected) = codecs.first() else {
        return Ok(());
    };
    runtime.update_session_codec(session_id, selected.clone())
}

pub(crate) fn error_frame_from_error(
    cx: &mut Cx,
    frame: &ServerFrame,
    error: &Error,
) -> Result<ServerFrame> {
    let codec = match &frame.envelope.reply_codec_hint {
        Some(hint) if cx.registry().codec_by_symbol(hint).is_some() => hint.clone(),
        _ => frame.codec.clone(),
    };
    let mut reply = ServerFrame::from_expr(
        cx,
        codec,
        FrameKind::Error,
        &Expr::String(error.to_string()),
        frame.envelope.consistency,
        Vec::new(),
        false,
    )?;
    reply.correlate = frame.msg_id;
    Ok(reply)
}

pub(crate) fn io_to_host(error: std::io::Error) -> Error {
    Error::HostError(format!("io {:?}: {}", error.kind(), error))
}

pub(crate) fn is_timeout(error: &Error) -> bool {
    matches!(error, Error::HostError(message) if message.contains("TimedOut") || message.contains("WouldBlock"))
}

pub(crate) enum ReadOutcome {
    Eof,
    Filled,
}

pub(crate) fn read_exact_or_eof<R: Read>(
    reader: &mut R,
    mut buffer: &mut [u8],
) -> Result<ReadOutcome> {
    let mut read_any = false;
    while !buffer.is_empty() {
        match reader.read(buffer) {
            Ok(0) if !read_any => return Ok(ReadOutcome::Eof),
            Ok(0) => {
                return Err(Error::HostError(
                    "truncated transport frame prefix".to_owned(),
                ));
            }
            Ok(read) => {
                read_any = true;
                let (_, rest) = buffer.split_at_mut(read);
                buffer = rest;
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(io_to_host(error)),
        }
    }
    Ok(ReadOutcome::Filled)
}
