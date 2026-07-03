use std::io::{ErrorKind, Read, Write};

use sim_kernel::{Error, Result};

use crate::transport::MAX_TRANSPORT_FRAME_BYTES;

use super::core::WsMessage;

pub(crate) fn read_ws_message<R: Read>(reader: &mut R) -> Result<Option<WsMessage>> {
    let mut first = [0u8; 2];
    match read_exact_or_eof(reader, &mut first)? {
        HeadRead::Eof => return Ok(None),
        HeadRead::Filled => {}
    }
    let fin = (first[0] & 0x80) != 0;
    let opcode = first[0] & 0x0f;
    if !fin {
        return Err(Error::HostError(
            "fragmented websocket frames are not supported".to_owned(),
        ));
    }
    let masked = (first[1] & 0x80) != 0;
    let mut payload_len = u64::from(first[1] & 0x7f);
    if payload_len == 126 {
        let mut buf = [0u8; 2];
        reader.read_exact(&mut buf).map_err(io_to_host)?;
        payload_len = u64::from(u16::from_be_bytes(buf));
    } else if payload_len == 127 {
        let mut buf = [0u8; 8];
        reader.read_exact(&mut buf).map_err(io_to_host)?;
        payload_len = u64::from_be_bytes(buf);
    }
    let payload_len = usize::try_from(payload_len)
        .map_err(|_| Error::HostError("websocket payload length overflow".to_owned()))?;
    if payload_len > MAX_TRANSPORT_FRAME_BYTES {
        return Err(Error::HostError(
            "websocket payload exceeds size limit".to_owned(),
        ));
    }
    let mut mask = [0u8; 4];
    if masked {
        reader.read_exact(&mut mask).map_err(io_to_host)?;
    }
    let mut payload = vec![0u8; payload_len];
    reader.read_exact(&mut payload).map_err(io_to_host)?;
    if masked {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }
    match opcode {
        0x2 => Ok(Some(WsMessage::Binary(payload))),
        0x8 => Ok(Some(WsMessage::Close)),
        other => Err(Error::HostError(format!(
            "unsupported websocket opcode {other}"
        ))),
    }
}

pub(crate) fn write_ws_binary<W: Write>(
    writer: &mut W,
    payload: &[u8],
    masked: bool,
) -> Result<()> {
    write_ws_frame(writer, 0x2, payload, masked)
}

pub(crate) fn write_ws_close<W: Write>(writer: &mut W, masked: bool) -> Result<()> {
    write_ws_frame(writer, 0x8, &[], masked)
}

fn write_ws_frame<W: Write>(
    writer: &mut W,
    opcode: u8,
    payload: &[u8],
    masked: bool,
) -> Result<()> {
    if payload.len() > MAX_TRANSPORT_FRAME_BYTES {
        return Err(Error::HostError(
            "websocket payload exceeds size limit".to_owned(),
        ));
    }
    let mut header = vec![0x80 | opcode];
    let mask_bit = if masked { 0x80 } else { 0x00 };
    match payload.len() {
        len @ 0..=125 => header.push(mask_bit | len as u8),
        len @ 126..=65535 => {
            header.push(mask_bit | 126);
            header.extend_from_slice(&(len as u16).to_be_bytes());
        }
        len => {
            header.push(mask_bit | 127);
            header.extend_from_slice(&(len as u64).to_be_bytes());
        }
    }
    writer.write_all(&header).map_err(io_to_host)?;
    if masked {
        let mask = [0x13, 0x37, 0x42, 0x99];
        writer.write_all(&mask).map_err(io_to_host)?;
        let masked_payload = payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4])
            .collect::<Vec<_>>();
        writer.write_all(&masked_payload).map_err(io_to_host)?;
    } else {
        writer.write_all(payload).map_err(io_to_host)?;
    }
    writer.flush().map_err(io_to_host)
}

enum HeadRead {
    Eof,
    Filled,
}

fn read_exact_or_eof<R: Read>(reader: &mut R, mut buffer: &mut [u8]) -> Result<HeadRead> {
    let mut read_any = false;
    while !buffer.is_empty() {
        match reader.read(buffer) {
            Ok(0) if !read_any => return Ok(HeadRead::Eof),
            Ok(0) => return Err(Error::HostError("truncated websocket frame".to_owned())),
            Ok(read) => {
                read_any = true;
                let (_, rest) = buffer.split_at_mut(read);
                buffer = rest;
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(io_to_host(error)),
        }
    }
    Ok(HeadRead::Filled)
}

fn io_to_host(error: std::io::Error) -> Error {
    Error::HostError(format!("io {:?}: {}", error.kind(), error))
}
