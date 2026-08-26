/// Optional learned reranking request; candidates are fenced by content id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JudgeRequest {
    pub model_site_id: String,
    pub policy_revision: String,
    pub input_id: ContentId,
    pub fenced_candidates: String,
}
/// Full judge result; invalid output records failure and preserves RRF order.
#[derive(Clone, Debug, PartialEq)]
pub struct JudgeReceipt {
    pub model_site_id: String,
    pub policy_revision: String,
    pub input_id: ContentId,
    pub output_id: Option<ContentId>,
    pub deltas: BTreeMap<String, f64>,
    pub failure: Option<String>,
}
pub trait Judge: Send + Sync {
    fn judge(&self, request: &JudgeRequest) -> Result<(ContentId, BTreeMap<String, f64>), String>;
}
pub fn call_judge(
    judge: &dyn Judge,
    model_site_id: &str,
    policy_revision: &str,
    candidates: &[String],
) -> JudgeReceipt {
    let datum = Datum::Vector(candidates.iter().cloned().map(Datum::String).collect());
    let input_id = datum
        .content_id()
        .expect("bounded strings are content-addressable");
    let fenced = fenced_data_text_for_id("search-candidates", &candidates.join("\n"), &input_id);
    let request = JudgeRequest {
        model_site_id: model_site_id.into(),
        policy_revision: policy_revision.into(),
        input_id: input_id.clone(),
        fenced_candidates: fenced,
    };
    match judge.judge(&request) {
        Ok((output_id, deltas)) if deltas.values().all(|v| v.is_finite()) => JudgeReceipt {
            model_site_id: model_site_id.into(),
            policy_revision: policy_revision.into(),
            input_id,
            output_id: Some(output_id),
            deltas,
            failure: None,
        },
        Ok(_) => JudgeReceipt {
            model_site_id: model_site_id.into(),
            policy_revision: policy_revision.into(),
            input_id,
            output_id: None,
            deltas: BTreeMap::new(),
            failure: Some("invalid judge output".into()),
        },
        Err(e) => JudgeReceipt {
            model_site_id: model_site_id.into(),
            policy_revision: policy_revision.into(),
            input_id,
            output_id: None,
            deltas: BTreeMap::new(),
            failure: Some(e),
        },
    }
}
