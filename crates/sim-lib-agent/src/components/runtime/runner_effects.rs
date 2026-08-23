use super::runner_cache::normalize_expr;
use sim_kernel::{
    ContentId, Cx, Datum, DatumStore, Effect, EvalRequest, Expr, Ref, Result, Symbol, core_any_ref,
    effect, value_from_ref,
};
use sim_lib_agent_runner_core::{ModelEventSink, ModelResponse};

fn model_infer_effect_kind() -> Symbol {
    Symbol::qualified("effect", "model-infer")
}

pub(super) struct ModelInferTarget<'a> {
    runner: &'a Symbol,
    model: &'a str,
    spec: &'a [(Symbol, Expr)],
}

impl<'a> ModelInferTarget<'a> {
    pub(super) fn new(runner: &'a Symbol, model: &'a str, spec: &'a [(Symbol, Expr)]) -> Self {
        Self {
            runner,
            model,
            spec,
        }
    }
}

pub(super) fn resolve_model_infer_effect<F>(
    cx: &mut Cx,
    target: &ModelInferTarget<'_>,
    request: &EvalRequest,
    infer: F,
) -> Result<Expr>
where
    F: FnOnce(&mut Cx) -> Result<Expr>,
{
    let effect = model_infer_effect(cx, target, request)?;
    let result = effect::resolve_effect(cx, effect, |cx, _effect| {
        let response = infer(cx)?;
        response_ref(cx, response)
    })?;
    ref_to_expr(cx, &result)
}

pub(super) fn resolve_model_infer_stream_effect<F>(
    cx: &mut Cx,
    target: &ModelInferTarget<'_>,
    request: &EvalRequest,
    events: &mut dyn ModelEventSink,
    infer: F,
) -> Result<ModelResponse>
where
    F: FnOnce(&mut Cx, &mut dyn ModelEventSink) -> Result<ModelResponse>,
{
    let effect = model_infer_effect(cx, target, request)?;
    let result = effect::resolve_effect(cx, effect, |cx, _effect| {
        let response = infer(cx, events)?;
        response_ref(cx, Expr::from(response))
    })?;
    ModelResponse::try_from(ref_to_expr(cx, &result)?)
}

pub(super) fn model_infer_replay_key(
    cx: &mut Cx,
    request: &Expr,
    result_shape: Option<&Expr>,
) -> Result<ContentId> {
    let mut effect = model_infer_effect_for_expr(cx, None, request, result_shape)?;
    effect.ensure_replay_key(Some(model_infer_implementation()))
}

#[cfg(test)]
fn model_infer_replay_key_for_target(
    cx: &mut Cx,
    target: &ModelInferTarget<'_>,
    request: &Expr,
    result_shape: Option<&Expr>,
) -> Result<ContentId> {
    let mut effect = model_infer_effect_for_expr(cx, Some(target), request, result_shape)?;
    effect.ensure_replay_key(Some(model_infer_implementation()))
}

fn model_infer_effect(
    cx: &mut Cx,
    target: &ModelInferTarget<'_>,
    request: &EvalRequest,
) -> Result<Effect> {
    let result_shape = request
        .result_shape
        .as_ref()
        .map(|shape| shape.object().as_expr(cx))
        .transpose()?;
    model_infer_effect_for_expr(cx, Some(target), &request.expr, result_shape.as_ref())
}

fn model_infer_effect_for_expr(
    cx: &mut Cx,
    target: Option<&ModelInferTarget<'_>>,
    request: &Expr,
    result_shape: Option<&Expr>,
) -> Result<Effect> {
    let target = target
        .map(model_infer_target_expr)
        .map(|expr| normalize_expr(&expr));
    let request = normalize_expr(request);
    let result_shape = result_shape.map(normalize_expr);
    let input = model_infer_input_datum(target.as_ref(), &request, result_shape.as_ref());
    let input = Ref::Content(cx.datum_store_mut().intern(input)?);
    let result_shape = match result_shape {
        Some(expr) => Ref::Content(cx.datum_store_mut().intern(expr_replay_datum(&expr))?),
        None => core_any_ref(),
    };
    Effect::new(
        cx.fresh_handle(),
        model_infer_effect_kind(),
        Ref::Symbol(Symbol::qualified("agent", "model-infer")),
        input,
        result_shape,
        effect::effect_resume_op_key(),
        effect::effect_abort_op_key(),
    )
    .with_replay_key(Some(model_infer_implementation()))
}

fn model_infer_input_datum(
    target: Option<&Expr>,
    request: &Expr,
    result_shape: Option<&Expr>,
) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("agent", "ModelInferInput"),
        fields: vec![
            (
                Symbol::new("version"),
                Datum::String("sim-agent-model-infer-v1".to_owned()),
            ),
            (
                Symbol::new("target"),
                match target {
                    Some(expr) => expr_replay_datum(expr),
                    None => Datum::Nil,
                },
            ),
            (Symbol::new("request"), expr_replay_datum(request)),
            (
                Symbol::new("result-shape"),
                match result_shape {
                    Some(expr) => expr_replay_datum(expr),
                    None => Datum::Nil,
                },
            ),
        ],
    }
}

fn model_infer_target_expr(target: &ModelInferTarget<'_>) -> Expr {
    Expr::Map(vec![
        entry("runner", Expr::Symbol(target.runner.clone())),
        entry("model", Expr::String(target.model.to_owned())),
        entry(
            "spec",
            Expr::List(
                target
                    .spec
                    .iter()
                    .map(|(key, value)| {
                        Expr::Map(vec![
                            entry("key", Expr::Symbol(key.clone())),
                            entry("value", value.clone()),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn expr_replay_datum(expr: &Expr) -> Datum {
    Datum::try_from(expr.clone()).unwrap_or_else(|_| Datum::Node {
        tag: Symbol::qualified("agent", "ExprDebugReplay"),
        fields: vec![(Symbol::new("debug"), Datum::String(format!("{expr:?}")))],
    })
}

fn response_ref(cx: &mut Cx, response: Expr) -> Result<Ref> {
    Ok(Ref::Content(
        cx.datum_store_mut().intern(Datum::try_from(response)?)?,
    ))
}

fn ref_to_expr(cx: &mut Cx, reference: &Ref) -> Result<Expr> {
    value_from_ref(cx, reference)?.object().as_expr(cx)
}

fn model_infer_implementation() -> Ref {
    Ref::Symbol(Symbol::qualified("agent", "model-infer-v1"))
}

fn entry(key: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(key)), value)
}

#[cfg(test)]
mod tests {
    use super::*;

    use sim_kernel::testing::bare_cx as cx;

    #[test]
    fn model_infer_replay_key_is_stable_for_reordered_maps() {
        let mut cx = cx();
        let left = Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("b")),
                Expr::String("two".to_owned()),
            ),
            (
                Expr::Symbol(Symbol::new("a")),
                Expr::String("one".to_owned()),
            ),
        ]);
        let right = Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("a")),
                Expr::String("one".to_owned()),
            ),
            (
                Expr::Symbol(Symbol::new("b")),
                Expr::String("two".to_owned()),
            ),
        ]);

        assert_eq!(
            model_infer_replay_key(&mut cx, &left, None).unwrap(),
            model_infer_replay_key(&mut cx, &right, None).unwrap()
        );
    }

    #[test]
    fn model_infer_replay_key_includes_target_metadata() {
        let mut cx = cx();
        let request = Expr::String("task".to_owned());
        let left_runner = Symbol::qualified("runner", "left");
        let right_runner = Symbol::qualified("runner", "right");
        let left_spec = vec![(
            Symbol::new("endpoint"),
            Expr::String("http://left".to_owned()),
        )];
        let right_spec = vec![(
            Symbol::new("endpoint"),
            Expr::String("http://right".to_owned()),
        )];
        let left = ModelInferTarget::new(&left_runner, "same/model", &left_spec);
        let right = ModelInferTarget::new(&right_runner, "same/model", &right_spec);

        assert_ne!(
            model_infer_replay_key_for_target(&mut cx, &left, &request, None).unwrap(),
            model_infer_replay_key_for_target(&mut cx, &right, &request, None).unwrap()
        );
    }

    #[test]
    fn model_infer_replay_key_accepts_repeated_spec_fields() {
        let mut cx = cx();
        let request = Expr::String("task".to_owned());
        let runner = Symbol::qualified("runner", "dupe");
        let spec = vec![
            (Symbol::new("capability"), Expr::String("one".to_owned())),
            (Symbol::new("capability"), Expr::String("two".to_owned())),
        ];
        let target = ModelInferTarget::new(&runner, "dupe/model", &spec);

        model_infer_replay_key_for_target(&mut cx, &target, &request, None).unwrap();
    }

    #[test]
    fn cassette_result_can_satisfy_model_infer_effect() {
        let mut cx = cx();
        let request = EvalRequest {
            expr: Expr::String("task".to_owned()),
            result_shape: None,
            required_capabilities: Vec::new(),
            deadline: None,
            consistency: sim_kernel::Consistency::LocalFirst,
            mode: sim_kernel::EvalMode::Eval,
            answer_limit: None,
            trace: false,
            stream_buffer: None,
            stream: false,
        };
        let runner = Symbol::qualified("runner", "fake");
        let target = ModelInferTarget::new(&runner, "fake/model", &[]);
        let key = model_infer_replay_key_for_target(&mut cx, &target, &request.expr, None).unwrap();
        let response = Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("model-response")),
                Expr::Bool(true),
            ),
            (
                Expr::Symbol(Symbol::new("runner")),
                Expr::Symbol(Symbol::qualified("runner", "fake")),
            ),
            (
                Expr::Symbol(Symbol::new("model")),
                Expr::String("fake/model".to_owned()),
            ),
            (Expr::Symbol(Symbol::new("content")), Expr::List(Vec::new())),
            (
                Expr::Symbol(Symbol::new("stop-reason")),
                Expr::Symbol(Symbol::new("stop")),
            ),
        ]);
        let response_ref = response_ref(&mut cx, response.clone()).unwrap();
        cx.effect_ledger_mut()
            .insert_cassette_result(key, response_ref);

        let actual = resolve_model_infer_effect(&mut cx, &target, &request, |_cx| {
            panic!("cassette result should bypass inference")
        })
        .unwrap();

        assert_eq!(actual, response);
    }
}
