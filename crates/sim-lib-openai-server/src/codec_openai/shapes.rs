use sim_codec_chat::validate_chat_transcript;
pub use sim_codec_chat::{OpenAiCodecOptions, OpenAiRequestOptions};
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
