use sim_kernel::{CodecId, Error, Result};

pub(crate) use decode_http_base64 as base64_decode;
pub(crate) use sim_codec_binary_base64::encode_base64 as base64_encode;

const HTTP_BASE64_CODEC_ID: CodecId = CodecId(0);

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

pub(crate) fn decode_http_base64(text: &str) -> Result<Vec<u8>> {
    sim_codec_binary_base64::decode_base64(HTTP_BASE64_CODEC_ID, text)
}

pub(crate) fn websocket_accept_value(client_key: &str) -> String {
    let mut bytes = client_key.as_bytes().to_vec();
    bytes.extend_from_slice(WS_GUID.as_bytes());
    base64_encode(&sha1_digest(&bytes))
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
