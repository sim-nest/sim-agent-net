/// Explicit connection state retained only for legacy clients.
pub struct LegacyConnection {
    service: McpService,
    connection_id: String,
    protocol_version: String,
    client_info: Option<Expr>,
    extensions: NegotiatedExtensions,
    requested_grants: Vec<String>,
    principal: Principal,
    initialized: bool,
    shutdown_requested: bool,
    requests_seen: u64,
}

impl LegacyConnection {
    /// Creates an uninitialized legacy connection around the modern service.
    pub fn new(
        service: McpService,
        connection_id: impl Into<String>,
        principal: Principal,
    ) -> Self {
        Self {
            service,
            connection_id: connection_id.into(),
            protocol_version: "2025-03-26".to_owned(),
            client_info: None,
            extensions: NegotiatedExtensions::none(),
            requested_grants: Vec::new(),
            principal,
            initialized: false,
            shutdown_requested: false,
            requests_seen: 0,
        }
    }

    /// Reports whether the initialized notification was received.
    pub fn initialized(&self) -> bool {
        self.initialized
    }
    /// Reports whether shutdown was requested.
    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }
    /// Client metadata retained from initialize.
    pub fn client_info(&self) -> Option<&Expr> {
        self.client_info.as_ref()
    }
    /// Protocol version retained from initialize negotiation.
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }
    /// Extension identifiers retained from the client's capabilities map.
    pub fn negotiated_extensions(&self) -> &NegotiatedExtensions {
        &self.extensions
    }
    /// Legacy grant names requested during initialization.
    pub fn requested_grants(&self) -> &[String] {
        &self.requested_grants
    }

    /// Handles one legacy request, constructing a complete modern context for ordinary methods.
    pub fn handle(&mut self, cx: &mut Cx, request: McpRequest) -> Result<Vec<McpEnvelope>> {
        match request.method.as_str() {
            "initialize" => {
                self.capture_initialize(&request.params)?;
                Ok(vec![McpEnvelope::Response(McpResponse {
                    id: request.id,
                    result: initialize_result(
                        &self.protocol_version,
                        self.service.description().name(),
                        self.service.description().version(),
                    ),
                })])
            }
            "initialized" | "notifications/initialized" => {
                self.initialized = true;
                Ok(vec![McpEnvelope::Response(McpResponse {
                    id: request.id,
                    result: Expr::Map(Vec::new()),
                })])
            }
            "shutdown" => {
                self.shutdown_requested = true;
                Ok(vec![McpEnvelope::Response(McpResponse {
                    id: request.id,
                    result: Expr::Map(Vec::new()),
                })])
            }
            _ => {
                self.requests_seen += 1;
                let context = RequestContext::new(
                    format!(
                        "{}:{}:{}",
                        self.connection_id,
                        self.requests_seen,
                        request_key(&request.id)
                    ),
                    self.protocol_version.clone(),
                    self.extensions.clone(),
                    self.principal.clone(),
                    CachePolicy::Bypass,
                );
                self.service
                    .handle(cx, &context, request)
                    .map(Iterator::collect)
            }
        }
    }

    /// Handles one decoded legacy envelope.
    ///
    /// Responses and errors are ignored because this adapter is a server-side
    /// compatibility boundary. Notifications update only explicit connection
    /// state and correctly produce no response envelope.
    pub fn handle_envelope(
        &mut self,
        cx: &mut Cx,
        envelope: McpEnvelope,
    ) -> Result<Vec<McpEnvelope>> {
        match envelope {
            McpEnvelope::Request(request) => self.handle(cx, request),
            McpEnvelope::Notification(notification) => {
                self.handle_notification(notification);
                Ok(Vec::new())
            }
            McpEnvelope::Response(_) | McpEnvelope::Error(_) => Ok(Vec::new()),
        }
    }

    fn handle_notification(&mut self, notification: McpNotification) {
        match notification.method.as_str() {
            "initialized" | "notifications/initialized" => self.initialized = true,
            "shutdown" => self.shutdown_requested = true,
            _ => {}
        }
    }

    fn capture_initialize(&mut self, params: &Expr) -> Result<()> {
        if matches!(params, Expr::Nil) {
            return Ok(());
        }
        let Expr::Map(fields) = params else {
            return Err(Error::TypeMismatch {
                expected: "initialize params map or nil",
                found: "invalid initialize params",
            });
        };
        for (key, value) in fields {
            let Some(key) = map_key(key) else {
                continue;
            };
            match key.as_str() {
                "protocolVersion" | "protocol-version" => {
                    if let Expr::String(version) = value {
                        self.protocol_version = version.clone();
                    }
                }
                "clientInfo" | "client-info" => self.client_info = Some(value.clone()),
                "capabilities" => {
                    if let Expr::Map(capabilities) = value {
                        for (name, _) in capabilities {
                            if let Some(name) = map_key(name) {
                                self.extensions = self.extensions.clone().with(name);
                            }
                        }
                    }
                }
                "grants" => {
                    let values = match value {
                        Expr::List(values) | Expr::Vector(values) => values.as_slice(),
                        _ => &[],
                    };
                    self.requested_grants = values
                        .iter()
                        .filter_map(|value| match value {
                            Expr::String(value) => Some(value.clone()),
                            Expr::Symbol(value) => Some(value.to_string()),
                            _ => None,
                        })
                        .collect();
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn map_key(key: &Expr) -> Option<String> {
    match key {
        Expr::String(value) => Some(value.clone()),
        Expr::Symbol(value) => Some(value.to_string()),
        _ => None,
    }
}

fn request_key(id: &Expr) -> String {
    match id {
        Expr::String(value) => value.clone(),
        Expr::Number(number) => number.canonical.clone(),
        Expr::Nil => "nil".to_owned(),
        _ => format!("{id:?}"),
    }
}

fn initialize_result(protocol_version: &str, name: &str, version: &str) -> Expr {
    let field = |name: &str, value| (Expr::Symbol(Symbol::new(name)), value);
    Expr::Map(vec![
        field("protocolVersion", Expr::String(protocol_version.to_owned())),
        field(
            "serverInfo",
            Expr::Map(vec![
                field("name", Expr::String(name.to_owned())),
                field("version", Expr::String(version.to_owned())),
            ]),
        ),
        field(
            "capabilities",
            Expr::Map(vec![
                field("tools", Expr::Map(Vec::new())),
                field("resources", Expr::Map(Vec::new())),
                field("prompts", Expr::Map(Vec::new())),
            ]),
        ),
    ])
}
