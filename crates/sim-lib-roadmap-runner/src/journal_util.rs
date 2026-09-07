use sim_kernel::{ContentId, Datum, Symbol};
use sim_lib_journal::StoreError;

use crate::{ExecutionJournalError, ExecutionPins};

pub(crate) fn store_error(error: StoreError) -> ExecutionJournalError {
    match error {
        StoreError::Journal(error) => error.into(),
    }
}
pub(crate) fn contains_secret(bytes: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    [
        "api_key=",
        "api-key:",
        "authorization: bearer ",
        "password=",
        "private key-----",
        "secret=",
    ]
    .iter()
    .any(|p| lower.contains(p))
}
pub(crate) fn bounded_summary(value: &str) -> String {
    value.chars().take(240).collect()
}
pub(crate) fn child_id(parent: &str, pins: &ExecutionPins) -> String {
    let identity = Datum::Node {
        tag: Symbol::qualified("roadmap-runner", "ChildExecutionIdentityV2"),
        fields: vec![
            (Symbol::new("parent"), Datum::String(parent.to_owned())),
            (Symbol::new("conduct"), Datum::String(pins.conduct.clone())),
            (Symbol::new("policy"), Datum::String(pins.policy.clone())),
            (
                Symbol::new("model-pick"),
                Datum::String(pins.model_pick.clone()),
            ),
            (
                Symbol::new("runner-generation"),
                Datum::String(pins.runner_generation.clone()),
            ),
            (
                Symbol::new("source-deck"),
                content_id_datum(&pins.source_deck),
            ),
        ],
    }
    .content_id()
    .expect("child execution identity has a fixed canonical shape");
    format!("{parent}-child-{}", content_id_text(&identity))
}

fn content_id_datum(id: &ContentId) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("core", "ContentId"),
        fields: vec![
            (
                Symbol::new("algorithm"),
                Datum::Symbol(id.algorithm.clone()),
            ),
            (Symbol::new("bytes"), Datum::Bytes(id.bytes.to_vec())),
        ],
    }
}

fn content_id_text(id: &ContentId) -> String {
    let bytes = id
        .bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{}:{bytes}", id.algorithm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pins() -> ExecutionPins {
        ExecutionPins {
            conduct: "conduct".into(),
            policy: "policy".into(),
            source_deck: ContentId::from_bytes(Symbol::qualified("deck", "sha256-v1"), [1; 32]),
            model_pick: "model".into(),
            runner_generation: "runner".into(),
        }
    }

    #[test]
    fn child_identity_covers_parent_and_every_execution_pin() {
        let baseline = pins();
        let expected = child_id("parent", &baseline);
        assert!(expected.contains("core/sha256-datum-v1:"));
        assert_ne!(child_id("other-parent", &baseline), expected);

        let mut variants = Vec::new();
        let mut value = baseline.clone();
        value.conduct = "other-conduct".into();
        variants.push(value);
        let mut value = baseline.clone();
        value.policy = "other-policy".into();
        variants.push(value);
        let mut value = baseline.clone();
        value.model_pick = "other-model".into();
        variants.push(value);
        let mut value = baseline.clone();
        value.runner_generation = "other-runner".into();
        variants.push(value);
        let mut value = baseline.clone();
        value.source_deck = ContentId::from_bytes(Symbol::qualified("deck", "sha256-v1"), [2; 32]);
        variants.push(value);
        let mut value = baseline;
        value.source_deck = ContentId::from_bytes(Symbol::qualified("deck", "other-v1"), [1; 32]);
        variants.push(value);

        for value in variants {
            assert_ne!(child_id("parent", &value), expected);
        }
    }
}
