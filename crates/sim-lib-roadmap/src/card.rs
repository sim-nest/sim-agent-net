use crate::{RoadmapValue, RoadmapValueKind};
use sim_kernel::{Cx, Expr, Result, Symbol, Value};

/// Build the bounded browse/Card face for a roadmap value.
pub fn roadmap_card(cx: &mut Cx, value: &RoadmapValue) -> Result<Value> {
    let fields = value.fields();
    let summary = card_summary(value.kind(), fields);
    cx.factory().table(vec![
        (
            Symbol::new("kind"),
            cx.factory()
                .symbol(Symbol::qualified("roadmap", value.kind().wire_name()))?,
        ),
        (
            Symbol::new("semantic-id"),
            cx.factory().string(value.semantic_id().into())?,
        ),
        (Symbol::new("summary"), cx.factory().string(summary)?),
        (
            Symbol::new("field-count"),
            cx.factory().string(fields.len().to_string())?,
        ),
        (Symbol::new("shape-known"), cx.factory().bool(true)?),
    ])
}
fn card_summary(
    kind: RoadmapValueKind,
    fields: &std::collections::BTreeMap<Symbol, Expr>,
) -> String {
    for name in ["title", "prose", "subject", "id"] {
        if let Some(Expr::String(text)) = fields.get(&Symbol::new(name)) {
            return text.chars().take(240).collect();
        }
    }
    format!("{} with {} fields", kind.wire_name(), fields.len())
}
