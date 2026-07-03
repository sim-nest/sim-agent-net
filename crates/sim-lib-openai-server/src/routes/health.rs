use crate::server::{GatewayRequest, GatewayResponse, GatewayRouteState};

/// Route path for the health check endpoint (`GET /health`).
pub const HEALTH_PATH: &str = "/health";
/// Static JSON body returned by the health endpoint.
pub const HEALTH_BODY: &str =
    r#"{"status":"ok","service":"sim-openai-gateway","version":"0","backend":"sim"}"#;

/// Handles `GET /health`, returning the static health response.
pub fn handle_health(_request: &GatewayRequest, _state: &GatewayRouteState) -> GatewayResponse {
    health_response()
}

/// Returns the static `200 OK` health response with [`HEALTH_BODY`].
pub fn health_response() -> GatewayResponse {
    GatewayResponse::json(200, HEALTH_BODY.as_bytes().to_vec())
}
