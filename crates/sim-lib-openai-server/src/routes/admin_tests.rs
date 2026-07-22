use super::*;

#[test]
fn typed_readers_keep_provider_key_policy() {
    let map = Expr::Map(vec![
        field("model", Expr::String("fixture".to_owned())),
        (
            Expr::String("usage".to_owned()),
            Expr::Map(vec![(
                Expr::String("latency-ms".to_owned()),
                Expr::String("12".to_owned()),
            )]),
        ),
        (
            Expr::Symbol(Symbol::qualified("openai", "qualified")),
            Expr::String("qualified".to_owned()),
        ),
    ]);

    assert_eq!(string_value(&map, "model"), Some("fixture"));
    let usage = map_value(&map, "usage").expect("usage map");
    assert_eq!(u64_field(usage, "latency-ms"), Some(12));
    assert_eq!(string_value(&map, "qualified"), None);
}
