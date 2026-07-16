use std::collections::{BTreeMap, BTreeSet};

use sim_codec_bridge::{BridgeBook, BridgePacket, BridgeVotePayload, content_id_string};
use sim_kernel::{ContentId, Cx, Error, Expr, Result, Symbol};
use sim_lib_bridge::{effective_caps, rx_check};
use sim_value::{access::field, build::entry};

use crate::{CompiledIntent, lift::content_id_for_expr};

/// Semantic verifier registered for a compiled intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verifier {
    /// Deterministic predicate evaluated over the decoded answer expression.
    Assertion {
        /// Predicate expression. Supported predicates are `forge/equals`,
        /// `forge/number-between`, and `forge/field-number-between`.
        predicate: Expr,
    },
    /// Judge vote represented as a checked BRIDGE packet.
    Judge {
        /// Seat that authored the judge packet.
        seat: String,
        /// Checked BRIDGE packet carrying one or more `bridge/Vote` parts.
        packet: Box<BridgePacket>,
        /// Optional parent packet used for BRIDGE reply legality.
        reply_to: Option<Box<BridgePacket>>,
        /// Vote target that must reach quorum.
        target: String,
        /// Minimum positive votes required for this verifier to pass.
        min_votes: u32,
    },
    /// Retrieval or ground-truth check against cited evidence.
    Evidence {
        /// `Given` ids, URIs, or content-id strings that must exist in the
        /// verifier catalog.
        cites: Vec<String>,
    },
}

/// Oracle backing a concrete verification probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeOracle {
    /// Expected decoded answer for the probe case.
    Expected(Expr),
    /// Content id of evidence that supplies the expected answer.
    Evidence(ContentId),
}

/// Concrete case that proves a compiled intent against required verifiers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyProbe {
    /// Concrete arguments for the known case.
    pub args: Expr,
    /// Expected answer or evidence reference for the known case.
    pub oracle: ProbeOracle,
    /// Verifier ids that must pass on this probe.
    pub verifier_ids: Vec<Symbol>,
}

impl VerifyProbe {
    /// Returns the content id of this probe record.
    pub fn content_id(&self) -> Result<ContentId> {
        content_id_for_expr(&self.to_expr())
    }

    fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            entry("args", self.args.clone()),
            entry("oracle", self.oracle.to_expr()),
            entry(
                "verifiers",
                Expr::Vector(
                    self.verifier_ids
                        .iter()
                        .cloned()
                        .map(Expr::Symbol)
                        .collect(),
                ),
            ),
        ])
    }
}

impl ProbeOracle {
    fn to_expr(&self) -> Expr {
        match self {
            Self::Expected(expected) => Expr::Map(vec![
                entry("kind", Expr::Symbol(Symbol::qualified("forge", "Expected"))),
                entry("answer", expected.clone()),
            ]),
            Self::Evidence(id) => Expr::Map(vec![
                entry("kind", Expr::Symbol(Symbol::qualified("forge", "Evidence"))),
                entry("content-id", Expr::String(content_id_string(id))),
            ]),
        }
    }
}

/// One failed semantic verifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyFailure {
    /// Verifier or probe id that failed.
    pub id: Symbol,
    /// Human-readable reason for the failure.
    pub reason: String,
}

/// Semantic verification result for one answer or probe set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    /// Verifier ids that passed.
    pub passed: Vec<Symbol>,
    /// Verifier or probe failures.
    pub failed: Vec<VerifyFailure>,
}

impl VerifyReport {
    /// Returns true when every required verifier passed.
    pub fn accepted(&self) -> bool {
        self.failed.is_empty()
    }

    fn pass(&mut self, id: Symbol) {
        self.passed.push(id);
    }

    fn fail(&mut self, id: Symbol, reason: impl Into<String>) {
        self.failed.push(VerifyFailure {
            id,
            reason: reason.into(),
        });
    }

    fn extend(&mut self, mut other: VerifyReport) {
        self.passed.append(&mut other.passed);
        self.failed.append(&mut other.failed);
    }
}

/// Local catalog binding verifier ids, probes, and cited ground truth.
#[derive(Clone, Debug, Default)]
pub struct VerifyCatalog {
    verifiers: BTreeMap<Symbol, Verifier>,
    probes: BTreeMap<ContentId, VerifyProbe>,
    intent_probes: BTreeMap<Symbol, BTreeSet<ContentId>>,
    evidence: BTreeMap<String, Expr>,
}

impl VerifyCatalog {
    /// Builds an empty verifier catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers or replaces a verifier by stable id.
    pub fn register_verifier(&mut self, id: Symbol, verifier: Verifier) {
        self.verifiers.insert(id, verifier);
    }

    /// Returns a verifier by id.
    pub fn verifier(&self, id: &Symbol) -> Option<&Verifier> {
        self.verifiers.get(id)
    }

    /// Registers a probe for an intent name and returns its content id.
    pub fn register_probe(&mut self, intent: Symbol, probe: VerifyProbe) -> Result<ContentId> {
        let id = probe.content_id()?;
        self.insert_probe(intent, id.clone(), probe);
        Ok(id)
    }

    /// Inserts a probe with an explicit content id.
    pub fn insert_probe(&mut self, intent: Symbol, id: ContentId, probe: VerifyProbe) {
        self.probes.insert(id.clone(), probe);
        self.intent_probes.entry(intent).or_default().insert(id);
    }

    /// Returns probe ids attached to `intent`.
    pub fn probe_ids_for(&self, intent: &Symbol) -> Vec<ContentId> {
        self.intent_probes
            .get(intent)
            .into_iter()
            .flat_map(|ids| ids.iter().cloned())
            .collect()
    }

    /// Stores cited evidence by `Given` id, URI, or content-id string.
    pub fn insert_evidence(&mut self, cite: impl Into<String>, value: Expr) {
        self.evidence.insert(cite.into(), value);
    }

    /// Stores cited evidence under its canonical content-id string.
    pub fn insert_content_evidence(&mut self, id: &ContentId, value: Expr) {
        self.insert_evidence(content_id_string(id), value);
    }

    /// Runs the intent's required verifiers against a decoded answer.
    pub fn verify_answer(
        &self,
        cx: &mut Cx,
        intent: &CompiledIntent,
        answer: &Expr,
    ) -> Result<VerifyReport> {
        let mut report = VerifyReport::default();
        for id in &intent.verifiers {
            match self.verifiers.get(id) {
                Some(verifier) => match self.run_verifier(cx, verifier, answer) {
                    Ok(()) => report.pass(id.clone()),
                    Err(reason) => report.fail(id.clone(), reason),
                },
                None => report.fail(id.clone(), "semantic verifier is not registered"),
            }
        }
        Ok(report)
    }

    /// Runs a concrete probe using only the verifiers named by the probe.
    pub fn verify_probe(
        &self,
        cx: &mut Cx,
        intent: &CompiledIntent,
        probe: &VerifyProbe,
    ) -> Result<VerifyReport> {
        let answer = self.answer_for_oracle(&probe.oracle)?;
        let mut scoped = intent.clone();
        scoped.verifiers = probe.verifier_ids.clone();
        self.verify_answer(cx, &scoped, &answer)
    }

    /// Runs every probe named by the intent and fails closed when proof is
    /// missing or incomplete.
    pub fn verify_intent_probes(
        &self,
        cx: &mut Cx,
        intent: &CompiledIntent,
    ) -> Result<VerifyReport> {
        let mut report = VerifyReport::default();
        if intent.verifiers.is_empty() {
            report.fail(
                Symbol::qualified("forge", "verifier"),
                "intent declares no semantic verifiers",
            );
            return Ok(report);
        }
        if intent.probes.is_empty() {
            report.fail(
                Symbol::qualified("forge", "probe"),
                "intent declares no verification probes",
            );
            return Ok(report);
        }

        let covered = self.covered_verifiers(intent);
        for required in &intent.verifiers {
            if !covered.contains(required) {
                report.fail(
                    required.clone(),
                    "no verification probe requires this verifier",
                );
            }
        }

        for id in &intent.probes {
            match self.probes.get(id) {
                Some(probe) => report.extend(self.verify_probe(cx, intent, probe)?),
                None => report.fail(
                    Symbol::qualified("forge", "probe"),
                    format!(
                        "verification probe {} is not registered",
                        content_id_string(id)
                    ),
                ),
            }
        }
        Ok(report)
    }

    fn covered_verifiers(&self, intent: &CompiledIntent) -> BTreeSet<Symbol> {
        intent
            .probes
            .iter()
            .filter_map(|id| self.probes.get(id))
            .flat_map(|probe| probe.verifier_ids.iter().cloned())
            .collect()
    }

    fn answer_for_oracle(&self, oracle: &ProbeOracle) -> Result<Expr> {
        match oracle {
            ProbeOracle::Expected(answer) => Ok(answer.clone()),
            ProbeOracle::Evidence(id) => self
                .evidence
                .get(&content_id_string(id))
                .cloned()
                .ok_or_else(|| {
                    Error::Eval(format!(
                        "probe evidence {} is absent",
                        content_id_string(id)
                    ))
                }),
        }
    }

    fn run_verifier(
        &self,
        cx: &mut Cx,
        verifier: &Verifier,
        answer: &Expr,
    ) -> std::result::Result<(), String> {
        match verifier {
            Verifier::Assertion { predicate } => {
                let narrowed = sim_kernel::CapabilitySet::new();
                cx.with_capabilities(narrowed, |_| check_assertion(predicate, answer))
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
            Verifier::Judge {
                seat,
                packet,
                reply_to,
                target,
                min_votes,
            } => {
                let narrowed = effective_caps(cx, packet).map_err(|err| err.to_string())?;
                cx.with_capabilities(narrowed, |scoped| {
                    check_judge(
                        scoped,
                        seat,
                        packet,
                        reply_to.as_deref(),
                        target,
                        *min_votes,
                    )
                })
                .map_err(|err| err.to_string())
                .and_then(|result| result)
            }
            Verifier::Evidence { cites } => {
                let narrowed = sim_kernel::CapabilitySet::new();
                cx.with_capabilities(narrowed, |_| Ok(self.check_evidence(cites, answer)))
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
        }
    }

    fn check_evidence(&self, cites: &[String], answer: &Expr) -> std::result::Result<(), String> {
        if cites.is_empty() {
            return Err("evidence verifier must cite at least one source".to_owned());
        }
        let mut matched = false;
        for cite in cites {
            let Some(evidence) = self.evidence.get(cite) else {
                return Err(format!("cited evidence {cite} is absent"));
            };
            matched |= answer.canonical_eq(evidence);
        }
        if matched {
            Ok(())
        } else {
            Err("answer does not match cited ground truth".to_owned())
        }
    }
}

/// Runs semantic verification with an empty catalog.
///
/// Callers that need registered verifier definitions should use
/// [`VerifyCatalog::verify_answer`].
pub fn verify_answer(cx: &mut Cx, intent: &CompiledIntent, answer: &Expr) -> Result<VerifyReport> {
    VerifyCatalog::new().verify_answer(cx, intent, answer)
}

fn check_assertion(predicate: &Expr, answer: &Expr) -> Result<std::result::Result<(), String>> {
    let kind = match field(predicate, "predicate") {
        Some(Expr::Symbol(symbol)) => symbol,
        _ => {
            return Ok(Err(
                "assertion predicate must name a predicate symbol".to_owned()
            ));
        }
    };

    if *kind == Symbol::qualified("forge", "equals") {
        let Some(expected) = field(predicate, "expected") else {
            return Ok(Err("forge/equals predicate is missing expected".to_owned()));
        };
        return if answer.canonical_eq(expected) {
            Ok(Ok(()))
        } else {
            Ok(Err("answer did not equal expected expression".to_owned()))
        };
    }

    if *kind == Symbol::qualified("forge", "number-between") {
        return check_number_between(predicate, answer);
    }

    if *kind == Symbol::qualified("forge", "field-number-between") {
        let field_name = match field(predicate, "field") {
            Some(Expr::String(name)) => name.as_str(),
            Some(Expr::Symbol(symbol)) if symbol.namespace.is_none() => symbol.name.as_ref(),
            _ => {
                return Ok(Err(
                    "field-number-between predicate is missing field".to_owned()
                ));
            }
        };
        let Some(value) = field(answer, field_name) else {
            return Ok(Err(format!("answer is missing field {field_name}")));
        };
        return check_number_between(predicate, value);
    }

    Ok(Err(format!("unsupported assertion predicate {kind}")))
}

fn check_number_between(predicate: &Expr, value: &Expr) -> Result<std::result::Result<(), String>> {
    let Some(value) = integer_value(value)? else {
        return Ok(Err("answer value is not an integer number".to_owned()));
    };
    let min = optional_i64(predicate, "min")?;
    let max = optional_i64(predicate, "max")?;
    if let Some(min) = min
        && value < min
    {
        return Ok(Err(format!("answer value {value} is below minimum {min}")));
    }
    if let Some(max) = max
        && value > max
    {
        return Ok(Err(format!("answer value {value} is above maximum {max}")));
    }
    Ok(Ok(()))
}

fn optional_i64(expr: &Expr, name: &str) -> Result<Option<i64>> {
    match field(expr, name) {
        Some(value) => integer_value(value)?
            .map(Some)
            .ok_or_else(|| Error::Eval(format!("{name} must be an integer number when present"))),
        None => Ok(None),
    }
}

fn integer_value(expr: &Expr) -> Result<Option<i64>> {
    match expr {
        Expr::Number(number) => number
            .canonical
            .parse::<i64>()
            .map(Some)
            .map_err(|_| Error::Eval(format!("{} is not an integer", number.canonical))),
        _ => Ok(None),
    }
}

fn check_judge(
    cx: &mut Cx,
    seat: &str,
    packet: &BridgePacket,
    reply_to: Option<&BridgePacket>,
    target: &str,
    min_votes: u32,
) -> Result<std::result::Result<(), String>> {
    if min_votes == 0 {
        return Ok(Err("judge quorum must require at least one vote".to_owned()));
    }
    if packet.header.from != seat {
        return Ok(Err(format!(
            "judge packet came from {}, expected {seat}",
            packet.header.from
        )));
    }
    let report = rx_check(cx, &BridgeBook::standard(), packet, reply_to)?;
    if !report.accepted() {
        return Ok(Err(format!(
            "judge packet failed BRIDGE rx_check: {:?}",
            report.obligations
        )));
    }

    let mut votes = 0u32;
    for part in &packet.body {
        if part.kind != Symbol::qualified("bridge", "Vote") {
            continue;
        }
        let vote = BridgeVotePayload::from_expr(&part.payload)?;
        if vote.target == target && vote.scores.iter().any(|score| score.value > 0) {
            votes = votes.saturating_add(1);
        }
    }

    if votes >= min_votes {
        Ok(Ok(()))
    } else {
        Ok(Err(format!(
            "judge quorum for {target} has {votes} vote(s), needs {min_votes}"
        )))
    }
}
