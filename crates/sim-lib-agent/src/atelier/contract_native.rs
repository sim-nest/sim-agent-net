//! Contract-native Atelier backend descriptors.

use serde_json::{Value, json};
use sim_kernel::Symbol;

use super::{
    AgentMission, AtelierAction, GuardCapability, GuardDecision, guard_action,
    self_hosting::cassette_content_hash,
};

/// Stable JSON schema for contract-native Atelier cache evidence.
pub const CONTRACT_NATIVE_SCHEMA: &str = "sim.atelier.contract-native.v1";

/// Backend used by the Atelier shell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AtelierBackend {
    /// Current source-Radar authoring view.
    #[default]
    SourceRadar,
    /// Source-free FORGE contract authoring view.
    ContractNative,
}

impl AtelierBackend {
    /// Parses a backend label accepted by `simctl atelier --backend`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "source-radar" => Some(Self::SourceRadar),
            "contract-native" => Some(Self::ContractNative),
            _ => None,
        }
    }

    /// Returns the stable backend label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceRadar => "source-radar",
            Self::ContractNative => "contract-native",
        }
    }
}

/// Contract deck totals cached for the contract-native Atelier backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractNativeDeckSummary {
    /// Number of cards selected for the task-scoped deck.
    pub cards: u64,
    /// Number of cards that carry complete callable contract data.
    pub complete_cards: u64,
    /// Number of partial cards preserved with diagnostics.
    pub partial_cards: u64,
    /// Non-fatal deck diagnostics.
    pub diagnostics: Vec<String>,
}

/// Projection reductions and token accounting cached for the backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractNativeProjectionSummary {
    /// Prompt token budget used for projection.
    pub token_budget: u64,
    /// Estimated prompt tokens in the selected projection.
    pub tokens: u64,
    /// Cards retained in any representation.
    pub included: u64,
    /// Cards reduced to summary-only form.
    pub summary_only: u64,
    /// Cards dropped because they did not fit the budget.
    pub dropped: u64,
    /// Reduction diagnostics emitted during projection.
    pub diagnostics: Vec<String>,
}

/// Grammar metadata cached with the contract-native backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractNativeGrammarSummary {
    /// Grammar dialect used by the output contract.
    pub dialect: String,
    /// Target codec for terminal model output.
    pub target_codec: String,
    /// Return Shape expression used to derive the grammar.
    pub return_shape: String,
    /// Whether strict grammar output is required.
    pub strict: bool,
}

/// One route attempt cached from the deterministic authoring loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractNativeRouteAttempt {
    /// Route target id.
    pub target: String,
    /// Attempt status such as `failed` or `accepted`.
    pub status: String,
    /// Optional failure or skip reason.
    pub reason: Option<String>,
}

/// One guarded action denied for contract-native authoring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractNativeGuardDenial {
    /// Stable denial id used by cache consumers.
    pub id: String,
    /// Human-readable action label.
    pub action: String,
    /// Refusal reason from the Atelier guard.
    pub reason: String,
    /// Capability that would normally guard the action family.
    pub required_capability: String,
}

/// Deterministic cache report for the contract-native Atelier backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractNativeAtelierReport {
    /// Backend label.
    pub backend: AtelierBackend,
    /// Contract deck totals.
    pub deck: ContractNativeDeckSummary,
    /// Projection reductions and token totals.
    pub projection: ContractNativeProjectionSummary,
    /// Grammar metadata.
    pub grammar: ContractNativeGrammarSummary,
    /// Route attempts.
    pub route_attempts: Vec<ContractNativeRouteAttempt>,
    /// Non-fatal backend diagnostics.
    pub diagnostics: Vec<String>,
    /// Stable cassette hash for the deterministic evidence events.
    pub cassette_hash: String,
    /// Guard denials retained by the backend.
    pub guard_denials: Vec<ContractNativeGuardDenial>,
}

impl ContractNativeAtelierReport {
    /// Encodes the report as the public Atelier cache JSON shape.
    pub fn to_json(&self) -> Value {
        json!({
            "schema": CONTRACT_NATIVE_SCHEMA,
            "backend": self.backend.as_str(),
            "contract_deck": {
                "cards": self.deck.cards,
                "complete_cards": self.deck.complete_cards,
                "partial_cards": self.deck.partial_cards,
                "diagnostics": self.deck.diagnostics,
            },
            "projection": {
                "token_budget": self.projection.token_budget,
                "tokens": self.projection.tokens,
                "included": self.projection.included,
                "summary_only": self.projection.summary_only,
                "dropped": self.projection.dropped,
                "diagnostics": self.projection.diagnostics,
            },
            "grammar": {
                "dialect": self.grammar.dialect,
                "target_codec": self.grammar.target_codec,
                "return_shape": self.grammar.return_shape,
                "strict": self.grammar.strict,
            },
            "route_attempts": self.route_attempts.iter().map(|attempt| {
                json!({
                    "target": attempt.target,
                    "status": attempt.status,
                    "reason": attempt.reason,
                })
            }).collect::<Vec<_>>(),
            "diagnostics": self.diagnostics,
            "cassette_hash": self.cassette_hash,
            "guard_denials": self.guard_denials.iter().map(|denial| {
                json!({
                    "id": denial.id,
                    "action": denial.action,
                    "reason": denial.reason,
                    "required_capability": denial.required_capability,
                })
            }).collect::<Vec<_>>(),
        })
    }
}

/// Builds deterministic cache evidence for contract-native Atelier mode.
pub fn deterministic_contract_native_report() -> ContractNativeAtelierReport {
    let mission = AgentMission::new(
        Symbol::qualified("agent/mission", "contract-native-atelier"),
        "sim-agent-net",
    )
    .with_capability(GuardCapability::EditRepo("sim-agent-net".to_owned()));
    let events = [
        "contract-native:deck:task-scoped",
        "contract-native:projection:tokens=114",
        "contract-native:grammar:shapegrammar",
        "contract-native:route:cheap-downshift:failed",
        "contract-native:route:escalation-downshift:accepted",
        "contract-native:guard:denials-retained",
    ];
    ContractNativeAtelierReport {
        backend: AtelierBackend::ContractNative,
        deck: ContractNativeDeckSummary {
            cards: 4,
            complete_cards: 3,
            partial_cards: 1,
            diagnostics: vec![
                "missing authored example preserved with Shape-synthesized fallback".to_owned(),
            ],
        },
        projection: ContractNativeProjectionSummary {
            token_budget: 160,
            tokens: 114,
            included: 3,
            summary_only: 1,
            dropped: 1,
            diagnostics: vec![
                "contract projection reduced table/entries to summary only under token budget"
                    .to_owned(),
                "contract projection dropped unrelated export under token budget".to_owned(),
            ],
        },
        grammar: ContractNativeGrammarSummary {
            dialect: "shapegrammar".to_owned(),
            target_codec: "codec:lisp".to_owned(),
            return_shape: "(list-rest () Any)".to_owned(),
            strict: true,
        },
        route_attempts: vec![
            ContractNativeRouteAttempt {
                target: "cheap-downshift".to_owned(),
                status: "failed".to_owned(),
                reason: Some("terminal output failed codec or Shape check".to_owned()),
            },
            ContractNativeRouteAttempt {
                target: "escalation-downshift".to_owned(),
                status: "accepted".to_owned(),
                reason: None,
            },
        ],
        diagnostics: vec![
            "contract-native cache evidence is deterministic and source-free".to_owned(),
            "sim-web-shell serves this cache without issuing model requests".to_owned(),
        ],
        cassette_hash: cassette_content_hash(&events),
        guard_denials: contract_native_guard_denials(&mission),
    }
}

/// Evaluates the guard denials contract-native mode must retain by default.
pub fn contract_native_guard_denials(mission: &AgentMission) -> Vec<ContractNativeGuardDenial> {
    [
        (
            "meta-workspace-edit",
            AtelierAction::edit_file(
                mission.leased_repo(),
                ".meta-workspace/packages/sim-lib-agent/src/atelier.rs",
            ),
            "EditRepo(sim-agent-net)",
        ),
        (
            "cross-repo-write",
            AtelierAction::edit_file("sim-web", "crates/sim-web-shell/src/atelier.rs"),
            "EditRepo(sim-web)",
        ),
        (
            "github-outward-action",
            AtelierAction::AddGithubRemote {
                remote: "https://github.com/sim-nest/sim-private.git".to_owned(),
            },
            "PlanPin",
        ),
    ]
    .into_iter()
    .filter_map(|(id, action, required_capability)| {
        let decision = guard_action(mission, action);
        let GuardDecision::Refused(refusal) = decision else {
            return None;
        };
        Some(ContractNativeGuardDenial {
            id: id.to_owned(),
            action: refusal.action().label(),
            reason: refusal.reason().to_owned(),
            required_capability: required_capability.to_owned(),
        })
    })
    .collect()
}
