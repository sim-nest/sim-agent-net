use std::sync::Arc;

// conformance: the GenAI assembly recipe runs one checked ASK exchange through a placed target.

#[cfg(feature = "runner-process")]
use super::genai_assembly_support::{
    ProcessFixture, decode_process_request, model_response_for, process_runner, shell_quote_path,
    shell_quote_text,
};
use super::genai_assembly_support::{RecordingRunner, assert_recorded_contract, json_text};
#[cfg(feature = "runner-ollama")]
use super::genai_assembly_support::{
    assert_ollama_card_is_local, bind_loopback_listener, http_json_body, ollama_runner,
    spawn_ollama_recipe_mock,
};
use super::support::{eval_cx, install_agent_lib, install_roundtrip_codecs};
use crate::{AI_RUNNER_PLACEMENT_CAPABILITY, ExternalRunnerSpec, RECIPES, external_runner_value};
use sim_codec::{Input, decode_eval_expr_with_codec, decode_with_codec, lower_operator_nodes};
use sim_cookbook::{RecipeCard, RecipeStore};
use sim_kernel::{
    Args, CapabilityName, CapabilitySet, Cx, Expr, ReadPolicy, Symbol, Value,
    macro_expand_eval_capability, read_construct_capability, read_eval_capability,
};
use sim_lib_core::{SurfacePackLib, SurfacePackSpec};
use sim_value::access::field;

const RECIPE_ID: &str = "agent/01-basics/genai-assembly";
const MODEL_SITE: &str = "model-site:genai";
const SETUP_SOURCE: &str = include_str!("../../recipes/01-basics/genai-assembly/setup.siml");

#[test]
fn genai_assembly_recipe_runs_exact_source_against_replaceable_target() {
    let mut cx = recipe_cx();
    let card = genai_card();
    assert_eq!(setup_text(&card), SETUP_SOURCE);

    let recording = Arc::new(RecordingRunner::new([json_text(&Expr::String(
        "recorded checked answer".to_owned(),
    ))]));
    let runner = external_runner_value(
        &mut cx,
        ExternalRunnerSpec {
            symbol: Symbol::qualified("runner", "genai-recording"),
            model: "genai/recording".to_owned(),
            capabilities: Vec::new(),
            spec: Vec::new(),
            runner: recording.clone(),
        },
    )
    .unwrap();
    place_runner(&mut cx, runner, false);

    let run = sim_lib_cookbook::run_recipe(&mut cx, &card).unwrap();
    assert_run_ok(&mut cx, &card, &run);
    assert_eq!(
        reply_payload(&mut cx, &run),
        Expr::String("recorded checked answer".to_owned())
    );
    assert_recorded_contract(&recording.requests());

    #[cfg(feature = "runner-process")]
    {
        let fixture = ProcessFixture::new();
        let response = json_text(&Expr::from(model_response_for(
            Symbol::qualified("runner", "genai-process"),
            "genai/process",
            json_text(&Expr::String("process checked answer".to_owned())),
        )));
        let process = process_runner(
            &mut cx,
            &format!(
                "cat > {}; printf '%s' {} | tee {}",
                shell_quote_path(fixture.request_path()),
                shell_quote_text(&response),
                shell_quote_path(fixture.output_path())
            ),
        );
        place_runner(&mut cx, process, true);

        grant_real_local_caps(&mut cx);
        let processed =
            eval_recipe_source_with_real_local_caps(&mut cx, &card).unwrap_or_else(|err| {
                panic!(
                    "process recipe eval failed: {err:?}; stdout: {:?}; request: {:?}",
                    std::fs::read_to_string(fixture.output_path()),
                    std::fs::read_to_string(fixture.request_path())
                )
            });
        assert_eq!(
            reply_payload_from_expr(&processed),
            Expr::String("process checked answer".to_owned())
        );
        assert_recorded_contract(&[decode_process_request(fixture.request_path())]);
        assert_eq!(
            std::fs::read_to_string(fixture.output_path()).unwrap(),
            response
        );
    }

    #[cfg(feature = "runner-ollama")]
    if let Some(listener) = bind_loopback_listener() {
        let port = listener.local_addr().unwrap().port();
        let server = spawn_ollama_recipe_mock(listener);
        let ollama = ollama_runner(&mut cx, port);
        assert_ollama_card_is_local(&mut cx, &ollama);
        place_runner(&mut cx, ollama, true);

        grant_real_local_caps(&mut cx);
        let run = eval_recipe_source_with_real_local_caps(&mut cx, &card);
        let requests = server.join().unwrap();
        let run =
            run.unwrap_or_else(|err| panic!("ollama eval failed: {err:?}; requests: {requests:?}"));
        assert_eq!(
            reply_payload_from_expr(&run),
            Expr::String("ollama checked answer".to_owned())
        );
        assert_eq!(requests.len(), 1, "{requests:?}");
        let request = &requests[0];
        assert!(request.starts_with("POST /api/chat HTTP/1.1"), "{request}");
        assert!(!request.contains("Authorization:"), "{request}");
        let body = http_json_body(request);
        assert_eq!(body["model"], "qwen3.5:4b");
        assert_eq!(body["stream"], false);
        assert!(
            body["grammar"]
                .as_str()
                .is_some_and(|grammar| grammar.contains("root")),
            "{body:?}"
        );
        assert!(
            body["messages"].as_array().is_some_and(|messages| {
                messages.iter().any(|message| {
                    message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("Explain SIM in one sentence."))
                })
            }),
            "{body:?}"
        );
    }

    let fake = fake_runner(
        &mut cx,
        "genai-repair-fake",
        "genai/fake",
        [
            json_text(&Expr::Bool(false)),
            json_text(&Expr::String("fake repaired answer".to_owned())),
        ],
    );
    place_runner(&mut cx, fake, true);
    let repaired = sim_lib_cookbook::run_recipe(&mut cx, &card).unwrap();
    assert_run_ok(&mut cx, &card, &repaired);
    assert_eq!(
        reply_payload(&mut cx, &repaired),
        Expr::String("fake repaired answer".to_owned())
    );

    let replacement = fake_runner(
        &mut cx,
        "genai-replacement-fake",
        "genai/replacement",
        [json_text(&Expr::String("replacement answer".to_owned()))],
    );
    place_runner(&mut cx, replacement, true);
    let replaced = sim_lib_cookbook::run_recipe(&mut cx, &card).unwrap();
    assert_run_ok(&mut cx, &card, &replaced);
    assert_eq!(
        reply_payload(&mut cx, &replaced),
        Expr::String("replacement answer".to_owned())
    );

    assert_eq!(setup_text(&card), SETUP_SOURCE);
}

fn recipe_cx() -> Cx {
    let mut cx = eval_cx();
    let core = SurfacePackLib {
        spec: SurfacePackSpec {
            lib_id: sim_lib_core::manifest_name(),
            values: Vec::new(),
        },
    };
    cx.load_lib(&core).unwrap();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    sim_lib_bridge::install_bridge_lib(&mut cx).unwrap();
    cx.grant(read_eval_capability());
    cx.grant(macro_expand_eval_capability());
    cx.grant(CapabilityName::new("ai/run"));
    cx
}

fn genai_card() -> RecipeCard {
    let mut store = RecipeStore::new();
    store.register_book(RECIPES).unwrap();
    store.card(RECIPE_ID).cloned().unwrap()
}

fn assert_run_ok(cx: &mut Cx, card: &RecipeCard, run: &sim_cookbook::RecipeRun) {
    if run.ok {
        return;
    }
    let expr = lower_operator_nodes(
        decode_eval_expr_with_codec(
            cx,
            &Symbol::qualified("codec", card.codec.as_str()),
            Input::Text(setup_text(card).to_owned()),
            trusted_recipe_read_policy(),
        )
        .unwrap(),
    );
    let allowed = sim_lib_cookbook::CookbookCapabilityProfile::granted()
        .into_iter()
        .fold(CapabilitySet::new(), CapabilitySet::grant)
        .grant(read_construct_capability());
    let direct = cx.with_capabilities(allowed, |cx| cx.eval_expr(expr));
    panic!("expected recipe run to pass, got {run:?}; direct eval: {direct:?}");
}

#[cfg(any(feature = "runner-ollama", feature = "runner-process"))]
fn eval_recipe_source_with_real_local_caps(
    cx: &mut Cx,
    card: &RecipeCard,
) -> sim_kernel::Result<Expr> {
    let expr = lower_operator_nodes(
        decode_eval_expr_with_codec(
            cx,
            &Symbol::qualified("codec", card.codec.as_str()),
            Input::Text(setup_text(card).to_owned()),
            trusted_recipe_read_policy(),
        )
        .unwrap(),
    );
    let value = cx.with_capabilities(real_local_capabilities(cx), |cx| cx.eval_expr(expr))?;
    value.object().as_expr(cx)
}

fn trusted_recipe_read_policy() -> ReadPolicy {
    ReadPolicy {
        trust: sim_kernel::TrustLevel::TrustedSource,
        capabilities: CapabilitySet::new()
            .grant(read_construct_capability())
            .grant(read_eval_capability())
            .grant(macro_expand_eval_capability()),
    }
}

fn fake_runner(
    cx: &mut Cx,
    name: &str,
    model: &str,
    script: impl IntoIterator<Item = String>,
) -> Value {
    let script_value = cx
        .factory()
        .expr(Expr::List(script.into_iter().map(Expr::String).collect()))
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

fn place_runner(cx: &mut Cx, runner: Value, replace: bool) {
    cx.grant_named(AI_RUNNER_PLACEMENT_CAPABILITY);
    let mut args = vec![
        cx.factory().string(MODEL_SITE.to_owned()).unwrap(),
        runner.clone(),
    ];
    if replace {
        args.push(cx.factory().symbol(Symbol::new(":replace")).unwrap());
        args.push(cx.factory().bool(true).unwrap());
    }
    cx.call_function(&Symbol::qualified("runner", "place"), Args::new(args))
        .unwrap();
}

#[cfg(any(feature = "runner-ollama", feature = "runner-process"))]
fn real_local_capabilities(cx: &Cx) -> CapabilitySet {
    let mut capabilities = cx.capabilities().clone();
    for capability in [
        "ai-runner",
        "ai-runner-local",
        "ai-runner-network",
        "ai/run",
        "exec",
        "host.process",
    ] {
        capabilities.insert(CapabilityName::new(capability));
    }
    capabilities.insert(read_construct_capability());
    capabilities.insert(read_eval_capability());
    capabilities.insert(macro_expand_eval_capability());
    capabilities
}

#[cfg(any(feature = "runner-ollama", feature = "runner-process"))]
fn grant_real_local_caps(cx: &mut Cx) {
    for capability in [
        "ai-runner",
        "ai-runner-local",
        "ai-runner-network",
        "ai/run",
        "exec",
        "host.process",
    ] {
        cx.grant_named(capability);
    }
}

fn setup_text(card: &RecipeCard) -> &str {
    std::str::from_utf8(&card.setup).unwrap()
}

fn reply_payload(cx: &mut Cx, run: &sim_cookbook::RecipeRun) -> Expr {
    assert_eq!(run.forms, 1);
    let expr = decode_lisp(cx, &run.results[0]);
    reply_payload_from_expr(&expr)
}

fn reply_payload_from_expr(expr: &Expr) -> Expr {
    let Some(Expr::Vector(parts)) = field(expr, "body") else {
        panic!("reply packet missing body: {expr:?}");
    };
    let Some(part) = parts.first() else {
        panic!("reply packet has empty body: {expr:?}");
    };
    field(part, "payload").cloned().unwrap()
}

fn decode_lisp(cx: &mut Cx, text: &str) -> Expr {
    decode_with_codec(
        cx,
        &Symbol::qualified("codec", "lisp"),
        Input::Text(text.to_owned()),
        ReadPolicy::default(),
    )
    .unwrap()
}
