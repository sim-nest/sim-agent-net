use super::support::{eval_cx, install_agent_lib, install_roundtrip_codecs};
use sim_kernel::{
    Args, Error, Expr, Symbol, Value, macro_expand_eval_capability, read_eval_capability,
};

const SEEDED_LISP_RECIPE: &str = "codec/lisp/01-basics/quote-symbol";

#[test]
fn agent_cookbook_card_can_search_and_run_seeded_recipe() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    assert!(cx.registry().lib(&sim_lib_core::manifest_name()).is_some());

    let card = cx
        .resolve_value(&Symbol::qualified("agent", "cookbook"))
        .unwrap();
    let card = expr(&mut cx, &card);
    assert_eq!(
        table_value(&card, "search"),
        Some(&Expr::Symbol(Symbol::qualified("agent", "cookbook-search")))
    );
    assert_eq!(
        table_value(&card, "run"),
        Some(&Expr::Symbol(Symbol::qualified("agent", "cookbook-run")))
    );

    let search = call_tool(&mut cx, "search", "quote").unwrap();
    let search = expr(&mut cx, &search);
    assert_list_contains_string(&search, SEEDED_LISP_RECIPE);

    let err = call_tool(&mut cx, "run", SEEDED_LISP_RECIPE).unwrap_err();
    assert!(
        matches!(err, Error::CapabilityDenied { ref capability } if capability == &read_eval_capability()),
        "{err:?}"
    );

    cx.grant(read_eval_capability());
    cx.grant(macro_expand_eval_capability());
    let run = call_tool(&mut cx, "run", SEEDED_LISP_RECIPE).unwrap();
    let run = expr(&mut cx, &run);
    assert_eq!(
        table_value(&run, "recipe"),
        Some(&Expr::String(SEEDED_LISP_RECIPE.to_owned()))
    );
    assert_eq!(table_value(&run, "ok"), Some(&Expr::Bool(true)));
}

fn call_tool(cx: &mut sim_kernel::Cx, name: &str, arg: &str) -> sim_kernel::Result<Value> {
    let target = cx
        .factory()
        .symbol(Symbol::qualified("agent", format!("cookbook-{name}")))?;
    let arg = cx.factory().string(arg.to_owned())?;
    cx.call_function(
        &Symbol::qualified("agent", "call-tool"),
        Args::new(vec![target, arg]),
    )
}

fn expr(cx: &mut sim_kernel::Cx, value: &Value) -> Expr {
    value.object().as_expr(cx).unwrap()
}

fn table_value<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(key, value)| {
        let Expr::Symbol(symbol) = key else {
            return None;
        };
        (symbol == &Symbol::new(name)).then_some(value)
    })
}

fn assert_list_contains_string(expr: &Expr, expected: &str) {
    let Expr::List(items) = expr else {
        panic!("expected string list");
    };
    assert!(
        items
            .iter()
            .any(|item| matches!(item, Expr::String(value) if value == expected)),
        "{expected} missing from {items:?}"
    );
}
