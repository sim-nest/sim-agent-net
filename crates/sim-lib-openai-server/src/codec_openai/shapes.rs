use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Expr, Result};

/// Validated chat transcript expression accepted by `codec:openai`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatTranscript {
    expr: Expr,
}

impl ChatTranscript {
    /// Validates `expr` as a chat transcript and wraps it.
    pub fn new(expr: Expr) -> Result<Self> {
        validate_chat_transcript(&expr)?;
        Ok(Self { expr })
    }

    /// Returns the wrapped transcript expression.
    pub fn expr(&self) -> &Expr {
        &self.expr
    }

    /// Consumes the wrapper and returns the transcript expression.
    pub fn into_expr(self) -> Expr {
        self.expr
    }
}

/// Options for OpenAI-compatible provider request JSON generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenAiCodecOptions {
    /// Model identifier to place in the generated request.
    pub model: String,
    /// Whether to request a streamed (SSE) response.
    pub stream: bool,
    /// Whether to include the `tools` field in the generated request.
    pub tools: bool,
}

impl OpenAiCodecOptions {
    /// Builds codec options from a model id and the stream/tools flags.
    pub fn new(model: impl Into<String>, stream: bool, tools: bool) -> Self {
        Self {
            model: model.into(),
            stream,
            tools,
        }
    }
}

/// Alias for [`OpenAiCodecOptions`] when used as request-generation options.
pub type OpenAiRequestOptions = OpenAiCodecOptions;
