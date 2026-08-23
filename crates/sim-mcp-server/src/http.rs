//! Product-owned native binding for the protocol-neutral raw HTTP seam.

use sim_cancel::Cancellation;
use sim_codec_mcp::McpCodecLib;
use sim_kernel::{Cx, HandleSeed, Result, capability::CapabilitySet};
use sim_lib_mcp::{
    CliOptions, HttpLauncher, McpProfile, McpService, Principal, ServerDescription, Transport,
};
use sim_lib_mcp_http::{
    AuthRejection, HttpClock, IdentityProvider, LegacyDispatch, McpHttpHandler,
    OAuthIdentityProvider, OriginPolicy, RequestIdentity, ScopeGrantPolicy, ServerPolicy,
    ServiceDispatch,
};
use sim_lib_mcp_legacy::LegacyConnection;
use sim_lib_oauth_core::{
    AccessTokenVerifier, OAuthError, ScopeSet, Secret, SecureUrl, VerifiedPrincipal,
};
use sim_lib_server::{
    BodyLimits, BodyReader, RawConnection, RawHttpServer, RequestHead, RequestScope, ResponseHead,
    ResponseWriter, TrailersPolicy,
};
use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    time::Duration,
};

pub(crate) struct ProductHttpLauncher;

struct Anonymous;
impl IdentityProvider for Anonymous {
    fn identify(&self, _: &RequestHead) -> std::result::Result<RequestIdentity, AuthRejection> {
        Ok(RequestIdentity {
            principal: Principal::new("anonymous-loopback"),
            grants: CapabilitySet::new(),
        })
    }
}
struct ProductIdentity(Box<dyn IdentityProvider>);
impl IdentityProvider for ProductIdentity {
    fn identify(&self, head: &RequestHead) -> std::result::Result<RequestIdentity, AuthRejection> {
        self.0.identify(head)
    }
}
struct ConfiguredVerifier {
    token: Secret,
    subject: String,
}
impl AccessTokenVerifier for ConfiguredVerifier {
    fn verify(
        &self,
        token: &Secret,
        issuer: &SecureUrl,
        resource: &SecureUrl,
        scopes: &ScopeSet,
        now: u64,
    ) -> sim_lib_oauth_core::Result<VerifiedPrincipal> {
        if token.expose() != self.token.expose() {
            return Err(OAuthError("invalid access token"));
        }
        VerifiedPrincipal::new(
            issuer.clone(),
            self.subject.clone(),
            [resource.as_str().to_owned()].into(),
            resource.clone(),
            scopes.clone(),
            now.saturating_add(300),
            None,
        )
    }
}
struct Clock;
impl HttpClock for Clock {
    fn http_date(&self) -> String {
        "Thu, 01 Jan 1970 00:00:00 GMT".into()
    }
    fn keepalive(&self) -> Option<u64> {
        None
    }
}

impl HttpLauncher for ProductHttpLauncher {
    fn serve(&self, cx: &mut Cx, options: &CliOptions) -> Result<()> {
        let Transport::Http { address, route } = &options.transport else {
            return Ok(());
        };
        let mut codec_cx = cx.fork_from_seed(HandleSeed::new(0x4d43_5048));
        let codec = McpCodecLib::new(codec_cx.registry_mut().fresh_codec_id());
        codec_cx.load_lib(&codec)?;
        let host_seed = cx.fork_from_seed(HandleSeed::new(0x4d43_5053));
        let (mut policy, identity) = if options.anonymous_loopback {
            (
                ServerPolicy::new(route, OriginPolicy::LoopbackOnly, options.max_body_bytes)?,
                ProductIdentity(Box::new(Anonymous)),
            )
        } else {
            let key_ring = std::fs::read(options.key_ring.as_ref().expect("validated key ring"))
                .map_err(sim_kernel::Error::host_io)?;
            if key_ring.is_empty() {
                return Err(sim_kernel::Error::Eval(
                    "protected-state key ring is empty".into(),
                ));
            }
            let verifier_path = options.oauth_verifier.as_ref().expect("validated verifier");
            let verifier_material =
                std::fs::read_to_string(verifier_path).map_err(sim_kernel::Error::host_io)?;
            let (token, subject) =
                verifier_material
                    .trim_end()
                    .split_once('\n')
                    .ok_or_else(|| {
                        sim_kernel::Error::Eval(
                            "OAuth verifier file requires token and subject lines".into(),
                        )
                    })?;
            let verifier = ConfiguredVerifier {
                token: Secret::new(token).map_err(|e| sim_kernel::Error::Eval(e.to_string()))?,
                subject: subject.to_owned(),
            };
            let issuer = SecureUrl::parse(
                options.oauth_issuer.as_deref().expect("validated issuer"),
                false,
            )
            .map_err(|e| sim_kernel::Error::Eval(e.to_string()))?;
            let resource = SecureUrl::parse(
                options
                    .oauth_resource
                    .as_deref()
                    .expect("validated resource"),
                false,
            )
            .map_err(|e| sim_kernel::Error::Eval(e.to_string()))?;
            let metadata = SecureUrl::parse(
                options
                    .oauth_metadata
                    .as_deref()
                    .expect("validated metadata"),
                false,
            )
            .map_err(|e| sim_kernel::Error::Eval(e.to_string()))?;
            let scopes = ScopeSet::parse(&options.oauth_scopes.join(" "))
                .map_err(|e| sim_kernel::Error::Eval(e.to_string()))?;
            let identity = OAuthIdentityProvider::new(
                verifier,
                || {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |v| v.as_secs())
                },
                issuer,
                resource,
                metadata,
                scopes,
                ScopeGrantPolicy::default(),
            );
            (
                ServerPolicy::remote(route, options.origins.clone(), options.max_body_bytes, true)?,
                ProductIdentity(Box::new(identity)),
            )
        };
        if let Some(endpoint) = &options.legacy_endpoint {
            policy = policy.with_legacy_endpoint(endpoint)?;
        }
        let mut handler = McpHttpHandler::new(
            policy,
            ServiceDispatch::new(
                McpService::new(ServerDescription::new(
                    "sim-mcp-server",
                    env!("CARGO_PKG_VERSION"),
                    McpProfile::all(),
                )),
                host_seed,
            ),
            identity,
            Clock,
            codec_cx,
        );
        if options.legacy_endpoint.is_some() {
            let legacy_seed = cx.fork_from_seed(HandleSeed::new(0x4d43_504c));
            handler = handler.with_legacy(LegacyDispatch::new(
                LegacyConnection::new(
                    McpService::new(ServerDescription::new(
                        "sim-mcp-server",
                        env!("CARGO_PKG_VERSION"),
                        McpProfile::all(),
                    )),
                    "product-http",
                    Principal::new("legacy-http"),
                ),
                legacy_seed,
            ))?;
        }
        let server = RawHttpServer::new(
            handler,
            BodyLimits {
                max_request_bytes: options.max_body_bytes,
                max_chunk_bytes: options.max_body_bytes.min(64 * 1024),
            },
            TrailersPolicy::Deny,
            Duration::from_millis(options.timeout_ms),
        )?;
        let listener = TcpListener::bind(address).map_err(sim_kernel::Error::host_io)?;
        let lifetime = Cancellation::new();
        for stream in listener.incoming() {
            let mut connection = NativeConnection::read(
                stream.map_err(sim_kernel::Error::host_io)?,
                options.max_body_bytes,
                Duration::from_millis(options.timeout_ms),
            )
            .map_err(sim_kernel::Error::host_io)?;
            server.serve(&mut connection, &lifetime)?;
            connection.flush().map_err(sim_kernel::Error::host_io)?;
        }
        Ok(())
    }
}

struct NativeConnection {
    head: RequestHead,
    body: NativeBody,
    response: NativeResponse,
}
impl NativeConnection {
    fn read(mut stream: TcpStream, cap: usize, timeout: Duration) -> io::Result<Self> {
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        let peer = stream.peer_addr().ok().map(|v| v.to_string());
        let local = stream.local_addr().ok().map(|v| v.to_string());
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            let count = stream.read(&mut chunk)?;
            if count == 0 || bytes.len() + count > 64 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "bounded HTTP head incomplete",
                ));
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        let split = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let text = std::str::from_utf8(&bytes[..split - 4])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "HTTP head is not UTF-8"))?;
        let mut lines = text.split("\r\n");
        let mut request = lines.next().unwrap_or_default().split_ascii_whitespace();
        let method = request
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing method"))?
            .to_owned();
        let target = request
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing target"))?
            .to_owned();
        if request.next() != Some("HTTP/1.1") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP/1.1 required",
            ));
        }
        let mut headers = Vec::new();
        for line in lines {
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed header"))?;
            headers.push((name.to_owned(), value.trim().to_owned()));
        }
        let lengths: Vec<_> = headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("content-length"))
            .collect();
        if lengths.len() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "one Content-Length required",
            ));
        }
        let length: usize = lengths[0]
            .1
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid Content-Length"))?;
        if length > cap {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request body cap exceeded",
            ));
        }
        let mut body = bytes[split..].to_vec();
        if body.len() > length {
            body.truncate(length);
        }
        let received = body.len();
        body.resize(length, 0);
        stream.read_exact(&mut body[received..])?;
        Ok(Self {
            head: RequestHead {
                method,
                target,
                headers,
                peer,
                local,
            },
            body: NativeBody(Some(body)),
            response: NativeResponse {
                stream,
                bytes: Vec::new(),
                finished: false,
            },
        })
    }
    fn flush(&mut self) -> io::Result<()> {
        self.response.stream.write_all(&self.response.bytes)
    }
}
impl RawConnection for NativeConnection {
    fn parts(&mut self) -> (&RequestHead, &mut dyn BodyReader, &mut dyn ResponseWriter) {
        (&self.head, &mut self.body, &mut self.response)
    }
}
struct NativeBody(Option<Vec<u8>>);
impl BodyReader for NativeBody {
    fn next_chunk(&mut self, _: &RequestScope) -> io::Result<Option<Vec<u8>>> {
        Ok(self.0.take())
    }
}
struct NativeResponse {
    stream: TcpStream,
    bytes: Vec<u8>,
    finished: bool,
}
impl ResponseWriter for NativeResponse {
    fn write_head(&mut self, head: ResponseHead, _: &RequestScope) -> io::Result<()> {
        write!(&mut self.bytes, "HTTP/1.1 {} SIM\r\n", head.status)?;
        for (n, v) in head.headers {
            write!(&mut self.bytes, "{n}: {v}\r\n")?;
        }
        Ok(())
    }
    fn write_chunk(&mut self, chunk: &[u8], _: &RequestScope) -> io::Result<()> {
        if !self.finished {
            write!(
                &mut self.bytes,
                "Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n",
                chunk.len()
            )?;
        } else {
            write!(&mut self.bytes, "{:x}\r\n", chunk.len())?;
        }
        self.bytes.extend_from_slice(chunk);
        self.bytes.extend_from_slice(b"\r\n");
        self.finished = true;
        Ok(())
    }
    fn finish(&mut self, _: &[(String, String)], _: &RequestScope) -> io::Result<()> {
        if !self.finished {
            self.bytes
                .extend_from_slice(b"Content-Length: 0\r\nConnection: close\r\n\r\n");
        } else {
            self.bytes.extend_from_slice(b"0\r\n\r\n");
        }
        Ok(())
    }
}
