use sim_citizen_derive::Citizen;
use sim_kernel::{Expr, Symbol};

/// Metadata describing one runner/model candidate for routing and inspection.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "agent-runner/ModelCard", version = 1)]
pub struct ModelCard {
    /// Registered runner symbol.
    pub runner: Symbol,
    /// Model name advertised by the runner.
    pub model: String,
    /// Provider identity, such as `openai-compatible` or `process`.
    pub provider: Symbol,
    /// Locality hint such as local, remote, or agent-backed.
    pub locality: Symbol,
    /// Open extension fields preserved for policy and browse surfaces.
    pub extra: Vec<(Expr, Expr)>,
}

impl ModelCard {
    /// Constructs a card with explicit runner, provider, and locality metadata.
    pub fn new(
        runner: Symbol,
        model: impl Into<String>,
        provider: Symbol,
        locality: Symbol,
    ) -> Self {
        Self {
            runner,
            model: model.into(),
            provider,
            locality,
            extra: Vec::new(),
        }
    }
}

impl Default for ModelCard {
    fn default() -> Self {
        Self::new(
            Symbol::qualified("runner", "fixture"),
            "fixture-model",
            Symbol::new("fixture-provider"),
            Symbol::new("local"),
        )
    }
}

/// One availability/score proposal produced for market-style runner selection.
#[derive(Clone, Debug, PartialEq, Default, Citizen)]
#[citizen(symbol = "agent-runner/ModelBid", version = 1)]
pub struct ModelBid {
    /// Whether the runner can currently satisfy the request.
    pub available: bool,
    /// Optional explanation when unavailable or for diagnostics.
    pub reason: Option<String>,
    /// Preference score when the runner is available.
    pub score: Option<f64>,
    /// Optional model override or chosen concrete model name.
    pub model: Option<String>,
    /// Open extension fields preserved for policy-specific metadata.
    pub extra: Vec<(Expr, Expr)>,
}

impl ModelBid {
    /// Constructs an unavailable bid with an explanatory reason.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            reason: Some(reason.into()),
            score: None,
            model: None,
            extra: Vec::new(),
        }
    }
}

/// Returns the citizen class symbol for [`ModelCard`].
pub fn model_card_class_symbol() -> Symbol {
    Symbol::qualified("agent-runner", "ModelCard")
}

/// Returns the citizen class symbol for [`ModelBid`].
pub fn model_bid_class_symbol() -> Symbol {
    Symbol::qualified("agent-runner", "ModelBid")
}
