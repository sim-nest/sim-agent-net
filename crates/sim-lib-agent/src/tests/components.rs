use super::support::{
    as_component, eval_cx, flatten_text, install_agent_lib, install_roundtrip_codecs,
    install_test_codec, request_frame, temp_memory_path, temp_text_path,
};
use crate::{AgentLib, Component, ComponentKind};
use sim_codec::{Input, decode_with_codec, encode_with_codec};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{EncodeOptions, Error, Expr, Lib, ReadPolicy, Symbol};
use sim_lib_server::{EvalSite, FrameKind, ServerFrame, eval_reply_from_frame};

#[test]
fn a4_exports_register_all_component_constructors() {
    let exports = AgentLib.manifest().exports;
    for symbol in [
        Symbol::qualified("planner", "budget"),
        Symbol::qualified("planner", "refine"),
        Symbol::qualified("planner", "parallel"),
        Symbol::qualified("planner", "chain"),
        Symbol::qualified("judge", "rubric"),
        Symbol::qualified("judge", "ranked-vote"),
        Symbol::qualified("judge", "threshold"),
        Symbol::qualified("router", "round-robin"),
        Symbol::qualified("router", "bid"),
        Symbol::qualified("router", "sticky"),
        Symbol::qualified("persona", "style"),
        Symbol::qualified("persona", "language"),
        Symbol::qualified("persona", "translator"),
        Symbol::qualified("retriever", "vector"),
        Symbol::qualified("retriever", "web"),
        Symbol::qualified("retriever", "file"),
        Symbol::qualified("retriever", "db"),
        Symbol::qualified("sandbox", "wasm"),
        Symbol::qualified("sandbox", "subprocess"),
        Symbol::qualified("sandbox", "capability-restricted"),
        Symbol::qualified("recorder", "journal"),
        Symbol::qualified("recorder", "audit"),
        Symbol::qualified("recorder", "prometheus"),
        Symbol::qualified("voice", "tts"),
        Symbol::qualified("voice", "stt"),
        Symbol::qualified("memory", "working"),
        Symbol::qualified("memory", "file"),
        Symbol::qualified("memory", "vector"),
        Symbol::qualified("memory", "blackboard"),
        Symbol::qualified("memory", "persona"),
        Symbol::qualified("runner", "cassette"),
        Symbol::qualified("runner", "echo"),
        Symbol::qualified("runner", "fake"),
    ] {
        assert!(exports.iter().any(|export| {
            matches!(export, sim_kernel::Export::Function { symbol: export_symbol, .. } if export_symbol == &symbol)
        }));
    }
    #[cfg(feature = "runner-process")]
    assert!(exports.iter().any(|export| {
        matches!(export, sim_kernel::Export::Function { symbol, .. } if symbol == &Symbol::qualified("runner", "process"))
    }));
    #[cfg(feature = "runner-http")]
    assert!(exports.iter().any(|export| {
        matches!(export, sim_kernel::Export::Function { symbol, .. } if symbol == &Symbol::qualified("runner", "openai-compatible"))
    }));
    #[cfg(feature = "runner-ollama")]
    assert!(exports.iter().any(|export| {
        matches!(export, sim_kernel::Export::Function { symbol, .. } if symbol == &Symbol::qualified("runner", "ollama"))
    }));
}

#[test]
fn a4_minimum_components_construct_reflect_and_answer() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let planner = cx
        .call_function(
            &Symbol::qualified("planner", "budget"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":max-turns")).unwrap(),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "3".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    let planner_component = as_component(&planner);
    assert_eq!(planner_component.kind(), ComponentKind::Planner);
    let planner_request = request_frame(
        &mut cx,
        Expr::List(vec![
            Expr::String("inspect".to_owned()),
            Expr::String("summarize".to_owned()),
        ]),
    );
    let planner_reply = planner_component.answer(&mut cx, planner_request).unwrap();
    let planner_value = eval_reply_from_frame(&mut cx, &planner_reply)
        .unwrap()
        .value;
    let Expr::Map(plan_entries) = planner_value.object().as_expr(&mut cx).unwrap() else {
        panic!("planner should reply with a table expr");
    };
    assert!(plan_entries.iter().any(|(key, value)| {
        *key == Expr::Symbol(Symbol::new("strategy"))
            && *value == Expr::Symbol(Symbol::new("budget"))
    }));

    let judge = cx
        .call_function(
            &Symbol::qualified("judge", "rubric"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":rubric")).unwrap(),
                cx.factory()
                    .expr(Expr::String("correctness".to_owned()))
                    .unwrap(),
            ]),
        )
        .unwrap();
    let judge_request = request_frame(
        &mut cx,
        Expr::List(vec![
            Expr::String("candidate".to_owned()),
            Expr::String("correctness".to_owned()),
        ]),
    );
    let judge_reply = as_component(&judge).answer(&mut cx, judge_request).unwrap();
    let judge_expr = eval_reply_from_frame(&mut cx, &judge_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(flatten_text(&judge_expr).contains("approved"));

    let router = cx
        .call_function(
            &Symbol::qualified("router", "round-robin"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":targets")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::Symbol(Symbol::new("alpha")),
                        Expr::Symbol(Symbol::new("beta")),
                    ]))
                    .unwrap(),
            ]),
        )
        .unwrap();
    let router_component = as_component(&router);
    let first_request = request_frame(&mut cx, Expr::String("one".to_owned()));
    let first_frame = router_component.answer(&mut cx, first_request).unwrap();
    let first = eval_reply_from_frame(&mut cx, &first_frame)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    let second_request = request_frame(&mut cx, Expr::String("two".to_owned()));
    let second_frame = router_component.answer(&mut cx, second_request).unwrap();
    let second = eval_reply_from_frame(&mut cx, &second_frame)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_ne!(first, second);

    let persona = cx
        .call_function(
            &Symbol::qualified("persona", "style"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":voice")).unwrap(),
                cx.factory().string("terse".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let persona_request = request_frame(&mut cx, Expr::String("Explain SIM".to_owned()));
    let persona_reply = as_component(&persona)
        .answer(&mut cx, persona_request)
        .unwrap();
    let persona_expr = eval_reply_from_frame(&mut cx, &persona_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    let persona_text = flatten_text(&persona_expr);
    assert!(persona_text.contains("explain"));
    assert!(persona_text.split_whitespace().count() <= 12);
}

#[test]
fn a5_runner_components_reflect_answer_and_exhaust_script() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let echo = cx
        .call_function(
            &Symbol::qualified("runner", "echo"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("echo-runner")).unwrap(),
                cx.factory().symbol(Symbol::new(":model")).unwrap(),
                cx.factory().string("echo/model".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let echo_component = as_component(&echo);
    assert_eq!(echo_component.kind(), ComponentKind::Runner);
    let echo_reflection = echo_component.reflect(&mut cx).unwrap();
    assert!(flatten_text(&echo_reflection).contains("echo/model"));

    cx.grant_named("agent-spawn");
    let agent = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":runners")).unwrap(),
                echo.clone(),
            ]),
        )
        .unwrap();
    let selected = cx
        .call_function(
            &Symbol::qualified("agent", "component"),
            sim_kernel::Args::new(vec![
                agent.clone(),
                cx.factory().string("runner".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    assert_eq!(as_component(&selected).kind(), ComponentKind::Runner);

    let response = {
        let request = request_frame(
            &mut cx,
            Expr::Map(vec![
                (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
                (
                    Expr::Symbol(Symbol::new("task")),
                    Expr::String("summarize lib".to_owned()),
                ),
                (
                    Expr::Symbol(Symbol::new("messages")),
                    Expr::List(Vec::new()),
                ),
            ]),
        );
        echo_component.answer(&mut cx, request).unwrap()
    };
    let response_expr = eval_reply_from_frame(&mut cx, &response)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    validate_chat_transcript(&response_expr).unwrap();
    assert!(flatten_text(&response_expr).contains("summarize lib"));

    let fake = cx
        .call_function(
            &Symbol::qualified("runner", "fake"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":script")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::String("first reply".to_owned()),
                        Expr::String("second reply".to_owned()),
                    ]))
                    .unwrap(),
            ]),
        )
        .unwrap();
    let fake_component = as_component(&fake);
    for expected in ["first reply", "second reply"] {
        let reply = {
            let request = request_frame(
                &mut cx,
                Expr::Map(vec![
                    (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
                    (
                        Expr::Symbol(Symbol::new("task")),
                        Expr::String("next".to_owned()),
                    ),
                    (
                        Expr::Symbol(Symbol::new("messages")),
                        Expr::List(Vec::new()),
                    ),
                ]),
            );
            fake_component.answer(&mut cx, request).unwrap()
        };
        let expr = eval_reply_from_frame(&mut cx, &reply)
            .unwrap()
            .value
            .object()
            .as_expr(&mut cx)
            .unwrap();
        validate_chat_transcript(&expr).unwrap();
        assert!(flatten_text(&expr).contains(expected));
    }
    let exhausted = {
        let request = request_frame(
            &mut cx,
            Expr::Map(vec![
                (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
                (
                    Expr::Symbol(Symbol::new("task")),
                    Expr::String("last".to_owned()),
                ),
                (
                    Expr::Symbol(Symbol::new("messages")),
                    Expr::List(Vec::new()),
                ),
            ]),
        );
        fake_component.answer(&mut cx, request).unwrap()
    };
    let exhausted_expr = eval_reply_from_frame(&mut cx, &exhausted)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    validate_chat_transcript(&exhausted_expr).unwrap();
    assert!(flatten_text(&exhausted_expr).contains("no scripted response"));
}

#[test]
fn a4_retriever_sandbox_recorder_and_voice_work() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let file_path = temp_text_path("retriever");
    std::fs::write(&file_path, "SIM agent retriever").unwrap();
    cx.grant_named("file-read");

    let retriever = cx
        .call_function(
            &Symbol::qualified("retriever", "file"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":root")).unwrap(),
                cx.factory()
                    .string(file_path.parent().unwrap().display().to_string())
                    .unwrap(),
            ]),
        )
        .unwrap();
    let retriever_request = request_frame(
        &mut cx,
        Expr::String(
            file_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        ),
    );
    let retriever_reply = as_component(&retriever)
        .answer(&mut cx, retriever_request)
        .unwrap();
    let retriever_expr = eval_reply_from_frame(&mut cx, &retriever_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(flatten_text(&retriever_expr).contains("sim agent retriever"));

    let sandbox = cx
        .call_function(
            &Symbol::qualified("sandbox", "capability-restricted"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":allow")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![Expr::Symbol(Symbol::new("math"))]))
                    .unwrap(),
            ]),
        )
        .unwrap();
    let denied_request = request_frame(&mut cx, Expr::String("hello".to_owned()));
    let denied = as_component(&sandbox).answer(&mut cx, denied_request);
    assert!(matches!(denied, Err(Error::CapabilityDenied { .. })));
    cx.grant_named("sandbox");
    let sandbox_request = request_frame(&mut cx, Expr::String("hello".to_owned()));
    let sandbox_reply = as_component(&sandbox)
        .answer(&mut cx, sandbox_request)
        .unwrap();
    let sandbox_expr = eval_reply_from_frame(&mut cx, &sandbox_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(sandbox_expr, Expr::String("hello".to_owned()));

    let journal_path = temp_memory_path("journal");
    let recorder = cx
        .call_function(
            &Symbol::qualified("recorder", "journal"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":path")).unwrap(),
                cx.factory()
                    .string(journal_path.display().to_string())
                    .unwrap(),
            ]),
        )
        .unwrap();
    let notify = ServerFrame::from_expr(
        &mut cx,
        Symbol::qualified("codec", "binary"),
        FrameKind::Notify,
        &Expr::String("frame event".to_owned()),
        sim_kernel::Consistency::LocalFirst,
        Vec::new(),
        false,
    )
    .unwrap();
    as_component(&recorder).answer(&mut cx, notify).unwrap();
    let snapshot_request = request_frame(
        &mut cx,
        Expr::List(vec![Expr::Symbol(Symbol::new("snapshot"))]),
    );
    let snapshot_reply = as_component(&recorder)
        .answer(&mut cx, snapshot_request)
        .unwrap();
    let snapshot = eval_reply_from_frame(&mut cx, &snapshot_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    for codec in [
        Symbol::qualified("codec", "binary"),
        Symbol::qualified("codec", "json"),
        Symbol::qualified("codec", "lisp"),
    ] {
        let encoded =
            encode_with_codec(&mut cx, &codec, &snapshot, EncodeOptions::default()).unwrap();
        let decoded = decode_with_codec(
            &mut cx,
            &codec,
            match encoded {
                sim_codec::Output::Text(text) => Input::Text(text),
                sim_codec::Output::Bytes(bytes) => Input::Bytes(bytes),
            },
            ReadPolicy::default(),
        )
        .unwrap();
        assert!(decoded.canonical_eq(&snapshot));
    }

    let voice = cx
        .call_function(
            &Symbol::qualified("voice", "tts"),
            sim_kernel::Args::default(),
        )
        .unwrap();
    let voice_request = request_frame(&mut cx, Expr::String("speak".to_owned()));
    let voice_reply = as_component(&voice).answer(&mut cx, voice_request).unwrap();
    let voice_expr = eval_reply_from_frame(&mut cx, &voice_reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(flatten_text(&voice_expr).contains("audio"));

    let _ = std::fs::remove_file(file_path);
    let _ = std::fs::remove_file(journal_path);
}
