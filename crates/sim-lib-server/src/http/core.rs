use sim_kernel::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParsedUrl {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WsMessage {
    Binary(Vec<u8>),
    Close,
}

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

impl From<sim_lib_net_core::UrlParts> for ParsedUrl {
    fn from(parts: sim_lib_net_core::UrlParts) -> Self {
        Self {
            scheme: parts.scheme,
            host: parts.host,
            port: parts.port,
            path: parts.path,
        }
    }
}

// URL parsing defers to the shared `sim_lib_net_core` primitive
// (`parse_url_for_scheme_preserving_path`): it resolves ws/wss (80/443) and
// http/https default ports, rejects a scheme mismatch, and preserves a caller's
// trailing-slash path. This server keeps only the transport error mapping local.
pub(crate) fn parse_url(url: &str, expected_scheme: &str, default_path: &str) -> Result<ParsedUrl> {
    sim_lib_net_core::parse_url_for_scheme_preserving_path(url, expected_scheme, default_path)
        .map(ParsedUrl::from)
        .map_err(|error| Error::Eval(format!("invalid {expected_scheme} url {url}: {error}")))
}

pub(crate) fn format_url(parsed: &ParsedUrl) -> String {
    format!(
        "{}://{}:{}{}",
        parsed.scheme, parsed.host, parsed.port, parsed.path
    )
}

pub(crate) fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub(crate) fn base64_decode(text: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(Error::HostError("invalid base64 length".to_owned()));
    }
    for chunk in bytes.chunks(4) {
        let c0 = decode_b64(chunk[0])?;
        let c1 = decode_b64(chunk[1])?;
        let c2 = if chunk[2] == b'=' {
            None
        } else {
            Some(decode_b64(chunk[2])?)
        };
        let c3 = if chunk[3] == b'=' {
            None
        } else {
            Some(decode_b64(chunk[3])?)
        };
        out.push((c0 << 2) | (c1 >> 4));
        if let Some(c2) = c2 {
            out.push(((c1 & 0x0f) << 4) | (c2 >> 2));
            if let Some(c3) = c3 {
                out.push(((c2 & 0x03) << 6) | c3);
            }
        }
    }
    Ok(out)
}

pub(crate) fn websocket_accept_value(client_key: &str) -> String {
    let mut bytes = client_key.as_bytes().to_vec();
    bytes.extend_from_slice(WS_GUID.as_bytes());
    base64_encode(&sha1_digest(&bytes))
}

fn decode_b64(byte: u8) -> Result<u8> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(Error::HostError("invalid base64 byte".to_owned())),
    }
}

fn sha1_digest(bytes: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let bit_len = (bytes.len() as u64) * 8;
    let mut padded = bytes.to_vec();
    padded.push(0x80);
    while !(padded.len() + 8).is_multiple_of(64) {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (index, block) in chunk.chunks_exact(4).enumerate() {
            w[index] = u32::from_be_bytes([block[0], block[1], block[2], block[3]]);
        }
        for index in 16..80 {
            w[index] = (w[index - 3] ^ w[index - 8] ^ w[index - 14] ^ w[index - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for (index, word) in w.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => (((b & c) | ((!b) & d)), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => (((b & c) | (b & d) | (c & d)), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}
