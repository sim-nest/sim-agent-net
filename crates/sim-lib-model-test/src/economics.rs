//! Replayable model-resource evidence and economic admission.
//!
//! This module records domain facts. Cost aggregation and decisions remain in
//! `sim-lib-study`; there is deliberately no balance ledger or alternate cost
//! formula here.

use sim_kernel::{ContentId, Symbol};
use sim_lib_study::{AttributedResourceEvent, ResourceCause};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DecisionAxis {
    Monetary,
    Quota,
    Capacity,
    Energy,
    Latency,
    MaintainerActiveTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation<T> {
    Available(T),
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PriceKind {
    InputToken,
    OutputToken,
    CachedInputToken,
    Credit,
    SubscriptionMarginal,
    SubscriptionAllocatedPeriod,
    Electricity,
    HardwareOccupancy,
    OperatorAttention,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceObservation {
    pub revision: ContentId,
    pub kind: PriceKind,
    pub amount_minor: u64,
    pub currency: Symbol,
    pub quantity: u64,
    pub observed_at_ms: u64,
    pub valid_until_ms: Option<u64>,
    pub evidence: ContentId,
}

#[derive(Clone, Debug, Default)]
pub struct PriceCatalog {
    revisions: BTreeMap<ContentId, PriceObservation>,
}

impl PriceCatalog {
    pub fn insert(&mut self, price: PriceObservation) -> Result<(), EconomicError> {
        if price.quantity == 0 {
            return Err(EconomicError::InvalidPrice);
        }
        match self.revisions.get(&price.revision) {
            Some(existing) if existing != &price => Err(EconomicError::RevisionCollision),
            Some(_) => Ok(()),
            None => {
                self.revisions.insert(price.revision.clone(), price);
                Ok(())
            }
        }
    }

    pub fn resolve(&self, revision: &ContentId, at_ms: u64) -> Observation<&PriceObservation> {
        match self.revisions.get(revision) {
            Some(price) if price.valid_until_ms.is_none_or(|expiry| at_ms <= expiry) => {
                Observation::Available(price)
            }
            _ => Observation::Unavailable,
        }
    }

    pub fn remove(&mut self, revision: &ContentId) {
        self.revisions.remove(revision);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnergyEvidenceKind {
    Measured,
    DeclaredPowerProfile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptResourceEvent {
    pub attempt: ContentId,
    pub evidence: ContentId,
    pub subject: Option<ContentId>,
    pub route: Option<ContentId>,
    pub axis: DecisionAxis,
    pub resource: Symbol,
    pub amount: u64,
    pub cause: ResourceCause,
    pub energy_evidence: Option<EnergyEvidenceKind>,
}

impl AttemptResourceEvent {
    pub fn generic_event(&self) -> AttributedResourceEvent {
        AttributedResourceEvent {
            evidence: self.evidence.clone(),
            subject: self.subject.clone(),
            route: self.route.clone(),
            resource: self.resource.clone(),
            amount: self.amount,
            cause: self.cause,
        }
    }
}

pub fn preferred_energy_event(
    measured: Option<AttemptResourceEvent>,
    declared: Option<AttemptResourceEvent>,
) -> Observation<AttemptResourceEvent> {
    measured
        .filter(|event| event.energy_evidence == Some(EnergyEvidenceKind::Measured))
        .or_else(|| {
            declared.filter(|event| {
                event.energy_evidence == Some(EnergyEvidenceKind::DeclaredPowerProfile)
            })
        })
        .map_or(Observation::Unavailable, Observation::Available)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptEnvelope {
    pub turns: u32,
    pub repairs_per_turn: u32,
    pub smokes: u32,
    pub retries_per_attempt: u32,
}

impl AttemptEnvelope {
    pub fn maximum_attempts(&self) -> u64 {
        let turns = u64::from(self.turns);
        let repairs = turns.saturating_mul(u64::from(self.repairs_per_turn));
        let bases = turns
            .saturating_add(repairs)
            .saturating_add(u64::from(self.smokes));
        bases.saturating_mul(u64::from(self.retries_per_attempt).saturating_add(1))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExposureSeal {
    pub missing_coordinates: BTreeSet<ContentId>,
    pub account_pseudonym: ContentId,
    pub price_revision: ContentId,
    pub reserve_minor: u64,
    pub expires_at_ms: u64,
    pub resource: Symbol,
    pub maximum_resource_per_attempt: u64,
    pub envelope: AttemptEnvelope,
}

impl ExposureSeal {
    pub fn maximum_resource(&self) -> u64 {
        self.maximum_resource_per_attempt
            .saturating_mul(self.envelope.maximum_attempts())
            .saturating_mul(self.missing_coordinates.len() as u64)
    }

    pub fn admit(&self, catalog: &PriceCatalog, now_ms: u64) -> Observation<()> {
        if now_ms > self.expires_at_ms
            || self.missing_coordinates.is_empty()
            || matches!(
                catalog.resolve(&self.price_revision, now_ms),
                Observation::Unavailable
            )
        {
            Observation::Unavailable
        } else {
            Observation::Available(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderUsagePage {
    pub page: u32,
    pub next_page: Option<u32>,
    pub launch_start_ms: u64,
    pub launch_end_ms: u64,
    pub exact_model_id: String,
    pub billable_response: bool,
    pub request_id: Option<String>,
    pub resource_events: Vec<AttemptResourceEvent>,
    pub concurrent_unrelated_activity: bool,
    pub balance_delta_minor: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reconciliation {
    Exact(Vec<AttemptResourceEvent>),
    Unreconciled { reasons: BTreeSet<Symbol> },
}

pub fn reconcile_provider_usage(
    pages: &[ProviderUsagePage],
    launch_start_ms: u64,
    launch_end_ms: u64,
    exact_model_id: &str,
) -> Reconciliation {
    let mut reasons = BTreeSet::new();
    let mut expected_page = 0;
    let mut events = Vec::new();
    for page in pages {
        if page.page != expected_page {
            reasons.insert(Symbol::new("pagination-gap"));
        }
        expected_page = page.next_page.unwrap_or(page.page.saturating_add(1));
        if page.launch_start_ms < launch_start_ms || page.launch_end_ms > launch_end_ms {
            reasons.insert(Symbol::new("outside-launch-interval"));
        }
        if page.exact_model_id != exact_model_id {
            reasons.insert(Symbol::new("model-id-mismatch"));
        }
        if page.billable_response && page.request_id.as_deref().is_none_or(str::is_empty) {
            reasons.insert(Symbol::new("missing-request-id"));
        }
        if page.concurrent_unrelated_activity {
            reasons.insert(Symbol::new("concurrent-unrelated-activity"));
        }
        if page.balance_delta_minor.is_some() && page.resource_events.is_empty() {
            reasons.insert(Symbol::new("balance-delta-only"));
        }
        events.extend(page.resource_events.clone());
    }
    if pages.last().is_some_and(|page| page.next_page.is_some()) {
        reasons.insert(Symbol::new("pagination-incomplete"));
    }
    if reasons.is_empty() {
        Reconciliation::Exact(events)
    } else {
        Reconciliation::Unreconciled { reasons }
    }
}

pub fn ceiling_evidence(reconciliation: &Reconciliation) -> Observation<&[AttemptResourceEvent]> {
    match reconciliation {
        Reconciliation::Exact(events) => Observation::Available(events),
        Reconciliation::Unreconciled { .. } => Observation::Unavailable,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureEffect {
    CandidateOperational,
    Observer,
    Unattributed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FailureProjection {
    pub record_resources: bool,
    pub quality_delta: i8,
    pub comparative_cost_censored: bool,
}

pub fn project_failure(effect: FailureEffect) -> FailureProjection {
    FailureProjection {
        record_resources: true,
        quality_delta: 0,
        comparative_cost_censored: !matches!(effect, FailureEffect::CandidateOperational),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EconomicError {
    InvalidPrice,
    RevisionCollision,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_kernel::Datum;
    use sim_lib_study::{cost_per_accepted, summarize_spend};

    fn cid(value: &str) -> ContentId {
        Datum::String(value.into()).content_id().unwrap()
    }

    fn price(revision: ContentId, amount_minor: u64, expiry: Option<u64>) -> PriceObservation {
        PriceObservation {
            revision,
            kind: PriceKind::InputToken,
            amount_minor,
            currency: Symbol::new("test-minor-unit"),
            quantity: 1,
            observed_at_ms: 1,
            valid_until_ms: expiry,
            evidence: cid("price-source"),
        }
    }

    fn event(attempt: &str, amount: u64, cause: ResourceCause) -> AttemptResourceEvent {
        AttemptResourceEvent {
            attempt: cid(attempt),
            evidence: cid(&format!("evidence-{attempt}")),
            subject: Some(cid("subject")),
            route: Some(cid("route")),
            axis: DecisionAxis::Monetary,
            resource: Symbol::new("minor-unit"),
            amount,
            cause,
            energy_evidence: None,
        }
    }

    #[test]
    fn unavailable_is_not_zero_and_subscription_prices_are_distinct() {
        assert_ne!(Observation::Unavailable, Observation::Available(0));
        assert_ne!(
            PriceKind::SubscriptionMarginal,
            PriceKind::SubscriptionAllocatedPeriod
        );
    }

    #[test]
    fn price_revisions_are_immutable_and_expiry_does_not_reprice_history() {
        let revision = cid("price-r1");
        let mut catalog = PriceCatalog::default();
        catalog
            .insert(price(revision.clone(), 7, Some(10)))
            .unwrap();
        assert_eq!(
            catalog.insert(price(revision.clone(), 8, Some(10))),
            Err(EconomicError::RevisionCollision)
        );
        assert!(matches!(
            catalog.resolve(&revision, 10),
            Observation::Available(_)
        ));
        assert!(matches!(
            catalog.resolve(&revision, 11),
            Observation::Unavailable
        ));
        catalog.remove(&revision);
        assert!(matches!(
            catalog.resolve(&revision, 1),
            Observation::Unavailable
        ));
    }

    #[test]
    fn measured_energy_outranks_declared_and_both_name_evidence() {
        let mut declared = event("declared", 30, ResourceCause::CandidateRoute);
        declared.axis = DecisionAxis::Energy;
        declared.energy_evidence = Some(EnergyEvidenceKind::DeclaredPowerProfile);
        let mut measured = event("measured", 20, ResourceCause::CandidateRoute);
        measured.axis = DecisionAxis::Energy;
        measured.energy_evidence = Some(EnergyEvidenceKind::Measured);
        assert_eq!(
            preferred_energy_event(Some(measured.clone()), Some(declared)),
            Observation::Available(measured)
        );
    }

    #[test]
    fn exposure_counts_turns_repairs_smokes_retries_and_exact_coordinates() {
        let seal = ExposureSeal {
            missing_coordinates: [cid("c1"), cid("c2")].into_iter().collect(),
            account_pseudonym: cid("account-pseudonym"),
            price_revision: cid("price"),
            reserve_minor: 100,
            expires_at_ms: 10,
            resource: Symbol::new("tokens"),
            maximum_resource_per_attempt: 5,
            envelope: AttemptEnvelope {
                turns: 2,
                repairs_per_turn: 1,
                smokes: 1,
                retries_per_attempt: 2,
            },
        };
        assert_eq!(seal.envelope.maximum_attempts(), 15);
        assert_eq!(seal.maximum_resource(), 150);
    }

    #[test]
    fn unreliable_cheap_tokens_can_cost_more_per_accepted_result() {
        let cheap = summarize_spend(
            &(0..4)
                .map(|i| {
                    event(&format!("cheap-{i}"), 3, ResourceCause::CandidateRoute).generic_event()
                })
                .collect::<Vec<_>>(),
        );
        let dear =
            summarize_spend(&[event("dear", 8, ResourceCause::CandidateRoute).generic_event()]);
        assert!(
            cost_per_accepted(&cheap, 1).cost_per_accepted
                > cost_per_accepted(&dear, 1).cost_per_accepted
        );
    }

    #[test]
    fn observer_failure_never_condemns_quality_and_censors_cost() {
        let projection = project_failure(FailureEffect::Observer);
        assert_eq!(projection.quality_delta, 0);
        assert!(projection.record_resources && projection.comparative_cost_censored);
    }

    #[test]
    fn balance_delta_and_concurrent_activity_never_enter_ceiling_test() {
        let page = ProviderUsagePage {
            page: 0,
            next_page: None,
            launch_start_ms: 1,
            launch_end_ms: 2,
            exact_model_id: "m".into(),
            billable_response: true,
            request_id: Some("r".into()),
            resource_events: vec![],
            concurrent_unrelated_activity: true,
            balance_delta_minor: Some(9),
        };
        let result = reconcile_provider_usage(&[page], 1, 2, "m");
        assert!(matches!(
            ceiling_evidence(&result),
            Observation::Unavailable
        ));
    }
}
// conformance: reconciled model resource economics.
