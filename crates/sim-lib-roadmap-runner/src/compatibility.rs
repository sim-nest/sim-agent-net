//! Sanitized semantic records for shadow qualification.
// conformance: sanitized roadmap shadow compatibility records.
//!
//! These values deliberately contain no oracle reader, journal adapter, filesystem
//! path, or transition authority. A private host may compare them with an oracle;
//! the public runner can only describe what it observed and propose a transition.

use sim_kernel::{ContentId, Datum, Symbol};

/// The dimensions compared by shadow qualification. Explanatory prose is not a
/// dimension: only its sanitized content identity is compared.
pub const SHADOW_DIMENSIONS: [&str; 10] = [
    "selection-input",
    "source-scope",
    "authority-ceiling",
    "refinement-decision",
    "mutation-proposal",
    "effect-intent",
    "proof-classification",
    "transition-proposal",
    "stop-class",
    "sanitized-explanation",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibilityClass {
    Passed,
    IntentionallyChanged,
    Unsupported,
    Failed,
}

impl CompatibilityClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::IntentionallyChanged => "intentionally-changed",
            Self::Unsupported => "unsupported",
            Self::Failed => "failed",
        }
    }
}

/// Authority-free output of one observe or disposable mutation case.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShadowObservation {
    pub case_id: String,
    pub category: String,
    pub dimensions: [String; 10],
    pub classification: CompatibilityClass,
    pub evidence_refs: Vec<String>,
}

impl ShadowObservation {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.case_id.is_empty() || self.category.is_empty() {
            return Err("case identity is empty");
        }
        if self.dimensions.iter().any(String::is_empty) {
            return Err("comparison dimension is empty");
        }
        if self.evidence_refs.is_empty() || self.evidence_refs.iter().any(String::is_empty) {
            return Err("evidence reference is empty");
        }
        for value in self.dimensions.iter().chain(self.evidence_refs.iter()) {
            if value.contains('\n')
                || value.contains('\r')
                || value.contains("/home/")
                || value.contains(".meta-workspace")
                || value.contains("roadmap-3.sqlite")
            {
                return Err("unsanitized shadow value");
            }
        }
        Ok(())
    }

    /// Stable, line-oriented interchange for private comparison orchestration.
    pub fn canonical(&self) -> Result<String, &'static str> {
        self.validate()?;
        let mut lines = vec![
            "schema=sim.roadmap-shadow-observation/v1".to_owned(),
            format!("case={}", self.case_id),
            format!("category={}", self.category),
            format!("classification={}", self.classification.as_str()),
        ];
        for (name, value) in SHADOW_DIMENSIONS.iter().zip(&self.dimensions) {
            lines.push(format!("dimension.{name}={value}"));
        }
        for evidence in &self.evidence_refs {
            lines.push(format!("evidence={evidence}"));
        }
        Ok(lines.join("\n") + "\n")
    }

    pub fn content_id(&self) -> Result<ContentId, &'static str> {
        self.validate()?;
        Datum::Node {
            tag: Symbol::qualified("roadmap-runner", "ShadowObservationIdentityV2"),
            fields: vec![
                (Symbol::new("case-id"), Datum::String(self.case_id.clone())),
                (
                    Symbol::new("category"),
                    Datum::String(self.category.clone()),
                ),
                (
                    Symbol::new("classification"),
                    Datum::Symbol(Symbol::qualified(
                        "shadow-classification",
                        self.classification.as_str(),
                    )),
                ),
                (
                    Symbol::new("dimensions"),
                    Datum::List(
                        SHADOW_DIMENSIONS
                            .iter()
                            .zip(&self.dimensions)
                            .map(|(name, value)| Datum::Node {
                                tag: Symbol::qualified("roadmap-runner", "ShadowDimensionV1"),
                                fields: vec![
                                    (Symbol::new("name"), Datum::String((*name).to_owned())),
                                    (Symbol::new("value"), Datum::String(value.clone())),
                                ],
                            })
                            .collect(),
                    ),
                ),
                (
                    Symbol::new("evidence-refs"),
                    Datum::Set(
                        self.evidence_refs
                            .iter()
                            .cloned()
                            .map(Datum::String)
                            .collect(),
                    ),
                ),
            ],
        }
        .content_id()
        .map_err(|_| "shadow observation is not canonical")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn specimen() -> ShadowObservation {
        ShadowObservation {
            case_id: "oracle/plan-revisions".into(),
            category: "selection".into(),
            dimensions: std::array::from_fn(|i| format!("value-{i}")),
            classification: CompatibilityClass::Passed,
            evidence_refs: vec!["content:oracle-plan-revisions".into()],
        }
    }
    #[test]
    fn canonical_is_stable_and_dimension_complete() {
        let value = specimen();
        let rendered = value.canonical().unwrap();
        for dimension in SHADOW_DIMENSIONS {
            assert!(rendered.contains(&format!("dimension.{dimension}=")));
        }
        assert_eq!(value.content_id(), value.content_id());
        let identity = value.content_id().unwrap();
        assert_eq!(identity.algorithm, sim_kernel::datum_content_algorithm());
        assert_eq!(identity.bytes.len(), 32);
    }
    #[test]
    fn identity_covers_every_semantic_field_and_treats_evidence_as_a_set() {
        let baseline = specimen();
        let expected = baseline.content_id().unwrap();
        let mut variants = Vec::new();
        let mut value = baseline.clone();
        value.case_id = "other-case".into();
        variants.push(value);
        let mut value = baseline.clone();
        value.category = "other-category".into();
        variants.push(value);
        let mut value = baseline.clone();
        value.classification = CompatibilityClass::IntentionallyChanged;
        variants.push(value);
        for index in 0..SHADOW_DIMENSIONS.len() {
            let mut value = baseline.clone();
            value.dimensions[index] = format!("other-{index}");
            variants.push(value);
        }
        let mut value = baseline.clone();
        value.evidence_refs.push("content:other".into());
        variants.push(value);
        for value in variants {
            assert_ne!(value.content_id().unwrap(), expected);
        }

        let mut reordered = baseline;
        reordered.evidence_refs.push("content:second".into());
        let expected = reordered.content_id().unwrap();
        reordered.evidence_refs.reverse();
        assert_eq!(reordered.content_id().unwrap(), expected);
    }
    #[test]
    fn rejects_private_or_control_plane_values() {
        for forbidden in [
            "/home/person/key",
            ".meta-workspace/Cargo.toml",
            "roadmap-3.sqlite",
        ] {
            let mut value = specimen();
            value.dimensions[0] = forbidden.into();
            assert_eq!(value.validate(), Err("unsanitized shadow value"));
        }
    }
}
