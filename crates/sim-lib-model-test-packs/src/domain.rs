use crate::PackError;
use sim_study_core::EvidenceClass;
use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_PORTFOLIO_SCHEMA: &str = "sim.model-test-domain-portfolio/v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainMetadata {
    pub id: String,
    pub purpose: String,
    pub facets: Vec<String>,
    pub eligibility: Vec<String>,
    pub pack_ids: Vec<String>,
    pub evidence_class: EvidenceClass,
    pub decision_specs: Vec<String>,
}

impl DomainMetadata {
    pub fn validate(&self) -> Result<(), PackError> {
        if self.id.trim().is_empty() || self.purpose.trim().is_empty() {
            return Err(PackError::Missing("domain id or purpose"));
        }
        for (values, name) in [
            (&self.facets, "domain facets"),
            (&self.eligibility, "domain eligibility"),
            (&self.pack_ids, "domain pack ids"),
            (&self.decision_specs, "domain decision specs"),
        ] {
            if values.is_empty() || values.iter().any(|value| value.trim().is_empty()) {
                return Err(PackError::Missing(name));
            }
            if values.iter().collect::<BTreeSet<_>>().len() != values.len() {
                return Err(PackError::Duplicate(name));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainPortfolio {
    pub schema: String,
    pub revision: String,
    pub domains: Vec<DomainMetadata>,
}

impl DomainPortfolio {
    pub fn validate(&self) -> Result<(), PackError> {
        if self.schema != DOMAIN_PORTFOLIO_SCHEMA || self.revision.trim().is_empty() {
            return Err(PackError::Schema);
        }
        let mut ids = BTreeSet::new();
        for domain in &self.domains {
            domain.validate()?;
            if !ids.insert(&domain.id) {
                return Err(PackError::Duplicate("domain id"));
            }
        }
        if self.domains.is_empty() {
            Err(PackError::Missing("domains"))
        } else {
            Ok(())
        }
    }

    pub fn by_id(&self) -> BTreeMap<&str, &DomainMetadata> {
        self.domains
            .iter()
            .map(|domain| (domain.id.as_str(), domain))
            .collect()
    }
}

pub trait DomainPortfolioLoader {
    /// The control plane selects private domain metadata; public dispatch stays open.
    fn load_active_portfolio(&self) -> Result<DomainPortfolio, PackError>;
}

/// Bounded public starter metadata. Domain behavior remains in installed packs.
pub fn public_starter_domains() -> Vec<DomainMetadata> {
    [
        (
            "structured-output-tool-use",
            "schema construction and bounded tool selection",
            ["structured-output", "tool-use"],
        ),
        (
            "sim-language-codec",
            "reason over installed SIM language and codec values",
            ["language-values", "codec-round-trip"],
        ),
        (
            "symbolic-numeric-mathematics",
            "derive exact symbolic and checked numeric results",
            ["symbolic-tree", "numeric-check"],
        ),
        (
            "simulation-modelling",
            "construct and diagnose bounded state models",
            ["state-trace", "causal-debug"],
        ),
        (
            "music-audio-structure",
            "reason over exact score and audio structure",
            ["score-structure", "signal-structure"],
        ),
        (
            "technical-documentation",
            "produce grounded technical explanations and references",
            ["explanation", "reference-check"],
        ),
    ]
    .into_iter()
    .map(|(id, purpose, families)| DomainMetadata {
        id: id.into(),
        purpose: purpose.into(),
        facets: vec!["correctness".into(), "format".into(), "resource".into()],
        eligibility: vec!["offline-reproducible".into(), "independent-oracle".into()],
        pack_ids: families
            .into_iter()
            .map(|family| format!("sim.model-test/{id}/{family}/v1"))
            .collect(),
        evidence_class: EvidenceClass::Publishable,
        decision_specs: vec!["capability-threshold/v1".into()],
    })
    .collect()
}
