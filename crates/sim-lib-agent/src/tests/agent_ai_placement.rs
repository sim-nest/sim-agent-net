use super::support::{
    eval_cx, flatten_text, install_agent_lib, install_test_codec, temp_memory_path,
};
use crate::AI_RUNNER_PLACEMENT_CAPABILITY;
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{
    AbiVersion, Args, Callable, Consistency, Error, EvalMode, EvalRequest, Export, Expr, Lib,
    LibManifest, LibTarget, Linker, LoadCx, Object, ObjectCompat, Result, Symbol, Value, Version,
};
use sim_value::access::field as map_field;
use std::sync::Arc;

fn model_request_expr(task: &str) -> Expr {
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String(task.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(Vec::new()),
        ),
    ])
}

fn routed_model_request(task: &str, placement: &str) -> EvalRequest {
    let Expr::Map(mut entries) = model_request_expr(task) else {
        unreachable!("model_request_expr returns a map");
    };
    entries.push((
        Expr::Symbol(Symbol::new("routing")),
        Expr::Map(vec![(
            Expr::Symbol(Symbol::new("placement")),
            Expr::String(placement.to_owned()),
        )]),
    ));
    model_request_from_expr(Expr::Map(entries))
}

fn model_request(task: &str) -> EvalRequest {
    model_request_from_expr(model_request_expr(task))
}

fn model_request_from_expr(expr: Expr) -> EvalRequest {
    EvalRequest {
        expr,
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

fn fake_runner(cx: &mut sim_kernel::Cx) -> Value {
    let script = Expr::List(vec![
        Expr::String("scripted".to_owned()),
        Expr::String("scripted".to_owned()),
    ]);
    let script_value = cx.factory().expr(script).unwrap();
    cx.call_function(
        &Symbol::qualified("runner", "fake"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":script")).unwrap(),
            script_value,
        ]),
    )
    .unwrap()
}

fn named_fake_runner(cx: &mut sim_kernel::Cx, name: &str, model: &str, text: &str) -> Value {
    let script_value = cx
        .factory()
        .expr(Expr::List(vec![Expr::String(text.to_owned())]))
        .unwrap();
    cx.call_function(
        &Symbol::qualified("runner", "fake"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory().symbol(Symbol::new(name)).unwrap(),
            cx.factory().symbol(Symbol::new(":model")).unwrap(),
            cx.factory().string(model.to_owned()).unwrap(),
            cx.factory().symbol(Symbol::new(":script")).unwrap(),
            script_value,
        ]),
    )
    .unwrap()
}

fn named_echo_runner(cx: &mut sim_kernel::Cx, name: &str, model: &str) -> Value {
    cx.call_function(
        &Symbol::qualified("runner", "echo"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory().symbol(Symbol::new(name)).unwrap(),
            cx.factory().symbol(Symbol::new(":model")).unwrap(),
            cx.factory().string(model.to_owned()).unwrap(),
        ]),
    )
    .unwrap()
}

fn named_cassette_runner(
    cx: &mut sim_kernel::Cx,
    name: &str,
    model: &str,
    journal: String,
) -> Value {
    cx.call_function(
        &Symbol::qualified("runner", "cassette"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory().symbol(Symbol::new(name)).unwrap(),
            cx.factory().symbol(Symbol::new(":model")).unwrap(),
            cx.factory().string(model.to_owned()).unwrap(),
            cx.factory().symbol(Symbol::new(":journal")).unwrap(),
            cx.factory().string(journal).unwrap(),
        ]),
    )
    .unwrap()
}

fn place_runner(cx: &mut sim_kernel::Cx, key: &str, runner: Value) {
    cx.grant_named(AI_RUNNER_PLACEMENT_CAPABILITY);
    cx.call_function(
        &Symbol::qualified("runner", "place"),
        Args::new(vec![
            cx.factory().string(key.to_owned()).unwrap(),
            runner.clone(),
        ]),
    )
    .unwrap();
}

fn replace_runner(cx: &mut sim_kernel::Cx, key: &str, runner: Value) {
    cx.grant_named(AI_RUNNER_PLACEMENT_CAPABILITY);
    cx.call_function(
        &Symbol::qualified("runner", "place"),
        Args::new(vec![
            cx.factory().string(key.to_owned()).unwrap(),
            runner.clone(),
            cx.factory().symbol(Symbol::new(":replace")).unwrap(),
            cx.factory().bool(true).unwrap(),
        ]),
    )
    .unwrap();
}

#[derive(Clone)]
struct StubLoadedSite {
    answer: String,
}

impl Object for StubLoadedSite {
    fn display(&self, _cx: &mut sim_kernel::Cx) -> Result<String> {
        Ok("#<stub-loaded-site>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for StubLoadedSite {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for StubLoadedSite {
    fn call(&self, cx: &mut sim_kernel::Cx, args: Args) -> Result<Value> {
        let [request] = args.values() else {
            return Err(sim_kernel::Error::Eval(
                "stub loaded site expects one request argument".to_owned(),
            ));
        };
        let request = request.object().as_expr(cx)?;
        let task = map_field(
            map_field(&request, "expr").ok_or_else(|| {
                sim_kernel::Error::Eval("loaded site request missing expr".to_owned())
            })?,
            "task",
        );
        assert!(matches!(task, Some(Expr::String(text)) if text == "loaded prompt"));
        cx.factory().string(self.answer.clone())
    }
}

struct StubLoadedSiteLib {
    symbol: Symbol,
    value: Value,
}

impl Lib for StubLoadedSiteLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("test", format!("loaded-site-{}", self.symbol.name)),
            version: Version("0.1.0".to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Site {
                symbol: self.symbol.clone(),
                runtime_id: None,
            }],
        }
    }

    fn load(&self, _cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        linker.site_value(self.symbol.clone(), self.value.clone())?;
        Ok(())
    }
}

fn register_loaded_site(cx: &mut sim_kernel::Cx, symbol: Symbol, answer: &str) {
    let value = cx
        .factory()
        .opaque(Arc::new(StubLoadedSite {
            answer: answer.to_owned(),
        }))
        .unwrap();
    cx.load_lib(&StubLoadedSiteLib { symbol, value }).unwrap();
}

#[test]
fn runner_place_requires_placement_capability() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let runner = named_fake_runner(
        &mut cx,
        "placement-denied-runner",
        "placement/denied",
        "denied",
    );
    let denied = cx
        .call_function(
            &Symbol::qualified("runner", "place"),
            Args::new(vec![
                cx.factory()
                    .string("model-site:placement-denied".to_owned())
                    .unwrap(),
                runner,
            ]),
        )
        .unwrap_err();

    assert!(matches!(
        denied,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new(AI_RUNNER_PLACEMENT_CAPABILITY)
    ));
}

#[test]
fn runner_place_rejects_duplicate_without_replace() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let key = "model-site:placement-duplicate";
    let first = named_fake_runner(
        &mut cx,
        "placement-duplicate-first",
        "placement/duplicate-first",
        "first",
    );
    let second = named_fake_runner(
        &mut cx,
        "placement-duplicate-second",
        "placement/duplicate-second",
        "second",
    );
    place_runner(&mut cx, key, first);

    let denied = cx
        .call_function(
            &Symbol::qualified("runner", "place"),
            Args::new(vec![cx.factory().string(key.to_owned()).unwrap(), second]),
        )
        .unwrap_err();

    assert!(denied.to_string().contains("already registered"));
}

#[test]
fn runner_place_explicit_replace_is_audited() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let key = "model-site:placement-replace";
    let first = named_fake_runner(
        &mut cx,
        "placement-replace-first",
        "placement/replace-first",
        "first",
    );
    let second = named_fake_runner(
        &mut cx,
        "placement-replace-second",
        "placement/replace-second",
        "second",
    );
    place_runner(&mut cx, key, first);
    let before = cx.effect_ledger().records().len();
    replace_runner(&mut cx, key, second);
    let after = cx.effect_ledger().records();

    assert!(after.len() > before);
    assert!(after.last().is_some_and(|record| record.result.is_some()));

    let card = cx
        .call_function(
            &Symbol::qualified("model", "site-card"),
            Args::new(vec![cx.factory().string(key.to_owned()).unwrap()]),
        )
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(
        card_string(&card, "model"),
        Some("placement/replace-second")
    );
}

#[test]
fn realize_resolves_fake_runner_through_placement_key() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let runner = fake_runner(&mut cx);
    let key = "model-site:default";
    place_runner(&mut cx, key, runner.clone());

    let direct_reply = runner
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(&mut cx, model_request("ignored"))
        .unwrap();
    let direct_expr = direct_reply.value.object().as_expr(&mut cx).unwrap();
    validate_chat_transcript(&direct_expr).unwrap();

    let placed = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "realize"))),
            args: vec![
                model_request_expr("ignored"),
                Expr::Symbol(Symbol::new(":on")),
                Expr::Call {
                    operator: Box::new(Expr::Symbol(Symbol::qualified("model", "at"))),
                    args: vec![Expr::String(key.to_owned())],
                },
            ],
        })
        .unwrap();
    let placed_expr = placed.object().as_expr(&mut cx).unwrap();
    validate_chat_transcript(&placed_expr).unwrap();

    assert_eq!(flatten_text(&direct_expr), flatten_text(&placed_expr));
    assert!(flatten_text(&placed_expr).contains("scripted"));
}

#[test]
fn loaded_site_resolves_through_model_at() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let symbol = Symbol::qualified("model", "loaded-site-realize");
    let key = symbol.to_string();
    register_loaded_site(&mut cx, symbol, "loaded answer");

    let placement = cx
        .call_function(
            &Symbol::qualified("model", "at"),
            Args::new(vec![cx.factory().string(key).unwrap()]),
        )
        .unwrap();
    let reply = placement
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(&mut cx, model_request("loaded prompt"))
        .unwrap();

    assert_eq!(
        reply.value.object().as_expr(&mut cx).unwrap(),
        Expr::String("loaded answer".to_owned())
    );
}

#[test]
fn loaded_site_appears_in_model_site_cards() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let symbol = Symbol::qualified("model", "loaded-site-card");
    let key = symbol.to_string();
    register_loaded_site(&mut cx, symbol, "loaded prompt");

    let listed = cx
        .call_function(&Symbol::qualified("model", "sites"), Args::new(Vec::new()))
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    let matching = matching_site_cards(&listed, &[&key]);
    assert_eq!(matching.len(), 1);
    assert_eq!(card_symbol(matching[0], "locality"), Some("loaded"));
    assert_eq!(card_string(matching[0], "model"), Some(key.as_str()));
    assert!(card_list(matching[0], "codecs").is_some_and(|codecs| !codecs.is_empty()));
    assert!(card_list(matching[0], "caps").is_some_and(|caps| caps.is_empty()));
    assert!(card_string(matching[0], "endpoint").is_none());
    assert!(card_string(matching[0], "path").is_none());

    let one = cx
        .call_function(
            &Symbol::qualified("model", "site-card"),
            Args::new(vec![cx.factory().string(key.clone()).unwrap()]),
        )
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(card_string(&one, "key"), Some(key.as_str()));
    assert_eq!(card_symbol(&one, "locality"), Some("loaded"));
}

#[test]
fn model_sites_lists_registered_runner_cards() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let journal = temp_memory_path("placement-catalog");
    let keys = [
        "model-site:catalog-fake",
        "model-site:catalog-echo",
        "model-site:catalog-cassette",
    ];
    let fake = named_fake_runner(&mut cx, "catalog-fake", "catalog/fake", "catalog fake");
    let echo = named_echo_runner(&mut cx, "catalog-echo", "catalog/echo");
    let cassette = named_cassette_runner(
        &mut cx,
        "catalog-cassette",
        "catalog/cassette",
        journal.display().to_string(),
    );
    place_runner(&mut cx, keys[0], fake);
    place_runner(&mut cx, keys[1], echo);
    place_runner(&mut cx, keys[2], cassette);

    let listed = cx
        .call_function(&Symbol::qualified("model", "sites"), Args::new(Vec::new()))
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    let matching = matching_site_cards(&listed, &keys);
    assert_eq!(matching.len(), 3);
    assert_eq!(card_symbol(matching[0], "locality"), Some("fake"));
    assert_eq!(card_string(matching[0], "model"), Some("catalog/fake"));
    assert_eq!(card_symbol(matching[1], "locality"), Some("local"));
    assert_eq!(card_string(matching[1], "model"), Some("catalog/echo"));
    assert_eq!(card_symbol(matching[2], "locality"), Some("local"));
    assert_eq!(card_string(matching[2], "model"), Some("catalog/cassette"));
    assert!(card_list(matching[0], "codecs").is_some_and(|codecs| !codecs.is_empty()));
    assert!(card_list(matching[0], "caps").is_some());

    let alias = cx
        .call_function(&Symbol::new("model-sites"), Args::new(Vec::new()))
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(map_field(&alias, "model-sites"), Some(&Expr::Bool(true)));

    let one = cx
        .call_function(
            &Symbol::qualified("model", "site-card"),
            Args::new(vec![cx.factory().string(keys[1].to_owned()).unwrap()]),
        )
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(card_string(&one, "key"), Some(keys[1]));
    assert_eq!(card_string(&one, "model"), Some("catalog/echo"));

    let _ = std::fs::remove_file(journal);
}

#[test]
fn routing_placement_reaches_selected_site() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let key = "model-site:routing-fake";
    let fake = named_fake_runner(&mut cx, "routing-fake", "routing/fake", "placed answer");
    let echo = named_echo_runner(&mut cx, "routing-echo", "routing/echo");
    place_runner(&mut cx, key, fake);

    let routed = echo
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(&mut cx, routed_model_request("echo this", key))
        .unwrap();
    let routed_expr = routed.value.object().as_expr(&mut cx).unwrap();
    validate_chat_transcript(&routed_expr).unwrap();

    let text = flatten_text(&routed_expr);
    assert!(text.contains("placed answer"));
    assert!(!text.contains("echo this"));
}

#[test]
fn routing_unknown_placement_fails_closed() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let echo = named_echo_runner(&mut cx, "routing-missing", "routing/missing");
    let missing = "model-site:missing-route-test";
    let err = echo
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(&mut cx, routed_model_request("ignored", missing))
        .err()
        .expect("missing placement key unexpectedly resolved");

    assert!(err.to_string().contains(missing));
}

#[test]
fn placement_key_resolution_failure_is_eval_error() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let placement = cx
        .call_function(
            &Symbol::qualified("model", "at"),
            Args::new(vec![
                cx.factory()
                    .string("model-site:missing-placement-test".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    let result = placement
        .object()
        .as_eval_fabric()
        .unwrap()
        .realize(&mut cx, model_request("ignored"));
    let err = match result {
        Ok(_) => panic!("missing placement key unexpectedly resolved"),
        Err(err) => err,
    };

    assert!(
        err.to_string()
            .contains("model-site:missing-placement-test")
    );
}

fn matching_site_cards<'a>(expr: &'a Expr, keys: &[&str]) -> Vec<&'a Expr> {
    let Some(Expr::List(cards)) = map_field(expr, "sites") else {
        return Vec::new();
    };
    keys.iter()
        .filter_map(|key| {
            cards
                .iter()
                .find(|card| card_string(card, "key") == Some(*key))
        })
        .collect()
}

fn card_string<'a>(expr: &'a Expr, key: &str) -> Option<&'a str> {
    match map_field(expr, key) {
        Some(Expr::String(value)) => Some(value),
        _ => None,
    }
}

fn card_symbol<'a>(expr: &'a Expr, key: &str) -> Option<&'a str> {
    match map_field(expr, key) {
        Some(Expr::Symbol(symbol)) => Some(symbol.name.as_ref()),
        _ => None,
    }
}

fn card_list<'a>(expr: &'a Expr, key: &str) -> Option<&'a [Expr]> {
    match map_field(expr, key) {
        Some(Expr::List(items)) => Some(items),
        _ => None,
    }
}
