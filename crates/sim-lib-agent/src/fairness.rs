//! Deterministic fairness and explanation facets for offline agent recipes.

use crate::value_from_expr;
use sim_kernel::{
    Cx, Error, Event, EventKind, EventLedger, Expr, NumberLiteral, Ref, Result, Symbol, Value,
    card::{Card, card_help_predicate, card_kind_predicate, card_result_predicate},
    effect_ledger::EffectLedger,
};
use sim_lib_numbers_stats::{BinaryOutcomeCounts, disparate_impact};
use std::{collections::BTreeMap, sync::Arc};

const ATTRIBUTION_KIND: &str = "fairness-attribution";

/// One attribution row derived from a run event or effect record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributionEvidence {
    /// Origin of the row, such as `event` or `effect`.
    pub source: Symbol,
    /// Event sequence number, when the row comes from an event.
    pub sequence: Option<u64>,
    /// Kind of event or effect record.
    pub kind: Symbol,
    /// Reference to the subject of the row.
    pub subject: Ref,
    /// Reference to the result, when one was produced.
    pub result: Option<Ref>,
    /// Whether the underlying effect resolved.
    pub resolved: bool,
    /// Whether the underlying effect was aborted.
    pub aborted: bool,
}

/// Card-shaped attribution summary over an event ledger and effect ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributionCard {
    /// Reference to the run this card summarizes.
    pub run: Ref,
    /// Number of events attributed to the run.
    pub event_count: usize,
    /// Number of effect records observed.
    pub effect_count: usize,
    /// Number of effect records that never resolved.
    pub unresolved_effect_count: usize,
    /// Number of effect records that were aborted.
    pub aborted_effect_count: usize,
    /// Per-event and per-effect attribution rows.
    pub evidence: Vec<AttributionEvidence>,
}

impl AttributionCard {
    /// Encodes this attribution card as ordinary SIM expression data.
    pub fn as_expr(&self) -> Expr {
        Expr::Map(vec![
            key(
                "kind",
                Expr::Symbol(Symbol::qualified("agent", ATTRIBUTION_KIND)),
            ),
            key("run", ref_expr(&self.run)),
            key("event-count", usize_expr(self.event_count)),
            key("effect-count", usize_expr(self.effect_count)),
            key(
                "unresolved-effect-count",
                usize_expr(self.unresolved_effect_count),
            ),
            key(
                "aborted-effect-count",
                usize_expr(self.aborted_effect_count),
            ),
            key(
                "evidence",
                Expr::List(
                    self.evidence
                        .iter()
                        .map(AttributionEvidence::as_expr)
                        .collect(),
                ),
            ),
        ])
    }

    /// Encodes this attribution as a kernel Card value for browse surfaces.
    pub fn as_card_value(&self, cx: &mut Cx) -> Result<Value> {
        let entries = vec![
            (
                card_kind_predicate(),
                value_from_expr(
                    cx,
                    &Expr::Symbol(Symbol::qualified("agent", ATTRIBUTION_KIND)),
                )?,
            ),
            (
                card_help_predicate(),
                value_from_expr(
                    cx,
                    &Expr::String(
                        "Deterministic attribution over run events and effects".to_owned(),
                    ),
                )?,
            ),
            (
                card_result_predicate(),
                value_from_expr(cx, &self.as_expr())?,
            ),
        ];
        cx.factory()
            .opaque(Arc::new(Card::new(self.run.clone(), entries)))
    }
}

impl AttributionEvidence {
    fn as_expr(&self) -> Expr {
        let mut entries = vec![
            key("source", Expr::Symbol(self.source.clone())),
            key("kind", Expr::Symbol(self.kind.clone())),
            key("subject", ref_expr(&self.subject)),
            key("resolved", Expr::Bool(self.resolved)),
            key("aborted", Expr::Bool(self.aborted)),
        ];
        if let Some(sequence) = self.sequence {
            entries.push(key("sequence", usize_expr(sequence as usize)));
        }
        if let Some(result) = &self.result {
            entries.push(key("result", ref_expr(result)));
        }
        Expr::Map(entries)
    }
}

/// Builds an attribution card from the evidence for one run.
pub fn attribution_card(
    run: Ref,
    events: &EventLedger,
    effects: &EffectLedger,
) -> Result<AttributionCard> {
    let run_events = events.events_for_run(&run);
    let records = effects.records();
    if run_events.is_empty() && records.is_empty() {
        return Err(Error::Eval(
            "attribution card requires event or effect evidence".to_owned(),
        ));
    }

    let mut evidence = Vec::with_capacity(run_events.len() + records.len());
    evidence.extend(run_events.iter().map(event_evidence));
    evidence.extend(records.iter().map(|record| AttributionEvidence {
        source: Symbol::new("effect"),
        sequence: None,
        kind: Symbol::new("effect-record"),
        subject: record.effect.clone(),
        result: record.result.clone(),
        resolved: record.resolved_event.is_some(),
        aborted: record.aborted,
    }));

    let unresolved_effect_count = records
        .iter()
        .filter(|record| record.resolved_event.is_none())
        .count();
    let aborted_effect_count = records.iter().filter(|record| record.aborted).count();

    Ok(AttributionCard {
        run,
        event_count: run_events.len(),
        effect_count: records.len(),
        unresolved_effect_count,
        aborted_effect_count,
        evidence,
    })
}

/// One deterministic synthetic record used for counterfactual search.
#[derive(Clone, Debug, PartialEq)]
pub struct SyntheticRecord {
    /// Unique record identifier.
    pub id: String,
    /// Group label the record belongs to.
    pub group: String,
    /// Whether the record was selected (the positive outcome).
    pub selected: bool,
    /// Finite feature values keyed by feature name.
    pub features: BTreeMap<String, f64>,
}

impl SyntheticRecord {
    /// Builds a synthetic record and rejects missing ids, groups, or live values.
    pub fn new(
        id: impl Into<String>,
        group: impl Into<String>,
        selected: bool,
        features: impl IntoIterator<Item = (impl Into<String>, f64)>,
    ) -> Result<Self> {
        let id = id.into();
        let group = group.into();
        if id.is_empty() {
            return Err(Error::Eval("synthetic record id is required".to_owned()));
        }
        if group.is_empty() {
            return Err(Error::Eval("synthetic record group is required".to_owned()));
        }
        let mut feature_map = BTreeMap::new();
        for (name, value) in features {
            let name = name.into();
            if name.is_empty() {
                return Err(Error::Eval("synthetic feature name is required".to_owned()));
            }
            if !value.is_finite() {
                return Err(Error::Eval(format!(
                    "synthetic feature {name} must be finite"
                )));
            }
            feature_map.insert(name, value);
        }
        if feature_map.is_empty() {
            return Err(Error::Eval(
                "synthetic record requires at least one feature".to_owned(),
            ));
        }
        Ok(Self {
            id,
            group,
            selected,
            features: feature_map,
        })
    }
}

/// One feature delta in a counterfactual explanation.
#[derive(Clone, Debug, PartialEq)]
pub struct CounterfactualChange {
    /// Name of the changed feature.
    pub feature: String,
    /// Source value, when the feature was present.
    pub from: Option<f64>,
    /// Counterfactual value, when the feature is present.
    pub to: Option<f64>,
    /// Magnitude of the change contributing to the distance.
    pub delta: f64,
}

/// Minimal-change counterfactual found in a deterministic synthetic table.
#[derive(Clone, Debug, PartialEq)]
pub struct Counterfactual {
    /// Id of the source record.
    pub source_id: String,
    /// Id of the nearest desired-outcome record.
    pub counterfactual_id: String,
    /// Outcome the counterfactual achieves.
    pub desired_selected: bool,
    /// Summed distance between the records.
    pub distance: f64,
    /// Per-feature changes from source to counterfactual.
    pub changes: Vec<CounterfactualChange>,
}

impl Counterfactual {
    /// Encodes the counterfactual as expression data for recipes.
    pub fn as_expr(&self) -> Expr {
        Expr::Map(vec![
            key("source-id", Expr::String(self.source_id.clone())),
            key(
                "counterfactual-id",
                Expr::String(self.counterfactual_id.clone()),
            ),
            key("desired-selected", Expr::Bool(self.desired_selected)),
            key("distance", f64_expr(self.distance)),
            key(
                "changes",
                Expr::List(
                    self.changes
                        .iter()
                        .map(CounterfactualChange::as_expr)
                        .collect(),
                ),
            ),
        ])
    }
}

impl CounterfactualChange {
    fn as_expr(&self) -> Expr {
        let mut entries = vec![
            key("feature", Expr::String(self.feature.clone())),
            key("delta", f64_expr(self.delta)),
        ];
        if let Some(from) = self.from {
            entries.push(key("from", f64_expr(from)));
        }
        if let Some(to) = self.to {
            entries.push(key("to", f64_expr(to)));
        }
        Expr::Map(entries)
    }
}

/// Finds the nearest synthetic record with the desired outcome.
pub fn minimal_counterfactual(
    records: &[SyntheticRecord],
    source_id: &str,
    desired_selected: bool,
) -> Result<Counterfactual> {
    let source = records
        .iter()
        .find(|record| record.id == source_id)
        .ok_or_else(|| Error::Eval(format!("counterfactual source not found: {source_id}")))?;
    if source.selected == desired_selected {
        return Err(Error::Eval(
            "counterfactual source already has desired outcome".to_owned(),
        ));
    }

    let mut candidates = records
        .iter()
        .filter(|record| record.id != source.id && record.selected == desired_selected)
        .map(|record| {
            let changes = feature_changes(source, record);
            let distance = changes.iter().map(|change| change.delta).sum::<f64>();
            (record, distance, changes)
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        left.1
            .total_cmp(&right.1)
            .then_with(|| left.0.id.cmp(&right.0.id))
    });

    let Some((record, distance, changes)) = candidates.into_iter().next() else {
        return Err(Error::Eval(
            "counterfactual search found no desired-outcome record".to_owned(),
        ));
    };

    Ok(Counterfactual {
        source_id: source.id.clone(),
        counterfactual_id: record.id.clone(),
        desired_selected,
        distance,
        changes,
    })
}

/// Binary-outcome group counts for fairness summaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FairnessGroup {
    /// Group label.
    pub label: String,
    /// Number of selected (positive-outcome) members.
    pub selected: u64,
    /// Total number of members.
    pub total: u64,
}

impl FairnessGroup {
    /// Builds a fairness group and rejects impossible counts.
    pub fn new(label: impl Into<String>, selected: u64, total: u64) -> Result<Self> {
        let label = label.into();
        if label.is_empty() {
            return Err(Error::Eval("fairness group label is required".to_owned()));
        }
        BinaryOutcomeCounts::new(selected, total).map_err(stats_error)?;
        Ok(Self {
            label,
            selected,
            total,
        })
    }

    fn counts(&self) -> Result<BinaryOutcomeCounts> {
        BinaryOutcomeCounts::new(self.selected, self.total).map_err(stats_error)
    }
}

/// Fairness summary backed by the F4 disparate-impact metric.
#[derive(Clone, Debug, PartialEq)]
pub struct FairnessSummary {
    /// Label of the reference group.
    pub reference_group: String,
    /// Label of the comparison group.
    pub comparison_group: String,
    /// Selection rate of the reference group.
    pub reference_rate: f64,
    /// Selection rate of the comparison group.
    pub comparison_rate: f64,
    /// Disparate-impact ratio against the four-fifths rule.
    pub four_fifths_ratio: f64,
    /// Whether the ratio passes the four-fifths threshold.
    pub passes_four_fifths: bool,
}

impl FairnessSummary {
    /// Encodes the summary as expression data for recipes.
    pub fn as_expr(&self) -> Expr {
        Expr::Map(vec![
            key(
                "reference-group",
                Expr::String(self.reference_group.clone()),
            ),
            key(
                "comparison-group",
                Expr::String(self.comparison_group.clone()),
            ),
            key("reference-rate", f64_expr(self.reference_rate)),
            key("comparison-rate", f64_expr(self.comparison_rate)),
            key("four-fifths-ratio", f64_expr(self.four_fifths_ratio)),
            key("passes-four-fifths", Expr::Bool(self.passes_four_fifths)),
        ])
    }
}

/// Computes a deterministic fairness summary for two binary-outcome groups.
pub fn fairness_summary(
    reference: &FairnessGroup,
    comparison: &FairnessGroup,
) -> Result<FairnessSummary> {
    let impact =
        disparate_impact(reference.counts()?, comparison.counts()?).map_err(stats_error)?;
    Ok(FairnessSummary {
        reference_group: reference.label.clone(),
        comparison_group: comparison.label.clone(),
        reference_rate: impact.reference_rate,
        comparison_rate: impact.comparison_rate,
        four_fifths_ratio: impact.ratio,
        passes_four_fifths: impact.passes_four_fifths,
    })
}

fn event_evidence(event: &Event) -> AttributionEvidence {
    let (kind, subject, result) = match &event.kind {
        EventKind::Started { request } => (Symbol::new("started"), request.clone(), None),
        EventKind::Claim { claim } => (Symbol::new("claim"), claim.clone(), None),
        EventKind::Diagnostic(_) => (
            Symbol::new("diagnostic"),
            Ref::Symbol(Symbol::qualified("event", "diagnostic")),
            None,
        ),
        EventKind::Trace(trace) => (Symbol::new("trace"), trace.clone(), None),
        EventKind::Chunk { payload } => (Symbol::new("chunk"), payload.clone(), None),
        EventKind::EffectRequested { effect } => {
            (Symbol::new("effect-requested"), effect.clone(), None)
        }
        EventKind::EffectResolved { effect, result } => (
            Symbol::new("effect-resolved"),
            effect.clone(),
            Some(result.clone()),
        ),
        EventKind::Capture { effect } => (Symbol::new("capture"), effect.clone(), None),
        EventKind::Card { subject, card } => {
            (Symbol::new("card"), subject.clone(), Some(card.clone()))
        }
        EventKind::Final(value) => (Symbol::new("final"), value.clone(), None),
        EventKind::Failed(error) => (Symbol::new("failed"), error.clone(), None),
        EventKind::Done => (
            Symbol::new("done"),
            Ref::Symbol(Symbol::qualified("event", "done")),
            None,
        ),
    };
    AttributionEvidence {
        source: Symbol::new("event"),
        sequence: Some(event.seq),
        kind,
        subject,
        result,
        resolved: true,
        aborted: false,
    }
}

fn feature_changes(
    source: &SyntheticRecord,
    counterfactual: &SyntheticRecord,
) -> Vec<CounterfactualChange> {
    let mut keys = source.features.keys().cloned().collect::<Vec<_>>();
    keys.extend(
        counterfactual
            .features
            .keys()
            .filter(|key| !source.features.contains_key(*key))
            .cloned(),
    );
    keys.sort();

    keys.into_iter()
        .filter_map(|feature| {
            let from = source.features.get(&feature).copied();
            let to = counterfactual.features.get(&feature).copied();
            let delta = match (from, to) {
                (Some(from), Some(to)) => (to - from).abs(),
                (Some(_), None) | (None, Some(_)) => 1.0,
                (None, None) => 0.0,
            };
            (delta > 0.0).then_some(CounterfactualChange {
                feature,
                from,
                to,
                delta,
            })
        })
        .collect()
}

fn ref_expr(reference: &Ref) -> Expr {
    match reference {
        Ref::Symbol(symbol) => Expr::Symbol(symbol.clone()),
        _ => Expr::String(format!("{reference:?}")),
    }
}

fn key(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}

fn usize_expr(value: usize) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn f64_expr(value: f64) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn stats_error(error: impl std::fmt::Display) -> Error {
    Error::Eval(error.to_string())
}
