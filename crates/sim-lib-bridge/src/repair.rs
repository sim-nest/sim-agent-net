use sim_kernel::{Expr, Symbol};
use sim_value::build::entry;

/// ASK failure data that can be fed back to a model as repair context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskFailure {
    /// The returned content could not be decoded by the declared codec.
    Decode {
        /// Declared return codec.
        codec: Symbol,
        /// Decode diagnostic.
        message: String,
    },
    /// The decoded value failed the declared return Shape.
    Shape {
        /// Expected shape descriptor.
        expected: String,
        /// Shape diagnostics.
        diagnostics: Vec<String>,
    },
}

impl AskFailure {
    /// Projects this failure into data suitable for fenced repair context.
    pub fn to_expr(&self) -> Expr {
        match self {
            Self::Decode { codec, message } => Expr::Map(vec![
                entry(
                    "kind",
                    Expr::Symbol(Symbol::qualified("bridge", "DecodeFailure")),
                ),
                entry("codec", Expr::Symbol(codec.clone())),
                entry("message", Expr::String(message.clone())),
            ]),
            Self::Shape {
                expected,
                diagnostics,
            } => Expr::Map(vec![
                entry(
                    "kind",
                    Expr::Symbol(Symbol::qualified("bridge", "ShapeFailure")),
                ),
                entry("expected", Expr::String(expected.clone())),
                entry(
                    "diagnostics",
                    Expr::Vector(diagnostics.iter().cloned().map(Expr::String).collect()),
                ),
            ]),
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::Decode { codec, message } => {
                format!("decode with {codec} failed: {message}")
            }
            Self::Shape {
                expected,
                diagnostics,
            } => {
                let actual = if diagnostics.is_empty() {
                    "no diagnostics".to_owned()
                } else {
                    diagnostics.join("; ")
                };
                format!("shape {expected} rejected answer: {actual}")
            }
        }
    }
}

/// Bounded ASK repair policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepairPolicy {
    /// Maximum retry count. Values above 2 are clamped.
    pub max_retries: u8,
}

impl RepairPolicy {
    /// Builds a repair policy, clamping retries to the BRIDGE maximum.
    pub fn new(max_retries: u8) -> Self {
        Self {
            max_retries: max_retries.min(2),
        }
    }

    pub(crate) fn retries(self) -> u8 {
        self.max_retries.min(2)
    }
}

impl Default for RepairPolicy {
    fn default() -> Self {
        Self::new(1)
    }
}
