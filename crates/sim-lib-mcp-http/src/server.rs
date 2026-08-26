/// Clock projection used only to date forbidden responses and bound SSE comments.
pub trait HttpClock: Send + Sync {
    /// Current RFC 9110 HTTP-date text.
    fn http_date(&self) -> String;
    /// One monotonic keep-alive sequence number, if a comment is due.
    fn keepalive(&self) -> Option<u64>;
}

/// Streamable HTTP request handler over the shared raw server seam.
pub struct McpHttpHandler<D, I, K> {
    policy: ServerPolicy,
    modern: D,
    legacy: Option<Box<dyn McpDispatch>>,
    identity: I,
    clock: K,
    codec: Mutex<Cx>,
}

impl<D, I, K> McpHttpHandler<D, I, K>
where
    D: McpDispatch,
    I: IdentityProvider,
    K: HttpClock,
{
    /// Creates a modern-only handler. `codec_cx` must have `codec:mcp` loaded.
    pub fn new(policy: ServerPolicy, modern: D, identity: I, clock: K, codec_cx: Cx) -> Self {
        Self {
            policy,
            modern,
            legacy: None,
            identity,
            clock,
            codec: Mutex::new(codec_cx),
        }
    }

    /// Attaches an explicit legacy dispatcher to the configured legacy endpoint.
    pub fn with_legacy(mut self, legacy: impl McpDispatch + 'static) -> Result<Self> {
        if self.policy.legacy_endpoint.is_none() {
            return Err(Error::Eval(
                "legacy dispatcher requires an explicit endpoint".into(),
            ));
        }
        self.legacy = Some(Box::new(legacy));
        Ok(self)
    }
}

impl<D, I, K> RawHandler for McpHttpHandler<D, I, K>
where
    D: McpDispatch,
    I: IdentityProvider,
    K: HttpClock,
{
    fn handle(
        &self,
        head: &RequestHead,
        body: &mut dyn BodyReader,
        response: &mut dyn ResponseWriter,
        scope: &RequestScope,
    ) -> Result<()> {
        let endpoint = target_path(&head.target);
        let dispatch: &dyn McpDispatch = if endpoint == self.policy.endpoint {
            &self.modern
        } else if self.policy.legacy_endpoint.as_deref() == Some(endpoint) {
            self.legacy
                .as_deref()
                .ok_or_else(|| Error::Eval("legacy endpoint unavailable".into()))?
        } else {
            return empty(
                response,
                scope,
                404,
                common_headers(&self.clock.http_date()),
            );
        };
        if head.method != "POST" {
            let mut headers = common_headers(&self.clock.http_date());
            headers.push(("Allow".into(), "POST".into()));
            return empty(response, scope, 405, headers);
        }
        let origins = header_values(head, "origin");
        if !self.policy.origins.admits(&origins) {
            return empty(
                response,
                scope,
                403,
                common_headers(&self.clock.http_date()),
            );
        }
        if unique_header(head, "content-type")
            .ok()
            .flatten()
            .is_none_or(|v| media_type(v) != JSON)
        {
            return protocol_error(
                response,
                scope,
                400,
                "content-type must be application/json",
                &self.clock.http_date(),
                &self.codec,
            );
        }
        let accept = match unique_header(head, "accept") {
            Ok(Some(value)) if accepts(value, JSON) || accepts(value, SSE) => value,
            _ => {
                return protocol_error(
                    response,
                    scope,
                    400,
                    "accept must include application/json or text/event-stream",
                    &self.clock.http_date(),
                    &self.codec,
                );
            }
        };
        if duplicate_mcp_headers(head) {
            return protocol_error(
                response,
                scope,
                400,
                "duplicate MCP projection header",
                &self.clock.http_date(),
                &self.codec,
            );
        }
        let bytes = read_body(body, scope, self.policy.max_body_bytes)
            .map_err(|e| Error::HostError(e.to_string()))?;
        let envelope = match decode(&self.codec, bytes) {
            Ok(value) => value,
            Err(error) => {
                return protocol_error(
                    response,
                    scope,
                    400,
                    &error.to_string(),
                    &self.clock.http_date(),
                    &self.codec,
                );
            }
        };
        if let Err(message) = check_projection(head, &envelope) {
            return protocol_error(
                response,
                scope,
                400,
                message,
                &self.clock.http_date(),
                &self.codec,
            );
        }
        let identity = match self.identity.identify(head) {
            Ok(identity) => identity,
            Err(rejection) => {
                let mut headers = common_headers(&self.clock.http_date());
                headers.push(("WWW-Authenticate".into(), rejection.challenge));
                return empty(response, scope, rejection.status, headers);
            }
        };
        let request_id = request_id(&envelope);
        let context = RequestContext::new(
            request_id,
            PROTOCOL,
            NegotiatedExtensions::none(),
            identity.principal,
            CachePolicy::Bypass,
        )
        .with_principal_grants(identity.grants);
        let replies = dispatch.dispatch(&context, scope.cancellation(), envelope.clone())?;
        if matches!(envelope, McpEnvelope::Notification(_)) {
            if !replies.is_empty() {
                return Err(Error::Eval(
                    "notification dispatcher emitted a response".into(),
                ));
            }
            return empty(
                response,
                scope,
                202,
                common_headers(&self.clock.http_date()),
            );
        }
        if accepts(accept, SSE) {
            return write_sse(
                response,
                scope,
                &replies,
                &self.clock,
                self.policy.max_keepalives,
                &self.codec,
            );
        }
        if replies.len() != 1 {
            return protocol_error(
                response,
                scope,
                400,
                "JSON response requires exactly one message",
                &self.clock.http_date(),
                &self.codec,
            );
        }
        let bytes = encode(&self.codec, &replies[0])?;
        let mut headers = common_headers(&self.clock.http_date());
        headers.push(("Content-Type".into(), JSON.into()));
        response
            .write_head(
                ResponseHead {
                    status: 200,
                    headers,
                },
                scope,
            )
            .inspect_err(|_| scope.cancel_peer_drop())
            .map_err(Error::host_io)?;
        response
            .write_chunk(&bytes, scope)
            .inspect_err(|_| scope.cancel_peer_drop())
            .map_err(Error::host_io)?;
        response
            .finish(&[], scope)
            .inspect_err(|_| scope.cancel_peer_drop())
            .map_err(Error::host_io)
    }
}

fn target_path(target: &str) -> &str {
    target.split('?').next().unwrap_or(target)
}
fn media_type(value: &str) -> &str {
    value.split(';').next().unwrap_or(value).trim()
}
fn accepts(value: &str, wanted: &str) -> bool {
    value
        .split(',')
        .any(|v| media_type(v).eq_ignore_ascii_case(wanted) || media_type(v) == "*/*")
}
fn header_values<'a>(head: &'a RequestHead, name: &str) -> Vec<&'a str> {
    head.headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
        .collect()
}
fn unique_header<'a>(
    head: &'a RequestHead,
    name: &str,
) -> std::result::Result<Option<&'a str>, ()> {
    let v = header_values(head, name);
    if v.len() > 1 {
        Err(())
    } else {
        Ok(v.first().copied())
    }
}
fn duplicate_mcp_headers(head: &RequestHead) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    head.headers
        .iter()
        .filter(|(n, _)| {
            n.to_ascii_lowercase().starts_with("mcp-")
                || n.to_ascii_lowercase().starts_with("x-mcp-")
        })
        .any(|(n, _)| !seen.insert(n.to_ascii_lowercase()))
}
fn read_body(body: &mut dyn BodyReader, scope: &RequestScope, cap: usize) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(chunk) = body.next_chunk(scope)? {
        if out.len().saturating_add(chunk.len()) > cap {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP body cap exceeded",
            ));
        }
        out.extend(chunk);
    }
    Ok(out)
}
fn request_id(envelope: &McpEnvelope) -> String {
    match envelope {
        McpEnvelope::Request(r) => format!("{:?}", r.id),
        McpEnvelope::Notification(n) => format!("notify:{}", n.method),
        _ => "invalid".into(),
    }
}

fn check_projection(
    head: &RequestHead,
    envelope: &McpEnvelope,
) -> std::result::Result<(), &'static str> {
    let (method, params) = match envelope {
        McpEnvelope::Request(r) => (r.method.as_str(), &r.params),
        McpEnvelope::Notification(n) => (n.method.as_str(), &n.params),
        _ => return Err("request body must be a request or notification"),
    };
    if unique_header(head, "mcp-protocol-version").map_err(|_| "duplicate protocol header")?
        != Some(PROTOCOL)
    {
        return Err("protocol header/body mismatch");
    }
    if let Some(projected) =
        unique_header(head, "mcp-method").map_err(|_| "duplicate method header")?
        && projected != method
    {
        return Err("method header/body mismatch");
    }
    let body_name = expr_field(params, "name");
    if let Some(projected) = unique_header(head, "mcp-name").map_err(|_| "duplicate name header")?
        && body_name != Some(projected)
    {
        return Err("name header/body mismatch");
    }
    if let Some(body_version) = expr_field(params, "protocolVersion")
        && body_version != PROTOCOL
    {
        return Err("protocol header/body mismatch");
    }
    for (name, value) in &head.headers {
        if let Some(parameter) = name.to_ascii_lowercase().strip_prefix("mcp-parameter-")
            && expr_field(params, parameter) != Some(value.as_str())
        {
            return Err("parameter header/body mismatch");
        }
    }
    for (name, value) in &head.headers {
        if name.to_ascii_lowercase().starts_with("x-mcp-") && !safe_projection(value) {
            return Err("unsafe MCP sentinel header");
        }
    }
    Ok(())
}
fn expr_field<'a>(expr: &'a Expr, wanted: &str) -> Option<&'a str> {
    match sim_value::access::field_any(expr, wanted) {
        Some(Expr::String(value)) => Some(value),
        _ => None,
    }
}
fn safe_projection(value: &str) -> bool {
    value.strip_prefix(":base64:").map_or_else(
        || !value.is_empty() && value.bytes().all(|b| (0x21..=0x7e).contains(&b)),
        |v| {
            !v.is_empty()
                && v.len() % 4 != 1
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
        },
    )
}
fn common_headers(date: &str) -> Vec<(String, String)> {
    vec![
        ("Date".into(), date.into()),
        ("Cache-Control".into(), "no-store, private".into()),
        ("Vary".into(), "Origin, Accept, Authorization".into()),
        ("X-Content-Type-Options".into(), "nosniff".into()),
    ]
}
fn empty(
    response: &mut dyn ResponseWriter,
    scope: &RequestScope,
    status: u16,
    headers: Vec<(String, String)>,
) -> Result<()> {
    response
        .write_head(ResponseHead { status, headers }, scope)
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)?;
    response
        .finish(&[], scope)
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)
}
fn protocol_error(
    response: &mut dyn ResponseWriter,
    scope: &RequestScope,
    status: u16,
    message: &str,
    date: &str,
    codec: &Mutex<Cx>,
) -> Result<()> {
    let envelope = McpEnvelope::Error(McpErrorEnvelope {
        id: Expr::Nil,
        error: McpError {
            code: PARSE_ERROR,
            message: message.into(),
            data: Expr::Nil,
        },
    });
    let bytes = encode(codec, &envelope)?;
    let mut headers = common_headers(date);
    headers.push(("Content-Type".into(), JSON.into()));
    response
        .write_head(ResponseHead { status, headers }, scope)
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)?;
    response
        .write_chunk(&bytes, scope)
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)?;
    response
        .finish(&[], scope)
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)
}
fn write_sse(
    response: &mut dyn ResponseWriter,
    scope: &RequestScope,
    replies: &[McpEnvelope],
    clock: &dyn HttpClock,
    keepalive_cap: usize,
    codec: &Mutex<Cx>,
) -> Result<()> {
    let mut headers = common_headers(&clock.http_date());
    headers.extend([
        ("Content-Type".into(), SSE.into()),
        ("X-Accel-Buffering".into(), "no".into()),
        ("Content-Encoding".into(), "identity".into()),
    ]);
    response
        .write_head(
            ResponseHead {
                status: 200,
                headers,
            },
            scope,
        )
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)?;
    for (index, reply) in replies.iter().enumerate() {
        if index < keepalive_cap
            && let Some(tick) = clock.keepalive()
        {
            response
                .write_chunk(format!(": keepalive {tick}\n\n").as_bytes(), scope)
                .inspect_err(|_| scope.cancel_peer_drop())
                .map_err(Error::host_io)?;
        }
        let payload = encode(codec, reply)?;
        let mut frame = b"event: message\ndata: ".to_vec();
        frame.extend(payload);
        frame.extend(b"\n\n");
        response
            .write_chunk(&frame, scope)
            .inspect_err(|_| scope.cancel_peer_drop())
            .map_err(Error::host_io)?;
    }
    response
        .finish(&[], scope)
        .inspect_err(|_| scope.cancel_peer_drop())
        .map_err(Error::host_io)
}
fn decode(codec: &Mutex<Cx>, bytes: Vec<u8>) -> Result<McpEnvelope> {
    let mut codec = codec
        .lock()
        .map_err(|_| Error::PoisonedLock("MCP HTTP codec"))?;
    let expr = decode_with_codec(
        &mut codec,
        &Symbol::qualified("codec", "mcp"),
        Input::Bytes(bytes),
        ReadPolicy::default(),
    )?;
    expr_to_envelope(&expr)
}
fn encode(codec: &Mutex<Cx>, envelope: &McpEnvelope) -> Result<Vec<u8>> {
    let mut codec = codec
        .lock()
        .map_err(|_| Error::PoisonedLock("MCP HTTP codec"))?;
    encode_with_codec(
        &mut codec,
        &Symbol::qualified("codec", "mcp"),
        &envelope_to_expr(envelope),
        EncodeOptions::default(),
    )?
    .into_text()
    .map(String::into_bytes)
}
