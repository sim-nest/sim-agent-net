//! Agent missions and recipe-pattern descriptors.

use super::{
    guard::GuardCapability,
    mission_lease::{WorkspaceLease, leases_expr},
};
use crate::{AgentPattern, AgentPatternSlot};
use sim_kernel::{Expr, Symbol};

/// Structured unit of Atelier agent work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentMission {
    id: Symbol,
    goal: String,
    scope: MissionScope,
    allowed_repos: Vec<String>,
    denied_paths: Vec<String>,
    code_free_repos: Vec<String>,
    capabilities: Vec<GuardCapability>,
    leases: Vec<WorkspaceLease>,
    validations: Vec<MissionRun>,
    docs_runs: Vec<MissionRun>,
    decision_points: Vec<HumanDecisionPoint>,
    recipe_pattern: Symbol,
    roles: Vec<AtelierAgentRole>,
    evidence_stream: Symbol,
}

impl AgentMission {
    /// Builds a mission scoped to one leased repository.
    pub fn new(id: Symbol, leased_repo: impl Into<String>) -> Self {
        let leased_repo = leased_repo.into();
        Self {
            id: id.clone(),
            goal: "guarded Atelier mission".to_owned(),
            scope: MissionScope::repo(leased_repo.clone()),
            allowed_repos: vec![leased_repo.clone()],
            denied_paths: vec![".meta-workspace/".to_owned()],
            code_free_repos: Vec::new(),
            capabilities: Vec::new(),
            leases: Vec::new(),
            validations: Vec::new(),
            docs_runs: Vec::new(),
            decision_points: Vec::new(),
            recipe_pattern: Symbol::new("a30-013-code-generation"),
            roles: default_roles(),
            evidence_stream: Symbol::qualified("atelier/dev", id.name.as_ref()),
        }
    }

    /// Adds a granted guard capability.
    pub fn with_capability(mut self, capability: GuardCapability) -> Self {
        push_unique(&mut self.capabilities, capability);
        self
    }

    /// Sets the human-readable mission goal.
    pub fn with_goal(mut self, goal: impl Into<String>) -> Self {
        self.goal = goal.into();
        self
    }

    /// Sets a concise scope summary while preserving the primary repository.
    pub fn with_scope_summary(mut self, summary: impl Into<String>) -> Self {
        self.scope.summary = summary.into();
        self
    }

    /// Adds an allowed repository to the mission scope.
    pub fn with_allowed_repo(mut self, repo: impl Into<String>) -> Self {
        push_unique(&mut self.allowed_repos, repo.into());
        self
    }

    /// Adds a denied path prefix such as `.meta-workspace/`.
    pub fn with_denied_path(mut self, path: impl Into<String>) -> Self {
        push_unique(&mut self.denied_paths, path.into());
        self
    }

    /// Adds a repository that must stay Rust-code-free.
    pub fn with_code_free_repo(mut self, repo: impl Into<String>) -> Self {
        push_unique(&mut self.code_free_repos, repo.into());
        self
    }

    /// Adds a workspace lease.
    pub fn with_lease(mut self, lease: WorkspaceLease) -> Self {
        self.leases.push(lease);
        self
    }

    /// Adds a required validation command.
    pub fn with_validation(mut self, run: MissionRun) -> Self {
        self.validations.push(run);
        self
    }

    /// Adds a required docs command.
    pub fn with_docs_run(mut self, run: MissionRun) -> Self {
        self.docs_runs.push(run);
        self
    }

    /// Adds a human decision point.
    pub fn with_decision_point(mut self, point: HumanDecisionPoint) -> Self {
        self.decision_points.push(point);
        self
    }

    /// Selects the 30-agent recipe pattern that frames this mission.
    pub fn with_recipe_pattern(mut self, recipe_pattern: Symbol) -> Self {
        self.recipe_pattern = recipe_pattern;
        self
    }

    /// Sets the deterministic Dev Cassette stream id.
    pub fn with_evidence_stream(mut self, evidence_stream: Symbol) -> Self {
        self.evidence_stream = evidence_stream;
        self
    }

    /// Returns the mission id.
    pub fn id(&self) -> &Symbol {
        &self.id
    }

    /// Returns the human-readable mission goal.
    pub fn goal(&self) -> &str {
        &self.goal
    }

    /// Returns the mission scope.
    pub fn scope(&self) -> &MissionScope {
        &self.scope
    }

    /// Returns the primary leased repository.
    pub fn leased_repo(&self) -> &str {
        &self.scope.primary_repo
    }

    /// Returns the repository allow-list.
    pub fn allowed_repos(&self) -> &[String] {
        &self.allowed_repos
    }

    /// Returns the denied path prefixes.
    pub fn denied_paths(&self) -> &[String] {
        &self.denied_paths
    }

    /// Returns the repositories that must stay Rust-code-free.
    pub fn code_free_repos(&self) -> &[String] {
        &self.code_free_repos
    }

    /// Returns the granted guard capabilities.
    pub fn capabilities(&self) -> &[GuardCapability] {
        &self.capabilities
    }

    /// Returns the workspace leases.
    pub fn leases(&self) -> &[WorkspaceLease] {
        &self.leases
    }

    /// Returns the required validation commands.
    pub fn validations(&self) -> &[MissionRun] {
        &self.validations
    }

    /// Returns the required docs commands.
    pub fn docs_runs(&self) -> &[MissionRun] {
        &self.docs_runs
    }

    /// Returns the declared human decision points.
    pub fn decision_points(&self) -> &[HumanDecisionPoint] {
        &self.decision_points
    }

    /// Returns the recipe pattern that frames this mission.
    pub fn recipe_pattern(&self) -> &Symbol {
        &self.recipe_pattern
    }

    /// Returns the agent roles assigned to this mission.
    pub fn roles(&self) -> &[AtelierAgentRole] {
        &self.roles
    }

    /// Returns the deterministic Dev Cassette evidence stream id.
    pub fn evidence_stream(&self) -> &Symbol {
        &self.evidence_stream
    }

    /// Builds the SUP.56 agent-pattern descriptor for this mission.
    pub fn descriptor(&self) -> AgentPattern {
        AgentPattern {
            id: self.id.clone(),
            description: Some(format!(
                "Atelier mission using recipe pattern {}",
                self.recipe_pattern
            )),
            sense: slots([
                ("goal", self.goal.as_str()),
                ("scope", self.scope.summary.as_str()),
            ]),
            model: slots([("fake-cassette-runner", "deterministic runner surface")]),
            plan: slots([("f3-decompose", "split the mission into leased sub-tasks")]),
            act: role_slots(&self.roles),
            memory: slots([("f2-radar-retrieve", "rank query over the Atelier index")]),
            tools: slots(
                self.validations
                    .iter()
                    .chain(self.docs_runs.iter())
                    .map(|run| {
                        (
                            run.label.as_str(),
                            "required mission command recorded in evidence",
                        )
                    }),
            ),
            policy: slots([
                ("allowed-repos", "repository allow-list"),
                ("denied-paths", "path deny-list"),
                ("guard-capabilities", "granted Guideline Firewall tokens"),
            ]),
            evaluation: slots([
                (
                    "f3-reflect",
                    "reflect over edits against retrieved evidence",
                ),
                ("f4-confidence", "statistics-scored confidence"),
            ]),
            guardrail: slots([("guideline-firewall", "capability-gated action checks")]),
            trace: slots([
                (
                    "dev-cassette-ledger",
                    "read, edit, guard, validation, and refusal evidence",
                ),
                (
                    "f6-attribution",
                    "attribution card from the evidence ledger",
                ),
            ]),
            extra: vec![
                (
                    Symbol::new("recipe-pattern"),
                    Expr::Symbol(self.recipe_pattern.clone()),
                ),
                (
                    Symbol::new("allowed-repos"),
                    string_list_expr(&self.allowed_repos),
                ),
                (Symbol::new("leases"), leases_expr(&self.leases)),
            ],
        }
    }

    /// Encodes the mission as ordinary SIM expression data.
    pub fn as_expr(&self) -> Expr {
        Expr::Map(vec![
            key("kind", Expr::Symbol(Symbol::new("agent-mission"))),
            key("id", Expr::Symbol(self.id.clone())),
            key("goal", Expr::String(self.goal.clone())),
            key("scope", self.scope.as_expr()),
            key("allowed-repos", string_list_expr(&self.allowed_repos)),
            key("denied-paths", string_list_expr(&self.denied_paths)),
            key("code-free-repos", string_list_expr(&self.code_free_repos)),
            key("leases", leases_expr(&self.leases)),
            key("validations", runs_expr(&self.validations)),
            key("docs-runs", runs_expr(&self.docs_runs)),
            key("decision-points", decisions_expr(&self.decision_points)),
            key("recipe-pattern", Expr::Symbol(self.recipe_pattern.clone())),
            key("descriptor", self.descriptor().as_expr()),
        ])
    }
}

/// Mission scope with a primary repository and concise summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionScope {
    /// Primary repository the mission operates in.
    pub primary_repo: String,
    /// Concise human-readable summary of the scope.
    pub summary: String,
}

impl MissionScope {
    /// Builds a scope for a single repository with a default summary.
    pub fn repo(repo: impl Into<String>) -> Self {
        let repo = repo.into();
        Self {
            primary_repo: repo.clone(),
            summary: format!("changes in {repo}"),
        }
    }

    fn as_expr(&self) -> Expr {
        Expr::Map(vec![
            key("primary-repo", Expr::String(self.primary_repo.clone())),
            key("summary", Expr::String(self.summary.clone())),
        ])
    }
}

/// Required validation or docs command for a mission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionRun {
    /// Short label identifying the command.
    pub label: String,
    /// Shell command to run.
    pub command: String,
}

impl MissionRun {
    /// Builds a mission run from a label and command.
    pub fn new(label: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            command: command.into(),
        }
    }

    pub(crate) fn as_expr(&self) -> Expr {
        Expr::Map(vec![
            key("label", Expr::String(self.label.clone())),
            key("command", Expr::String(self.command.clone())),
        ])
    }
}

/// Human decision point declared by a mission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HumanDecisionPoint {
    /// Identifier for the decision point.
    pub id: String,
    /// Question posed to the human.
    pub question: String,
}

impl HumanDecisionPoint {
    /// Builds a human decision point from an id and question.
    pub fn new(id: impl Into<String>, question: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            question: question.into(),
        }
    }

    pub(crate) fn as_expr(&self) -> Expr {
        Expr::Map(vec![
            key("id", Expr::String(self.id.clone())),
            key("question", Expr::String(self.question.clone())),
        ])
    }
}

/// Agent roles used by Atelier missions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtelierAgentRole {
    /// Maps the repository and plans the mission.
    Cartographer,
    /// Applies source edits.
    Editor,
    /// Handles codec-specific work.
    CodecSpecialist,
    /// Enforces Guideline Firewall capability checks.
    Guard,
    /// Runs required validation commands.
    Validator,
    /// Runs documentation commands.
    DocsAgent,
    /// Updates pinned commits.
    PinAgent,
    /// Reviews the produced changes.
    Reviewer,
    /// Hands off to a human decision point.
    HumanGate,
}

impl AtelierAgentRole {
    /// Returns the role as a qualified `atelier/agent` symbol.
    pub fn as_symbol(&self) -> Symbol {
        Symbol::qualified("atelier/agent", self.label())
    }

    /// Returns the stable kebab-case label for this role.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Cartographer => "cartographer",
            Self::Editor => "editor",
            Self::CodecSpecialist => "codec-specialist",
            Self::Guard => "guard",
            Self::Validator => "validator",
            Self::DocsAgent => "docs-agent",
            Self::PinAgent => "pin-agent",
            Self::Reviewer => "reviewer",
            Self::HumanGate => "human-gate",
        }
    }
}

fn default_roles() -> Vec<AtelierAgentRole> {
    vec![
        AtelierAgentRole::Cartographer,
        AtelierAgentRole::Editor,
        AtelierAgentRole::CodecSpecialist,
        AtelierAgentRole::Guard,
        AtelierAgentRole::Validator,
        AtelierAgentRole::DocsAgent,
        AtelierAgentRole::PinAgent,
        AtelierAgentRole::Reviewer,
        AtelierAgentRole::HumanGate,
    ]
}

fn role_slots(roles: &[AtelierAgentRole]) -> Vec<AgentPatternSlot> {
    roles
        .iter()
        .map(|role| AgentPatternSlot {
            name: role.as_symbol(),
            description: Some("Atelier mission handoff role".to_owned()),
            extra: Vec::new(),
        })
        .collect()
}

fn slots(
    slots: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
) -> Vec<AgentPatternSlot> {
    slots
        .into_iter()
        .map(|(name, description)| AgentPatternSlot {
            name: Symbol::new(name.into()),
            description: Some(description.into()),
            extra: Vec::new(),
        })
        .collect()
}

fn runs_expr(runs: &[MissionRun]) -> Expr {
    Expr::List(runs.iter().map(MissionRun::as_expr).collect())
}

fn decisions_expr(decisions: &[HumanDecisionPoint]) -> Expr {
    Expr::List(decisions.iter().map(HumanDecisionPoint::as_expr).collect())
}

fn string_list_expr(values: &[String]) -> Expr {
    Expr::List(values.iter().cloned().map(Expr::String).collect())
}

pub(crate) fn key(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}

fn push_unique<T: PartialEq>(values: &mut Vec<T>, value: T) {
    if !values.contains(&value) {
        values.push(value);
    }
}
