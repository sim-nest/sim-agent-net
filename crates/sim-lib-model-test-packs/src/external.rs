use crate::PackError;
use sha2::{Digest, Sha256};

pub const EXTERNAL_EVALUATOR_SCHEMA: &str = "sim.model-test-external-evaluator/v1";
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalFormat {
    EvalPlus,
    Open(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalCase {
    pub id: String,
    pub input_digest: String,
    pub expected_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalEvaluatorBundle {
    pub schema: String,
    pub format: ExternalFormat,
    pub evaluator_id: String,
    pub cases: Vec<ExternalCase>,
    pub content_digest: String,
    pub isolation_trusted: bool,
}
impl ExternalEvaluatorBundle {
    pub fn seal(
        format: ExternalFormat,
        evaluator_id: String,
        cases: Vec<ExternalCase>,
    ) -> Result<Self, PackError> {
        if evaluator_id.is_empty() || cases.is_empty() {
            return Err(PackError::Missing("external evaluator content"));
        }
        let mut h = Sha256::new();
        h.update(format!("{:?}|{}|{:?}", format, evaluator_id, cases));
        Ok(Self {
            schema: EXTERNAL_EVALUATOR_SCHEMA.into(),
            format,
            evaluator_id,
            cases,
            content_digest: format!("sha256:{:x}", h.finalize()),
            isolation_trusted: false,
        })
    }
    pub fn validate(&self) -> Result<(), PackError> {
        let rebuilt = Self::seal(
            self.format.clone(),
            self.evaluator_id.clone(),
            self.cases.clone(),
        )?;
        if self.schema != EXTERNAL_EVALUATOR_SCHEMA
            || self.isolation_trusted
            || self.content_digest != rebuilt.content_digest
        {
            Err(PackError::ObjectMismatch(
                "external evaluator bundle".into(),
            ))
        } else {
            Ok(())
        }
    }
}
