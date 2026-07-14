use super::support::{eval_cx, install_agent_lib, install_roundtrip_codecs};
use sim_kernel::{Args, Expr, Symbol};

#[test]
fn agent_recipe_result_decodes_embedded_recipe_expectation() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let recipe_id = cx
        .factory()
        .string("a30-001-autonomous-decision".to_owned())
        .unwrap();
    let value = cx
        .call_function(
            &Symbol::qualified("agent", "recipe-result"),
            Args::new(vec![recipe_id]),
        )
        .unwrap();
    let expr = value.object().as_expr(&mut cx).unwrap();

    let Expr::List(items) = expr else {
        panic!("expected recipe result list");
    };
    assert_eq!(
        items.first(),
        Some(&Expr::Symbol(Symbol::new("agent-pattern")))
    );
}
