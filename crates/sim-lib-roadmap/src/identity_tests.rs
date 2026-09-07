use std::collections::BTreeMap;

use sim_kernel::{Expr, Symbol};

use crate::{RoadmapValue, RoadmapValueKind};

fn fields(entries: &[(&str, Expr)]) -> BTreeMap<Symbol, Expr> {
    entries
        .iter()
        .map(|(key, value)| (Symbol::new(*key), value.clone()))
        .collect()
}

#[test]
fn roadmap_value_identity_is_a_full_width_tagged_content_id() {
    let value = RoadmapValue::new(
        RoadmapValueKind::Explanation,
        fields(&[
            ("subject", Expr::String("phase/a".into())),
            ("prose", Expr::String("evidence".into())),
        ]),
    )
    .unwrap();
    assert_eq!(
        value.semantic_id().algorithm,
        sim_kernel::datum_content_algorithm()
    );
    assert_eq!(value.semantic_id().bytes.len(), 32);

    let changed = RoadmapValue::new(
        RoadmapValueKind::Explanation,
        fields(&[
            ("subject", Expr::String("phase/a".into())),
            ("prose", Expr::String("different evidence".into())),
        ]),
    )
    .unwrap();
    assert_ne!(value.semantic_id(), changed.semantic_id());
}
