//! Change Capsule review model for agent-operated Atelier edits.

use std::collections::BTreeSet;

use sim_kernel::{Expr, Result, Symbol};
use sim_lib_stream_core::{DevCassette, DevEvent, LatencyClass};

/// Auditable package of patches, validation evidence, pins, and replay data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeCapsule {
    /// Stable capsule id.
    pub id: Symbol,
    /// Repositories and files the capsule is allowed to touch.
    pub scope: CapsuleScope,
    /// Patch summaries included in the review.
    pub patches: Vec<CapsulePatch>,
    /// Generated artifacts produced by tooling.
    pub generated_artifacts: Vec<GeneratedArtifact>,
    /// Validation jobs and their placed execution sites.
    pub validations: Vec<CapsuleJob>,
    /// Documentation jobs and their placed execution sites.
    pub docs_runs: Vec<CapsuleJob>,
    /// Public repo commits produced by the capsule.
    pub commits: Vec<CapsuleCommit>,
    /// Push records for public commits.
    pub pushes: Vec<CapsulePush>,
    /// Planned `repos.toml` pin updates.
    pub pin_plan: Vec<PinPlanEntry>,
    /// Generated front-page changes.
    pub site_changes: Vec<GeneratedArtifact>,
    /// Human-review risk notes.
    pub risks: Vec<String>,
    /// Rollback notes for each affected repo.
    pub rollback_notes: Vec<String>,
    /// Placement plan for editing, validation, docs, pins, and capsule assembly.
    pub placement_plan: Vec<PlacedJob>,
    /// Recorded development cassette for replay.
    pub cassette: DevCassette,
    /// F6-style fairness and attribution facet.
    pub fairness: CapsuleFacet,
}

/// Scope declared by a Change Capsule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapsuleScope {
    /// Repository names in scope.
    pub repos: Vec<String>,
    /// File or directory targets in scope.
    pub targets: Vec<String>,
}

/// One patch summarized for review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapsulePatch {
    /// Repository containing the patch.
    pub repo: String,
    /// Path changed by the patch.
    pub path: String,
    /// Review summary.
    pub summary: String,
}

impl CapsulePatch {
    /// Builds a patch summary.
    pub fn new(
        repo: impl Into<String>,
        path: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            repo: repo.into(),
            path: path.into(),
            summary: summary.into(),
        }
    }
}

/// Generated artifact or public front-page change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedArtifact {
    /// Repository that owns the artifact.
    pub repo: String,
    /// Artifact path.
    pub path: String,
    /// Tool that generated or checked the artifact.
    pub generator: String,
    /// Whether the artifact is part of a generated public docs lane.
    pub generated_public_doc: bool,
    /// Whether the capsule attempted a hand edit.
    pub hand_edited: bool,
}

impl GeneratedArtifact {
    /// Builds a generated artifact record.
    pub fn generated(
        repo: impl Into<String>,
        path: impl Into<String>,
        generator: impl Into<String>,
    ) -> Self {
        Self {
            repo: repo.into(),
            path: path.into(),
            generator: generator.into(),
            generated_public_doc: true,
            hand_edited: false,
        }
    }
}

/// Validation or docs job included in a capsule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapsuleJob {
    /// Job kind.
    pub kind: CapsuleJobKind,
    /// Stable job label.
    pub label: String,
    /// Command line or typed command template.
    pub command: String,
    /// Job placement.
    pub site: JobSite,
    /// Review outcome.
    pub outcome: CapsuleJobOutcome,
    /// Evidence log path.
    pub log_path: String,
}

impl CapsuleJob {
    /// Builds a passed validation job on a process site.
    pub fn validation(label: impl Into<String>, command: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            kind: CapsuleJobKind::Validation,
            label: label.clone(),
            command: command.into(),
            site: JobSite::Process,
            outcome: CapsuleJobOutcome::Passed,
            log_path: format!(".sim/atelier/logs/{label}.log"),
        }
    }

    /// Builds a passed docs job on a process site.
    pub fn docs(label: impl Into<String>, command: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            kind: CapsuleJobKind::Docs,
            label: label.clone(),
            command: command.into(),
            site: JobSite::Process,
            outcome: CapsuleJobOutcome::Passed,
            log_path: format!(".sim/atelier/logs/{label}.log"),
        }
    }

    /// Returns a copy marked as failed.
    pub fn failed(mut self) -> Self {
        self.outcome = CapsuleJobOutcome::Failed;
        self
    }
}

/// Capsule job kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapsuleJobKind {
    /// Validation command.
    Validation,
    /// Documentation command.
    Docs,
}

impl CapsuleJobKind {
    /// Stable kind label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Docs => "docs",
        }
    }
}

/// Result of a capsule job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapsuleJobOutcome {
    /// Job passed.
    Passed,
    /// Job failed.
    Failed,
}

/// Site class used for placed validation and docs jobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobSite {
    /// Local coroutine site for edit and assembly work.
    LocalCoroutine,
    /// Process site reached through `realize`.
    Process,
    /// Fabric site reached through `realize`.
    Fabric,
}

impl JobSite {
    /// Stable site label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalCoroutine => "local-coroutine",
            Self::Process => "process",
            Self::Fabric => "fabric",
        }
    }

    /// SUP.20 realization operation used for this placement.
    pub fn realize_operation(self) -> &'static str {
        match self {
            Self::LocalCoroutine => "local-coroutine",
            Self::Process | Self::Fabric => "realize",
        }
    }

    fn can_run_validation(self) -> bool {
        matches!(self, Self::Process | Self::Fabric)
    }
}

/// One public commit referenced by a capsule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapsuleCommit {
    /// Repository name.
    pub repo: String,
    /// Commit hash.
    pub hash: String,
}

/// One push record referenced by a capsule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapsulePush {
    /// Repository name.
    pub repo: String,
    /// Remote name or URL label.
    pub remote: String,
    /// Commit hash that was pushed.
    pub hash: String,
}

/// Planned `repos.toml` pin update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinPlanEntry {
    /// Repository name.
    pub repo: String,
    /// Current pinned commit.
    pub current_commit: String,
    /// New commit to pin.
    pub new_commit: String,
    /// Whether the new commit exists on the upstream remote.
    pub pushed_commit_exists: bool,
}

/// One placement-plan row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacedJob {
    /// Job label such as `edit`, `validation`, `docs`, or `capsule-assembly`.
    pub label: String,
    /// Site used for the job.
    pub site: JobSite,
}

impl PlacedJob {
    /// Builds a placement row.
    pub fn new(label: impl Into<String>, site: JobSite) -> Self {
        Self {
            label: label.into(),
            site,
        }
    }
}

/// F6-style fairness facet attached to a capsule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapsuleFacet {
    /// Facet label.
    pub label: String,
    /// Evidence summary.
    pub evidence: String,
    /// Confidence token.
    pub confidence: String,
}

/// Deterministic review output for a Change Capsule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapsuleReview {
    /// Whether the capsule is accepted.
    pub accepted: bool,
    /// Original cassette content hash.
    pub content_hash: String,
    /// Replay-computed content hash.
    pub replay_content_hash: String,
    /// Repositories previewed before pin edits.
    pub preview_repos: Vec<String>,
    /// Review failures.
    pub failure_reasons: Vec<String>,
}

/// Reviews a capsule against replay, placement, conformance, docs, and pin rules.
pub fn review_change_capsule(capsule: &ChangeCapsule) -> Result<CapsuleReview> {
    let replay_content_hash = capsule.cassette.replay_content_hash()?;
    let mut failure_reasons = Vec::new();
    if capsule.cassette.content_hash() != replay_content_hash {
        failure_reasons.push("cassette replay content hash mismatch".to_owned());
    }
    check_jobs(&capsule.validations, &mut failure_reasons);
    check_jobs(&capsule.docs_runs, &mut failure_reasons);
    check_placements(&capsule.placement_plan, &mut failure_reasons);
    check_generated_docs(&capsule.generated_artifacts, &mut failure_reasons);
    check_generated_docs(&capsule.site_changes, &mut failure_reasons);
    check_pins(&capsule.pin_plan, &mut failure_reasons);

    Ok(CapsuleReview {
        accepted: failure_reasons.is_empty(),
        content_hash: capsule.cassette.content_hash().to_owned(),
        replay_content_hash,
        preview_repos: preview_repos(capsule),
        failure_reasons,
    })
}

/// Builds a deterministic capsule fixture used by agents and view tests.
pub fn fake_change_capsule() -> Result<ChangeCapsule> {
    let node = Symbol::qualified("atelier/agent", "change-capsule");
    let cassette = DevCassette::from_events(
        Symbol::qualified("atelier/dev", "change-capsule-fixture"),
        vec![
            DevEvent::edit(node.clone(), summary("Patch capsule model"))?,
            DevEvent::validate(node.clone(), summary("cargo test change_capsule"))?,
            DevEvent::new(
                "docs",
                node.clone(),
                LatencyClass::OfflineRender,
                summary("simdoc check current"),
            )?,
            DevEvent::new(
                "pin",
                node.clone(),
                LatencyClass::Interactive,
                summary("pushed commit exists before pin"),
            )?,
            DevEvent::new(
                "reflect",
                node,
                LatencyClass::OfflineRender,
                summary("F6 facet records risk and rollback"),
            )?,
        ],
    )?;

    Ok(ChangeCapsule {
        id: Symbol::qualified("atelier/capsule", "fixture"),
        scope: CapsuleScope {
            repos: vec!["sim-agent-net".to_owned(), "sim-tooling".to_owned()],
            targets: vec![
                "crates/sim-lib-agent/src/atelier/capsule.rs".to_owned(),
                "src/atelier/capsule.rs".to_owned(),
            ],
        },
        patches: vec![
            CapsulePatch::new(
                "sim-agent-net",
                "crates/sim-lib-agent/src/atelier/capsule.rs",
                "agent capsule model",
            ),
            CapsulePatch::new("sim-tooling", "src/atelier/capsule.rs", "capsule cache"),
        ],
        generated_artifacts: vec![GeneratedArtifact::generated(
            "sim-tooling",
            "docs/generated/contract.md",
            "xtask simdoc",
        )],
        validations: vec![CapsuleJob::validation(
            "agent-capsule-tests",
            "cargo test -p sim-lib-agent change_capsule",
        )],
        docs_runs: vec![CapsuleJob::docs(
            "simdoc-agent-net",
            "cargo run -p xtask -- simdoc --check",
        )],
        commits: vec![CapsuleCommit {
            repo: "sim-agent-net".to_owned(),
            hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        }],
        pushes: vec![CapsulePush {
            repo: "sim-agent-net".to_owned(),
            remote: "origin".to_owned(),
            hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        }],
        pin_plan: vec![PinPlanEntry {
            repo: "sim-agent-net".to_owned(),
            current_commit: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
            new_commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            pushed_commit_exists: true,
        }],
        site_changes: vec![GeneratedArtifact::generated(
            "repo-docs",
            "docs/site/repos.md",
            "simctl site",
        )],
        risks: vec!["review capsule scope before pinning".to_owned()],
        rollback_notes: vec!["reset pin to previous commit and rerun site".to_owned()],
        placement_plan: vec![
            PlacedJob::new("edit", JobSite::LocalCoroutine),
            PlacedJob::new("capsule-assembly", JobSite::LocalCoroutine),
            PlacedJob::new("validation", JobSite::Process),
            PlacedJob::new("docs", JobSite::Process),
            PlacedJob::new("pin-plan", JobSite::LocalCoroutine),
        ],
        cassette,
        fairness: CapsuleFacet {
            label: "F6 trade-off".to_owned(),
            evidence: "validation, docs, pin, and rollback evidence recorded".to_owned(),
            confidence: "0.92".to_owned(),
        },
    })
}

fn check_jobs(jobs: &[CapsuleJob], failure_reasons: &mut Vec<String>) {
    for job in jobs {
        if !job.site.can_run_validation() {
            failure_reasons.push(format!(
                "{} job {} must run on process or fabric site",
                job.kind.as_str(),
                job.label
            ));
        }
        if job.site.realize_operation() != "realize" {
            failure_reasons.push(format!(
                "{} job {} is not realized",
                job.kind.as_str(),
                job.label
            ));
        }
        if job.outcome == CapsuleJobOutcome::Failed {
            failure_reasons.push(format!("{} job {} failed", job.kind.as_str(), job.label));
        }
    }
}

fn check_placements(placements: &[PlacedJob], failure_reasons: &mut Vec<String>) {
    for label in ["edit", "capsule-assembly"] {
        let local = placements
            .iter()
            .any(|job| job.label == label && job.site == JobSite::LocalCoroutine);
        if !local {
            failure_reasons.push(format!("{label} must stay on the local coroutine site"));
        }
    }
    for label in ["validation", "docs"] {
        let realized = placements
            .iter()
            .any(|job| job.label == label && job.site.can_run_validation());
        if !realized {
            failure_reasons.push(format!("{label} must be placed on process or fabric site"));
        }
    }
}

fn check_generated_docs(artifacts: &[GeneratedArtifact], failure_reasons: &mut Vec<String>) {
    for artifact in artifacts {
        if artifact.generated_public_doc && artifact.hand_edited {
            failure_reasons.push(format!(
                "generated public doc {}:{} must be regenerated, not hand-edited",
                artifact.repo, artifact.path
            ));
        }
    }
}

fn check_pins(pins: &[PinPlanEntry], failure_reasons: &mut Vec<String>) {
    for pin in pins {
        if !pin.pushed_commit_exists {
            failure_reasons.push(format!(
                "pin plan for {} requires an existing pushed upstream commit",
                pin.repo
            ));
        }
    }
}

fn preview_repos(capsule: &ChangeCapsule) -> Vec<String> {
    let mut repos = BTreeSet::new();
    repos.extend(capsule.scope.repos.iter().cloned());
    repos.extend(capsule.pin_plan.iter().map(|pin| pin.repo.clone()));
    repos.into_iter().collect()
}

fn summary(text: &str) -> Expr {
    Expr::Map(vec![(
        Expr::Symbol(Symbol::new("summary")),
        Expr::String(text.to_owned()),
    )])
}
