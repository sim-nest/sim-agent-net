use super::runner_cache::stable_cache_key;
use sim_kernel::{
    Consistency, Cx, DefaultFactory, EagerPolicy, EvalMode, EvalRequest, Expr, Symbol,
};
use std::sync::Arc;

#[test]
fn stable_cache_key_includes_output_shape() {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    let expr = Expr::Map(vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String("shape scoped".to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(Vec::new()),
        ),
        (Expr::Symbol(Symbol::new("cache")), Expr::Bool(true)),
    ]);
    let shape_one = sim_shape::shape_value(
        Symbol::qualified("test", "shape-one"),
        Arc::new(sim_shape::AnyShape),
    );
    let shape_two = sim_shape::shape_value(
        Symbol::qualified("test", "shape-two"),
        Arc::new(sim_shape::AnyShape),
    );
    let key_one = stable_cache_key(
        &mut cx,
        &eval_request(expr.clone(), Some(shape_one)),
        &Symbol::qualified("runner", "fake"),
        "shape-model",
        None,
    )
    .unwrap();
    let key_two = stable_cache_key(
        &mut cx,
        &eval_request(expr, Some(shape_two)),
        &Symbol::qualified("runner", "fake"),
        "shape-model",
        None,
    )
    .unwrap();
    assert_ne!(key_one, key_two);
}

fn eval_request(expr: Expr, result_shape: Option<sim_kernel::Value>) -> EvalRequest {
    EvalRequest {
        expr,
        mode: EvalMode::Eval,
        result_shape,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        required_capabilities: Vec::new(),
        deadline: None,
        consistency: Consistency::LocalFirst,
        trace: false,
    }
}
