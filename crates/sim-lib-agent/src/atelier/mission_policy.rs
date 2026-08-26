//! Immutable mission law and replaceable crew topology.
//!
//! This module deliberately contains no execution loop. A [`MissionPlan`] is
//! frozen input to an existing conduct: it says who may propose or adopt work,
//! which effects require an external decision, and which vetoes no crew or
//! model placement may negotiate away.

use sim_cookbook::fnv1a64_hex;
use sim_kernel::{Expr, NumberLiteral, Symbol};
use std::collections::{BTreeMap, BTreeSet};

/// Increasing levels of mission authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthorityLevel {
    /// Inspect declared mission inputs without changing mission state.
    Observe,
    /// Organize observations into a local working view.
    Organize,
    /// Propose local alternatives without selecting one.
    ProposeLocal,
    /// Adopt a local result admitted by the frozen mission law.
    AdoptLocal,
    /// Request an external effect. The request is not the effect.
    RequestExternal,
    /// Request a sensitive effect. The request is not the effect.
    RequestSensitive,
}

impl AuthorityLevel {
    fn label(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Organize => "organize",
            Self::ProposeLocal => "propose-local",
            Self::AdoptLocal => "adopt-local",
            Self::RequestExternal => "request-external",
            Self::RequestSensitive => "request-sensitive",
        }
    }
}

/// A role named by the plan, with a non-widening authority ceiling.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MissionRole {
    /// Stable role id.
    pub id: Symbol,
    /// Greatest action the role may request.
    pub authority: AuthorityLevel,
}

impl MissionRole {
    /// Creates one role ceiling.
    pub fn new(id: Symbol, authority: AuthorityLevel) -> Self {
        Self { id, authority }
    }
}

/// A model or execution site assigned as immutable plan data.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Placement {
    /// Role placed at the site.
    pub role: Symbol,
    /// Opaque site id; it grants no authority.
    pub site: Symbol,
}

impl Placement {
    /// Creates a role placement.
    pub fn new(role: Symbol, site: Symbol) -> Self {
        Self { role, site }
    }
}

/// Per-role tool boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCeiling {
    /// Role constrained by the ceiling.
    pub role: Symbol,
    /// Exact tools the role may request.
    pub tools: BTreeSet<Symbol>,
}

impl ToolCeiling {
    /// Creates a ceiling from exact tool ids.
    pub fn new(role: Symbol, tools: impl IntoIterator<Item = Symbol>) -> Self {
        Self {
            role,
            tools: tools.into_iter().collect(),
        }
    }
}

/// Replaceable, data-only crew arrangements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrewTopology {
    /// One role observes, proposes, and acts within one ceiling.
    Solo,
    /// Speculators propose and an independent verifier checks.
    SpeculateVerify,
    /// Separate evidence, proposal, and decision tables.
    ThreeTable,
    /// A jury proposes findings and a separately named judge adopts.
    JudgeJury,
    /// Placement candidates compete while mission law remains fixed.
    PlacementMarket,
}

impl CrewTopology {
    fn label(self) -> &'static str {
        match self {
            Self::Solo => "solo",
            Self::SpeculateVerify => "speculate-verify",
            Self::ThreeTable => "three-table",
            Self::JudgeJury => "judge-jury",
            Self::PlacementMarket => "placement-market",
        }
    }
}

/// Non-overridable reasons to refuse a mission request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MissionVeto {
    /// Deterministic verification did not pass.
    DeterministicVerification,
    /// A declared source check did not pass.
    SourceCheck,
    /// The caller lacks the requested capability.
    CapabilityRefusal,
    /// The request crosses the mission privacy fence.
    PrivacyFence,
    /// The mission budget is exhausted.
    BudgetExhausted,
    /// The role is absent or asks beyond its frozen ceiling.
    SelfWideningRole,
    /// The adopting judge selected itself rather than being named by the plan.
    SelfSelectedJudge,
    /// The request reads an undeclared adjacent room.
    HiddenAdjacentRoomRead,
    /// The request's pack closure differs from the plan-bound closure.
    PackClosureMismatch,
    /// The requested tool is outside the role's exact ceiling.
    ToolCeiling,
    /// The mission stop condition is already true.
    Stop,
}

/// Facts submitted to the pure mission admission boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionRequest {
    /// Calling role.
    pub role: Symbol,
    /// Requested authority.
    pub authority: AuthorityLevel,
    /// Optional tool requested by the role.
    pub tool: Option<Symbol>,
    /// Closure identity supplied by the caller.
    pub pack_closure: String,
    /// Judge selected by the proposal, if any.
    pub selected_judge: Option<Symbol>,
    /// Whether a read targets an undeclared adjacent room.
    pub hidden_adjacent_room_read: bool,
    /// Mandatory verification fact.
    pub deterministic_verification: bool,
    /// Mandatory source-check fact.
    pub source_checks: bool,
    /// Mandatory capability fact.
    pub capability_granted: bool,
    /// Mandatory privacy fact.
    pub privacy_fence_clear: bool,
    /// Budget units requested.
    pub budget_requested: u64,
}

/// Deterministic result of applying frozen mission law.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MissionAdmission {
    /// The local action is admitted.
    AdmitLocal,
    /// An external authority must decide; no effect has occurred.
    Escalate {
        /// Whether the request carries sensitive data or authority.
        sensitive: bool,
    },
    /// A non-overridable veto refused the request.
    Refuse(MissionVeto),
}

/// Construction failures for an immutable mission plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MissionPlanError {
    /// A role id was repeated.
    DuplicateRole(Symbol),
    /// A placement or tool ceiling names no declared role.
    UnknownRole(Symbol),
    /// The judge is not independent of the jury.
    JudgeInJury(Symbol),
    /// The pack closure is empty.
    EmptyPackClosure,
}

/// Content-identified mission law, composition, limits, and stop condition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionPlan {
    content_id: String,
    pack_closure: String,
    roles: BTreeMap<Symbol, AuthorityLevel>,
    placements: Vec<Placement>,
    tool_ceilings: BTreeMap<Symbol, BTreeSet<Symbol>>,
    budget: u64,
    escalation: Symbol,
    stop: bool,
    topology: CrewTopology,
    judge: Option<Symbol>,
    jury: BTreeSet<Symbol>,
}

impl MissionPlan {
    /// Freezes all mission inputs and derives their stable content id.
    #[allow(clippy::too_many_arguments)]
    pub fn freeze(
        pack_closure: impl Into<String>,
        roles: impl IntoIterator<Item = MissionRole>,
        placements: impl IntoIterator<Item = Placement>,
        tool_ceilings: impl IntoIterator<Item = ToolCeiling>,
        budget: u64,
        escalation: Symbol,
        stop: bool,
        topology: CrewTopology,
        judge: Option<Symbol>,
        jury: impl IntoIterator<Item = Symbol>,
    ) -> Result<Self, MissionPlanError> {
        let pack_closure = pack_closure.into();
        if pack_closure.is_empty() {
            return Err(MissionPlanError::EmptyPackClosure);
        }
        let mut role_map = BTreeMap::new();
        for role in roles {
            if role_map.insert(role.id.clone(), role.authority).is_some() {
                return Err(MissionPlanError::DuplicateRole(role.id));
            }
        }
        let placements: Vec<_> = placements.into_iter().collect();
        for placement in &placements {
            if !role_map.contains_key(&placement.role) {
                return Err(MissionPlanError::UnknownRole(placement.role.clone()));
            }
        }
        let mut ceilings = BTreeMap::new();
        for ceiling in tool_ceilings {
            if !role_map.contains_key(&ceiling.role) {
                return Err(MissionPlanError::UnknownRole(ceiling.role));
            }
            ceilings.insert(ceiling.role, ceiling.tools);
        }
        let jury: BTreeSet<_> = jury.into_iter().collect();
        if let Some(judge) = &judge {
            if !role_map.contains_key(judge) {
                return Err(MissionPlanError::UnknownRole(judge.clone()));
            }
            if jury.contains(judge) {
                return Err(MissionPlanError::JudgeInJury(judge.clone()));
            }
        }
        for juror in &jury {
            if !role_map.contains_key(juror) {
                return Err(MissionPlanError::UnknownRole(juror.clone()));
            }
        }
        let mut plan = Self {
            content_id: String::new(),
            pack_closure,
            roles: role_map,
            placements,
            tool_ceilings: ceilings,
            budget,
            escalation,
            stop,
            topology,
            judge,
            jury,
        };
        plan.content_id = format!("fnv1a64:{}", fnv1a64_hex(plan.canonical().as_bytes()));
        Ok(plan)
    }

    /// Stable identity over every law and topology input.
    pub fn content_id(&self) -> &str {
        &self.content_id
    }

    /// Bound pack-closure identity.
    pub fn pack_closure(&self) -> &str {
        &self.pack_closure
    }

    /// Frozen topology data.
    pub fn topology(&self) -> CrewTopology {
        self.topology
    }

    /// Frozen placements. Sites never grant authority.
    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    /// Applies mission law without consulting a model or topology runtime.
    pub fn admit(&self, request: &MissionRequest) -> MissionAdmission {
        if self.stop {
            return MissionAdmission::Refuse(MissionVeto::Stop);
        }
        if !request.deterministic_verification {
            return MissionAdmission::Refuse(MissionVeto::DeterministicVerification);
        }
        if !request.source_checks {
            return MissionAdmission::Refuse(MissionVeto::SourceCheck);
        }
        if !request.capability_granted {
            return MissionAdmission::Refuse(MissionVeto::CapabilityRefusal);
        }
        if !request.privacy_fence_clear {
            return MissionAdmission::Refuse(MissionVeto::PrivacyFence);
        }
        if request.budget_requested > self.budget {
            return MissionAdmission::Refuse(MissionVeto::BudgetExhausted);
        }
        if request.hidden_adjacent_room_read {
            return MissionAdmission::Refuse(MissionVeto::HiddenAdjacentRoomRead);
        }
        if request.pack_closure != self.pack_closure {
            return MissionAdmission::Refuse(MissionVeto::PackClosureMismatch);
        }
        let Some(ceiling) = self.roles.get(&request.role) else {
            return MissionAdmission::Refuse(MissionVeto::SelfWideningRole);
        };
        if request.authority > *ceiling {
            return MissionAdmission::Refuse(MissionVeto::SelfWideningRole);
        }
        if let Some(tool) = &request.tool
            && !self
                .tool_ceilings
                .get(&request.role)
                .is_some_and(|tools| tools.contains(tool))
        {
            return MissionAdmission::Refuse(MissionVeto::ToolCeiling);
        }
        if request.authority == AuthorityLevel::AdoptLocal
            && self.topology == CrewTopology::JudgeJury
            && (request.selected_judge.as_ref() != self.judge.as_ref()
                || self.jury.contains(&request.role))
        {
            return MissionAdmission::Refuse(MissionVeto::SelfSelectedJudge);
        }
        match request.authority {
            AuthorityLevel::RequestExternal => MissionAdmission::Escalate { sensitive: false },
            AuthorityLevel::RequestSensitive => MissionAdmission::Escalate { sensitive: true },
            _ => MissionAdmission::AdmitLocal,
        }
    }

    /// Ordinary SIM data for codec and Shape-facing boundaries.
    pub fn as_expr(&self) -> Expr {
        Expr::Map(vec![
            pair("kind", Expr::Symbol(Symbol::new("mission-plan"))),
            pair("content-id", Expr::String(self.content_id.clone())),
            pair("pack-closure", Expr::String(self.pack_closure.clone())),
            pair("topology", Expr::Symbol(Symbol::new(self.topology.label()))),
            pair(
                "budget",
                Expr::Number(NumberLiteral {
                    domain: Symbol::qualified("citizen", "int"),
                    canonical: self.budget.to_string(),
                }),
            ),
            pair("escalation", Expr::Symbol(self.escalation.clone())),
            pair("stop", Expr::Bool(self.stop)),
            pair(
                "roles",
                Expr::List(
                    self.roles
                        .iter()
                        .map(|(id, authority)| {
                            Expr::List(vec![
                                Expr::Symbol(id.clone()),
                                Expr::Symbol(Symbol::new(authority.label())),
                            ])
                        })
                        .collect(),
                ),
            ),
            pair(
                "placements",
                Expr::List(
                    self.placements
                        .iter()
                        .map(|p| {
                            Expr::List(vec![
                                Expr::Symbol(p.role.clone()),
                                Expr::Symbol(p.site.clone()),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }

    fn canonical(&self) -> String {
        format!(
            "pack={:?};roles={:?};placements={:?};tools={:?};budget={};escalation={};stop={};topology={};judge={:?};jury={:?}",
            self.pack_closure,
            self.roles,
            self.placements,
            self.tool_ceilings,
            self.budget,
            self.escalation,
            self.stop,
            self.topology.label(),
            self.judge,
            self.jury
        )
    }
}

fn pair(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}
