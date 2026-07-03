use serde_json::json;
use sim_kernel::Error;

use crate::objects::GatewayResponse;

#[derive(Clone, Debug)]
pub(crate) struct OpenAiRouteError {
    status: u16,
    message: String,
    error_type: &'static str,
    param: Option<&'static str>,
    code: &'static str,
}

impl OpenAiRouteError {
    pub(crate) fn bad_request(
        message: impl Into<String>,
        param: Option<&'static str>,
        code: &'static str,
    ) -> Self {
        Self {
            status: 400,
            message: message.into(),
            error_type: "invalid_request_error",
            param,
            code,
        }
    }

    pub(crate) fn invalid_json(message: impl Into<String>) -> Self {
        Self::bad_request(message, None, "invalid_json")
    }

    pub(crate) fn missing_required(param: &'static str) -> Self {
        Self::bad_request(
            format!("missing required parameter: {param}"),
            Some(param),
            "missing_required_parameter",
        )
    }

    pub(crate) fn bad_request_from_error(error: Error) -> Self {
        Self::bad_request(error.to_string(), None, "invalid_request")
    }

    pub(crate) fn bad_model_from_error(error: Error) -> Self {
        Self::bad_request(error.to_string(), Some("model"), "invalid_model")
    }

    pub(crate) fn model(error: Error, model: &str) -> Self {
        let message = error.to_string();
        if message.contains("model_not_found") || message.contains("only supports fixture/echo") {
            Self {
                status: 404,
                message: format!("model_not_found: {model}"),
                error_type: "invalid_request_error",
                param: Some("model"),
                code: "model_not_found",
            }
        } else {
            Self::bad_request(message, Some("model"), "invalid_model")
        }
    }

    pub(crate) fn internal(error: Error) -> Self {
        Self::internal_message(error.to_string())
    }

    pub(crate) fn internal_message(message: impl Into<String>) -> Self {
        Self {
            status: 500,
            message: message.into(),
            error_type: "server_error",
            param: None,
            code: "internal_error",
        }
    }

    pub(crate) fn forbidden(message: impl Into<String>, code: &'static str) -> Self {
        Self {
            status: 403,
            message: message.into(),
            error_type: "permission_error",
            param: None,
            code,
        }
    }

    pub(crate) fn not_found(response_id: &str) -> Self {
        Self::not_found_kind("response", response_id)
    }

    pub(crate) fn not_found_kind(kind: &'static str, id: &str) -> Self {
        Self {
            status: 404,
            message: format!("{kind} not found: {id}"),
            error_type: "invalid_request_error",
            param: Some("id"),
            code: "not_found",
        }
    }

    pub(crate) fn into_response(self) -> GatewayResponse {
        GatewayResponse::json(
            self.status,
            json!({
                "error": {
                    "message": self.message,
                    "type": self.error_type,
                    "param": self.param,
                    "code": self.code,
                }
            })
            .to_string()
            .into_bytes(),
        )
    }
}
