use super::support::{
    as_component, eval_cx, flatten_text, install_agent_lib, install_roundtrip_codecs, request_frame,
};
use crate::tools::{Tool, register_tool};
use sim_kernel::{Args, Cx, Error, Expr, Object, Result, Symbol, Value, read_eval_capability};
use sim_lib_server::{
    EvalSite, FrameEnvelope, FrameKind, ServerAddress, ServerFrame, eval_reply_from_frame,
};
use sim_shape::{AnyShape, shape_value};
use std::{any::Any, sync::Arc};

#[test]
fn r12_judges_compute_real_scores_rankings_and_rejections() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let rubric = cx
        .call_function(
            &Symbol::qualified("judge", "rubric"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":rubric")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::Symbol(Symbol::new(":correctness")),
                        number_expr(0.6),
                        Expr::Symbol(Symbol::new(":style")),
                        number_expr(0.2),
                        Expr::Symbol(Symbol::new(":brevity")),
                        number_expr(0.2),
                    ]))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":reference")).unwrap(),
                cx.factory()
                    .string("sim agents route work quickly".to_owned())
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":style-markers")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![Expr::String("quickly".to_owned())]))
                    .unwrap(),
            ]),
        )
        .unwrap();
    let rubric_request = request_frame(
        &mut cx,
        Expr::String("SIM agents route work quickly".to_owned()),
    );
    let rubric_reply = as_component(&rubric)
        .answer(&mut cx, rubric_request)
        .unwrap();
    let rubric_expr = eval_reply_from_frame(&mut cx, &rubric_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    let score = map_number_field(&rubric_expr, "score");
    assert!(score > 0.7, "rubric score too low: {score}");
    assert!(map_has_key(&rubric_expr, "breakdown"));

    let ranked = cx
        .call_function(
            &Symbol::qualified("judge", "ranked-vote"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":rubric")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::Symbol(Symbol::new(":correctness")),
                        number_expr(0.7),
                        Expr::Symbol(Symbol::new(":brevity")),
                        number_expr(0.3),
                    ]))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":reference")).unwrap(),
                cx.factory().string("hello sim world".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let ranked_request = request_frame(
        &mut cx,
        Expr::List(vec![
            Expr::String("hello sim world".to_owned()),
            Expr::String("hello world with extra padding words and clutter".to_owned()),
            Expr::String("unrelated output".to_owned()),
        ]),
    );
    let ranked_reply = as_component(&ranked)
        .answer(&mut cx, ranked_request)
        .unwrap();
    let ranked_expr = eval_reply_from_frame(&mut cx, &ranked_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(
        map_expr_field(&ranked_expr, "winner"),
        Expr::String("hello sim world".to_owned())
    );
    assert!(flatten_text(&ranked_expr).contains("ranking"));

    let threshold = cx
        .call_function(
            &Symbol::qualified("judge", "threshold"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":threshold")).unwrap(),
                cx.factory().expr(number_expr(0.8)).unwrap(),
                cx.factory().symbol(Symbol::new(":reference")).unwrap(),
                cx.factory()
                    .string("ship a precise concise answer".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    let threshold_request =
        request_frame(&mut cx, Expr::String("rambling mismatch text".to_owned()));
    let threshold_reply = as_component(&threshold)
        .answer(&mut cx, threshold_request)
        .unwrap();
    let threshold_expr = eval_reply_from_frame(&mut cx, &threshold_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(!map_bool_field(&threshold_expr, "approved"));
}

#[test]
fn r12_router_bid_runs_real_auction_and_picks_best_worker() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let cheap = register_bid_tool(&mut cx, Symbol::qualified("test", "cheap-bid"), 3.0);
    let expensive = register_bid_tool(&mut cx, Symbol::qualified("test", "expensive-bid"), 9.0);
    let _ = (cheap, expensive);

    let router = cx
        .call_function(
            &Symbol::qualified("router", "bid"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":targets")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::Symbol(Symbol::qualified("test", "expensive-bid")),
                        Expr::Symbol(Symbol::qualified("test", "cheap-bid")),
                    ]))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":metric")).unwrap(),
                cx.factory().symbol(Symbol::new("estimated-cost")).unwrap(),
                cx.factory().symbol(Symbol::new(":auction-window")).unwrap(),
                cx.factory().string("200ms".to_owned()).unwrap(),
            ]),
        )
        .unwrap();

    let router_request = request_frame(&mut cx, Expr::String("route this task".to_owned()));
    let reply = as_component(&router)
        .answer(&mut cx, router_request)
        .unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(
        map_expr_field(&expr, "target"),
        Expr::Symbol(Symbol::qualified("test", "cheap-bid"))
    );
    assert_eq!(map_number_field(&expr, "bid"), 3.0);
}

#[test]
fn r12_router_sticky_rejects_read_eval_payloads_before_routing() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    let router = cx
        .call_function(
            &Symbol::qualified("router", "sticky"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":targets")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::Symbol(Symbol::qualified("test", "first")),
                        Expr::Symbol(Symbol::qualified("test", "second")),
                    ]))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":sticky-key")).unwrap(),
                cx.factory().symbol(Symbol::new("session")).unwrap(),
            ]),
        )
        .unwrap();
    let frame = ServerFrame::new(
        Symbol::qualified("codec", "lisp"),
        FrameKind::Request,
        FrameEnvelope::default(),
        b"#. 1".to_vec(),
    );

    let err = as_component(&router).answer(&mut cx, frame).unwrap_err();

    assert_read_eval_denied(err);
}

#[test]
fn r12_personas_apply_real_style_language_and_translation_rules() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let style = cx
        .call_function(
            &Symbol::qualified("persona", "style"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":voice")).unwrap(),
                cx.factory().string("terse".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":prefix")).unwrap(),
                cx.factory().string("SIM:".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":suffix")).unwrap(),
                cx.factory().string("[ok]".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let style_request = request_frame(
        &mut cx,
        Expr::String(
            "SIM: explain the routing behavior with several extra filler words here [ok]"
                .to_owned(),
        ),
    );
    let style_reply = as_component(&style).answer(&mut cx, style_request).unwrap();
    let style_expr = eval_reply_from_frame(&mut cx, &style_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    let styled = expect_string(style_expr);
    assert!(styled.starts_with("SIM: "));
    assert!(styled.ends_with("[ok]"));
    assert!(styled.split_whitespace().count() <= 14);

    let language = cx
        .call_function(
            &Symbol::qualified("persona", "language"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":language")).unwrap(),
                cx.factory().string("es".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let language_request = request_frame(&mut cx, Expr::String("hello agent mystery".to_owned()));
    let language_reply = as_component(&language)
        .answer(&mut cx, language_request)
        .unwrap();
    let language_expr = eval_reply_from_frame(&mut cx, &language_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    let localized = expect_string(language_expr);
    assert!(localized.contains("hola"));
    assert!(localized.contains("agente"));
    assert!(localized.contains("[es:mystery]"));

    let translator = cx
        .call_function(
            &Symbol::qualified("persona", "translator"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":from")).unwrap(),
                cx.factory().string("en".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":to")).unwrap(),
                cx.factory().string("sv".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":table")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::Symbol(Symbol::new(":hello")),
                        Expr::String("hej".to_owned()),
                        Expr::Symbol(Symbol::new(":world")),
                        Expr::String("varld".to_owned()),
                    ]))
                    .unwrap(),
            ]),
        )
        .unwrap();
    let translator_request = request_frame(&mut cx, Expr::String("hello world unknown".to_owned()));
    let translator_reply = as_component(&translator)
        .answer(&mut cx, translator_request)
        .unwrap();
    let translator_expr = eval_reply_from_frame(&mut cx, &translator_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(
        map_expr_field(&translator_expr, "text"),
        Expr::String("hej varld unknown".to_owned())
    );
    assert!(map_number_field(&translator_expr, "coverage") > 0.6);
}

#[derive(Clone)]
struct FixedBidFn {
    bid: f64,
}

impl Object for FixedBidFn {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<function test/fixed-bid>".to_owned())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for FixedBidFn {
    fn class(&self, cx: &mut Cx) -> Result<sim_kernel::ClassRef> {
        cx.factory().class_stub(
            sim_kernel::ClassId(0),
            Symbol::qualified("core", "Function"),
        )
    }
    fn as_callable(&self) -> Option<&dyn sim_kernel::Callable> {
        Some(self)
    }
}

impl sim_kernel::Callable for FixedBidFn {
    fn call(&self, cx: &mut Cx, _args: Args) -> Result<Value> {
        cx.factory()
            .number_literal(Symbol::qualified("numbers", "f64"), self.bid.to_string())
    }
}

fn register_bid_tool(cx: &mut Cx, symbol: Symbol, bid: f64) -> Arc<Tool> {
    let callable = cx.factory().opaque(Arc::new(FixedBidFn { bid })).unwrap();
    let tool = Arc::new(Tool {
        symbol: symbol.clone(),
        description: "fixed bid worker".to_owned(),
        args_shape: shape_value(Symbol::qualified("test", "bid-args"), Arc::new(AnyShape)),
        result_shape: None,
        category: Symbol::new("worker"),
        capabilities: Vec::new(),
        function: callable,
        address: ServerAddress::Local,
        codecs: vec![Symbol::qualified("codec", "binary")],
    });
    let value = cx.factory().opaque(tool.clone()).unwrap();
    register_tool(cx, tool.clone(), value).unwrap();
    tool
}

fn map_expr_field(expr: &Expr, key: &str) -> Expr {
    let Expr::Map(entries) = expr else {
        panic!("expected map expr, found {expr:?}");
    };
    entries
        .iter()
        .find_map(|(entry_key, entry_value)| match entry_key {
            Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(entry_value.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing key {key} in {expr:?}"))
}

fn map_number_field(expr: &Expr, key: &str) -> f64 {
    match map_expr_field(expr, key) {
        Expr::Number(number) => number.canonical.parse::<f64>().unwrap(),
        other => panic!("expected numeric field {key}, found {other:?}"),
    }
}

fn map_bool_field(expr: &Expr, key: &str) -> bool {
    match map_expr_field(expr, key) {
        Expr::Bool(value) => value,
        other => panic!("expected bool field {key}, found {other:?}"),
    }
}

fn map_has_key(expr: &Expr, key: &str) -> bool {
    let Expr::Map(entries) = expr else {
        return false;
    };
    entries.iter().any(
        |(entry_key, _)| matches!(entry_key, Expr::Symbol(symbol) if symbol.name.as_ref() == key),
    )
}

fn assert_read_eval_denied(error: Error) {
    assert!(
        matches!(error, Error::CapabilityDenied { ref capability } if capability == &read_eval_capability()),
        "expected read-eval denial, got {error:?}",
    );
}

fn number_expr(value: f64) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn expect_string(expr: Expr) -> String {
    match expr {
        Expr::String(text) => text,
        other => panic!("expected string expr, found {other:?}"),
    }
}
