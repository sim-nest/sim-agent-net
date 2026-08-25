//! Sanitized semantic records for shadow qualification.
//!
//! These values deliberately contain no oracle reader, journal adapter, filesystem
//! path, or transition authority. A private host may compare them with an oracle;
//! the public runner can only describe what it observed and propose a transition.

use sha2::{Digest, Sha256};

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

    pub fn content_id(&self) -> Result<String, &'static str> {
        Ok(format!(
            "sha256:{:x}",
            Sha256::digest(self.canonical()?.as_bytes())
        ))
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
