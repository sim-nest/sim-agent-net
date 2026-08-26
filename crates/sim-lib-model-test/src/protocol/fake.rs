#[derive(Clone)]
pub struct FakeProtocol {
    metadata: ProtocolMetadata,
    kind: FakeProtocolKind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FakeProtocolKind {
    Scalar,
    Generated,
    JudgedDocument,
    WorkspaceMutation,
}
impl FakeProtocol {
    pub fn new(id: &str, kind: FakeProtocolKind) -> Self {
        Self {
            metadata: ProtocolMetadata {
                id: id.into(),
                domain: "fixture".into(),
                role: "model-under-test".into(),
                family: format!("{kind:?}"),
                revision: "fixture-r1".into(),
            },
            kind,
        }
    }
}
impl ModelTaskProtocol for FakeProtocol {
    fn metadata(&self) -> &ProtocolMetadata {
        &self.metadata
    }
    fn output_shape(&self) -> OutputShape {
        OutputShape {
            id: format!("model-test/{:?}", self.kind),
            description: "terminal UTF-8 response".into(),
        }
    }
    fn effect_requirements(&self) -> BTreeSet<EffectRequirement> {
        if self.kind == FakeProtocolKind::WorkspaceMutation {
            [EffectRequirement {
                capability: "workspace/write".into(),
                reason: "fixture mutation".into(),
            }]
            .into()
        } else {
            BTreeSet::new()
        }
    }
    fn prepare(&self, task: &TaskRevision, seed: u64) -> Result<PreparedTrial, ProtocolError> {
        if task.protocol != self.metadata.id {
            return Err(ProtocolError::WrongProtocol);
        }
        Ok(PreparedTrial {
            task_revision: task.id.clone(),
            public_bytes: task.visible_inputs.clone(),
            seed,
            output_shape: self.output_shape(),
            effects: self.effect_requirements(),
            private_oracle: Arc::from(task.hidden_oracle_id.as_bytes()),
        })
    }
    fn grade(&self, p: &PreparedTrial, r: &TerminalResponse) -> Vec<FacetObservation> {
        let text = String::from_utf8_lossy(&r.bytes);
        let expected = String::from_utf8_lossy(&p.public_bytes);
        let passed = match self.kind {
            FakeProtocolKind::Scalar => {
                text.trim().parse::<i64>().ok() == expected.trim().parse::<i64>().ok()
            }
            FakeProtocolKind::Generated => r.tool_receipts.iter().any(|x| x == &p.task_revision),
            FakeProtocolKind::JudgedDocument => {
                text.contains("therefore") && !text.contains("keyword-only")
            }
            FakeProtocolKind::WorkspaceMutation => {
                r.tool_receipts.iter().any(|x| x.starts_with("sha256:"))
            }
        };
        vec![FacetObservation {
            facet: format!("{:?}", self.kind),
            score: if passed { 1.0 } else { 0.0 },
            passed,
            reason: if passed {
                "semantic contract satisfied"
            } else {
                "semantic contract failed"
            }
            .into(),
            evidence_class: EvidenceClass::Deterministic,
            provenance: digest([p.task_revision.as_bytes(), &r.bytes]),
        }]
    }
}
