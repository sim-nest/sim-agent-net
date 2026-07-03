//! Content-addressed identities for distributed eval requests.

use std::cmp::Ordering;

use sim_kernel::{CanonicalKey, ContentId, Datum, EvalRequest, NumberLiteral, Symbol};

/// Content-addressed identity for an [`EvalRequest`].
///
/// A `ContentKey` wraps a small [`Datum`] that records the content id of the
/// canonical request datum. The canonical request includes the expression,
/// required capabilities sorted lexicographically, consistency, mode, answer
/// limit, stream buffer, stream flag, and trace flag.
///
/// Excluded fields are deliberate: `deadline` is a scheduling bound, not part
/// of the requested work, and `result_shape` is currently a process-local
/// runtime value rather than stable content-addressed data.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContentKey(Datum);

impl ContentKey {
    /// Derives a deterministic content key from the stable fields of `request`.
    pub fn from_request(request: &EvalRequest) -> Self {
        let id = request_content_datum(request)
            .content_id()
            .expect("content key request datum is canonical");
        Self(content_id_datum(id))
    }

    /// Returns the datum representation of this key.
    pub fn datum(&self) -> &Datum {
        &self.0
    }

    /// Consumes this key and returns its datum representation.
    pub fn into_datum(self) -> Datum {
        self.0
    }
}

impl PartialOrd for ContentKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ContentKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.canonical_bytes().cmp(&other.canonical_bytes())
    }
}

impl ContentKey {
    fn canonical_bytes(&self) -> Vec<u8> {
        self.0
            .canonical_bytes()
            .expect("content key datum is canonical")
    }
}

fn request_content_datum(request: &EvalRequest) -> Datum {
    let mut capabilities = request.required_capabilities.to_vec();
    capabilities.sort();

    Datum::Node {
        tag: Symbol::qualified("fabric", "eval-request-content-key-v1"),
        fields: vec![
            field("expr", canonical_key_datum(&request.expr.canonical_key())),
            field(
                "required-capabilities",
                Datum::Vector(
                    capabilities
                        .iter()
                        .map(|capability| Datum::String(capability.as_str().to_owned()))
                        .collect(),
                ),
            ),
            field(
                "consistency",
                Datum::Symbol(request.consistency.as_symbol()),
            ),
            field("mode", Datum::Symbol(request.mode.as_symbol())),
            field("answer-limit", optional_usize_datum(request.answer_limit)),
            field("stream-buffer", optional_usize_datum(request.stream_buffer)),
            field("stream", Datum::Bool(request.stream)),
            field("trace", Datum::Bool(request.trace)),
        ],
    }
}

fn content_id_datum(id: ContentId) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("fabric", "content-key"),
        fields: vec![
            field("algorithm", Datum::Symbol(id.algorithm)),
            field("digest", Datum::Bytes(id.bytes.to_vec())),
        ],
    }
}

fn canonical_key_datum(key: &CanonicalKey) -> Datum {
    match key {
        CanonicalKey::Atom(tag) => tagged_node("atom", tag, Vec::new()),
        CanonicalKey::Bool(tag, value) => {
            tagged_node("bool", tag, vec![field("value", Datum::Bool(*value))])
        }
        CanonicalKey::Bytes(tag, bytes) => tagged_node(
            "bytes",
            tag,
            vec![field("value", Datum::Bytes(bytes.clone()))],
        ),
        CanonicalKey::String(tag, value) => tagged_node(
            "string",
            tag,
            vec![field("value", Datum::String(value.clone()))],
        ),
        CanonicalKey::Symbol(tag, symbol) => tagged_node(
            "symbol",
            tag,
            vec![field("value", Datum::Symbol(symbol.clone()))],
        ),
        CanonicalKey::Pair(tag, left, right) => tagged_node(
            "pair",
            tag,
            vec![
                field("left", Datum::String(left.clone())),
                field("right", Datum::String(right.clone())),
            ],
        ),
        CanonicalKey::Compound(tag, items) => tagged_node(
            "compound",
            tag,
            vec![field(
                "items",
                Datum::Vector(items.iter().map(canonical_key_datum).collect()),
            )],
        ),
        CanonicalKey::CompoundNamed(tag, items) => tagged_node(
            "compound-named",
            tag,
            vec![field(
                "items",
                Datum::Vector(
                    items
                        .iter()
                        .map(|(name, value)| Datum::Node {
                            tag: Symbol::qualified("fabric", "canonical-key-item"),
                            fields: vec![
                                field("name", Datum::String(name.clone())),
                                field("value", canonical_key_datum(value)),
                            ],
                        })
                        .collect(),
                ),
            )],
        ),
    }
}

fn tagged_node(kind: &str, tag: &str, mut fields: Vec<(Symbol, Datum)>) -> Datum {
    fields.insert(0, field("tag", Datum::String(tag.to_owned())));
    Datum::Node {
        tag: Symbol::qualified("fabric", format!("canonical-key-{kind}")),
        fields,
    }
}

fn optional_usize_datum(value: Option<usize>) -> Datum {
    value.map_or(Datum::Nil, usize_datum)
}

fn usize_datum(value: usize) -> Datum {
    Datum::Number(NumberLiteral {
        domain: Symbol::qualified("core", "usize"),
        canonical: value.to_string(),
    })
}

fn field(name: &str, value: Datum) -> (Symbol, Datum) {
    (Symbol::new(name), value)
}

#[cfg(test)]
mod tests {
    use sim_kernel::{CapabilityName, Consistency, EvalMode, EvalRequest, Expr, Symbol};

    use super::ContentKey;

    fn req(expr: &str, caps: &[&str]) -> EvalRequest {
        EvalRequest {
            expr: Expr::String(expr.to_owned()),
            result_shape: None,
            required_capabilities: caps
                .iter()
                .map(|capability| CapabilityName::new(*capability))
                .collect(),
            deadline: None,
            consistency: Consistency::LocalFirst,
            mode: EvalMode::Eval,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            trace: false,
        }
    }

    #[test]
    fn identical_requests_produce_equal_content_keys() {
        assert_eq!(
            ContentKey::from_request(&req("hello", &["a", "b"])),
            ContentKey::from_request(&req("hello", &["a", "b"])),
        );
    }

    #[test]
    fn different_expressions_produce_distinct_keys() {
        assert_ne!(
            ContentKey::from_request(&req("hello", &["a"])),
            ContentKey::from_request(&req("world", &["a"])),
        );
    }

    #[test]
    fn capability_order_does_not_affect_key() {
        assert_eq!(
            ContentKey::from_request(&req("e", &["b", "a"])),
            ContentKey::from_request(&req("e", &["a", "b"])),
        );
    }

    fn model_req(task: &str, model: &str, temperature: &str) -> EvalRequest {
        EvalRequest {
            expr: Expr::Map(vec![
                (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
                (
                    Expr::Symbol(Symbol::new("task")),
                    Expr::String(task.to_owned()),
                ),
                (
                    Expr::Symbol(Symbol::new("messages")),
                    Expr::List(Vec::new()),
                ),
                (
                    Expr::Symbol(Symbol::new("model")),
                    Expr::String(model.to_owned()),
                ),
                (
                    Expr::Symbol(Symbol::new("temperature")),
                    Expr::Number(sim_kernel::NumberLiteral {
                        domain: Symbol::qualified("numbers", "f64"),
                        canonical: temperature.to_owned(),
                    }),
                ),
            ]),
            result_shape: None,
            required_capabilities: Vec::new(),
            deadline: None,
            consistency: Consistency::LocalFirst,
            mode: EvalMode::Eval,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            trace: false,
        }
    }

    #[test]
    fn model_id_change_changes_content_key() {
        assert_ne!(
            ContentKey::from_request(&model_req("summarize x", "local/qwen3.5:4b", "0.1")),
            ContentKey::from_request(&model_req("summarize x", "local/qwen3.6:35b", "0.1")),
        );
    }

    #[test]
    fn model_param_change_changes_content_key() {
        assert_ne!(
            ContentKey::from_request(&model_req("summarize x", "local/qwen3.5:4b", "0.1")),
            ContentKey::from_request(&model_req("summarize x", "local/qwen3.5:4b", "0.9")),
        );
    }

    #[test]
    fn identical_model_request_reproduces_content_key() {
        assert_eq!(
            ContentKey::from_request(&model_req("summarize x", "local/qwen3.5:4b", "0.1")),
            ContentKey::from_request(&model_req("summarize x", "local/qwen3.5:4b", "0.1")),
        );
    }
}
