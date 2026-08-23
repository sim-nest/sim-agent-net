//! Compatibility adapter for initialize-era MCP connections.
//!
//! This crate deliberately depends on the stateless [`sim_lib_mcp`] service;
//! the modern crate never depends on this adapter.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use sim_codec_mcp::{McpEnvelope, McpNotification, McpRequest, McpResponse};
use sim_kernel::{Cx, Error, Expr, Result};
use sim_lib_mcp::{CachePolicy, McpService, NegotiatedExtensions, Principal, RequestContext};

/// Cookbook recipes for the compatibility adapter.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

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
        Expr::Keyword(value) => Some(value.as_str().to_owned()),
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
    use sim_kernel::Keyword;
    let field = |name: &str, value| (Expr::Keyword(Keyword::new(name)), value);
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

#[cfg(test)]
mod tests {
    use super::*;
    use sim_kernel::Keyword;
    use sim_lib_mcp::{McpProfile, ServerDescription};

    fn request(id: &str, method: &str, params: Expr) -> McpRequest {
        McpRequest {
            id: Expr::String(id.into()),
            method: method.into(),
            params,
        }
    }

    #[test]
    fn delivered_2025_03_26_lifecycle_vector_is_preserved() {
        let service = McpService::new(ServerDescription::new(
            "sim",
            env!("CARGO_PKG_VERSION"),
            McpProfile::all(),
        ));
        let mut adapter = LegacyConnection::new(service, "legacy", Principal::new("client"));
        let mut cx = Cx::new();
        let replies = adapter
            .handle(&mut cx, request("1", "initialize", Expr::Nil))
            .unwrap();
        assert!(matches!(&replies[0], McpEnvelope::Response(_)));
        adapter
            .handle(
                &mut cx,
                request("2", "notifications/initialized", Expr::Nil),
            )
            .unwrap();
        assert!(adapter.initialized());
        adapter
            .handle(&mut cx, request("3", "ping", Expr::Nil))
            .unwrap();
        adapter
            .handle(&mut cx, request("4", "shutdown", Expr::Nil))
            .unwrap();
        assert!(adapter.shutdown_requested());
    }

    #[test]
    fn delivered_2025_03_26_stateless_vectors_use_the_identical_service_path() {
        for method in ["ping", "resources/list", "prompts/list", "tools/list"] {
            let direct = McpService::new(ServerDescription::new(
                "sim",
                env!("CARGO_PKG_VERSION"),
                McpProfile::all(),
            ));
            let context = RequestContext::new(
                "direct",
                "2025-03-26",
                NegotiatedExtensions::none(),
                Principal::new("client"),
                CachePolicy::Bypass,
            );
            let expected: Vec<_> = direct
                .handle(
                    &mut Cx::new(),
                    &context,
                    request("vector", method, Expr::Nil),
                )
                .unwrap()
                .collect();

            let adapted = McpService::new(ServerDescription::new(
                "sim",
                env!("CARGO_PKG_VERSION"),
                McpProfile::all(),
            ));
            let mut connection = LegacyConnection::new(adapted, "legacy", Principal::new("client"));
            let actual = connection
                .handle(&mut Cx::new(), request("vector", method, Expr::Nil))
                .unwrap();
            assert_eq!(actual, expected, "legacy vector diverged for {method}");
        }
    }

    #[test]
    fn initialized_notification_is_connection_state_and_has_no_reply() {
        let service = McpService::new(ServerDescription::new(
            "sim",
            env!("CARGO_PKG_VERSION"),
            McpProfile::all(),
        ));
        let mut adapter = LegacyConnection::new(service, "legacy", Principal::new("client"));
        let replies = adapter
            .handle_envelope(
                &mut Cx::new(),
                McpEnvelope::Notification(McpNotification {
                    method: "notifications/initialized".to_owned(),
                    params: Expr::Nil,
                }),
            )
            .unwrap();
        assert!(replies.is_empty());
        assert!(adapter.initialized());
    }

    #[test]
    fn invalid_initialize_params_keep_the_delivered_type_error() {
        let service = McpService::new(ServerDescription::new(
            "sim",
            env!("CARGO_PKG_VERSION"),
            McpProfile::all(),
        ));
        let mut adapter = LegacyConnection::new(service, "legacy", Principal::new("client"));
        assert!(matches!(
            adapter.handle(&mut Cx::new(), request("1", "initialize", Expr::Bool(true))),
            Err(Error::TypeMismatch { .. })
        ));
    }

    #[test]
    fn initialize_retains_explicit_version_extension_and_grant_facts() {
        let service = McpService::new(ServerDescription::new(
            "sim",
            env!("CARGO_PKG_VERSION"),
            McpProfile::all(),
        ));
        let mut adapter = LegacyConnection::new(service, "legacy", Principal::new("client"));
        let params = Expr::Map(vec![
            (
                Expr::Keyword(Keyword::new("protocolVersion")),
                Expr::String("2025-03-26".to_owned()),
            ),
            (
                Expr::Keyword(Keyword::new("capabilities")),
                Expr::Map(vec![(
                    Expr::Keyword(Keyword::new("sampling")),
                    Expr::Map(Vec::new()),
                )]),
            ),
            (
                Expr::Keyword(Keyword::new("grants")),
                Expr::Vector(vec![Expr::String("tools.call".to_owned())]),
            ),
        ]);
        adapter
            .handle(&mut Cx::new(), request("1", "initialize", params))
            .unwrap();
        assert_eq!(adapter.protocol_version(), "2025-03-26");
        assert!(adapter.negotiated_extensions().contains("sampling"));
        assert_eq!(adapter.requested_grants(), ["tools.call"]);
    }
}
