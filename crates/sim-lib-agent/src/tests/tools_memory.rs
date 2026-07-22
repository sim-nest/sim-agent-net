use super::support::{
    eval_cx, install_agent_lib, install_test_codec, register_sum_tool, temp_memory_path,
};
use crate::tools::tool_export_kind;
use crate::{AGENT_LIB_ID, FileMemory, MemoryBackend, WorkingMemory, fs_write_capability};
use sim_kernel::{Error, Expr, Symbol};
use sim_lib_server::EvalSite;

#[test]
fn tool_registration_lists_metadata_and_exports_records() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    register_sum_tool(&mut cx);

    let tools = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("agent", "tools"))),
            args: vec![
                Expr::Symbol(Symbol::new(":category")),
                Expr::Quote {
                    mode: sim_kernel::QuoteMode::Quote,
                    expr: Box::new(Expr::Symbol(Symbol::new("math"))),
                },
            ],
        })
        .unwrap();
    let Expr::List(items) = tools.object().as_expr(&mut cx).unwrap() else {
        panic!("agent/tools should return a list");
    };
    assert_eq!(items.len(), 1);
    let Expr::Map(entries) = &items[0] else {
        panic!("tool metadata should be a table");
    };
    assert!(entries.iter().any(|(key, value)| {
        *key == Expr::Symbol(Symbol::new("category")) && *value == Expr::Symbol(Symbol::new("math"))
    }));

    let loaded = cx.registry().lib(&Symbol::new(AGENT_LIB_ID)).unwrap();
    assert!(loaded.exports.iter().any(|record| {
        record.kind == tool_export_kind()
            && record.symbol == Symbol::qualified("test", "sum")
            && matches!(record.state, sim_kernel::ExportState::Resolved { .. })
    }));
}

#[test]
fn tool_call_and_eval_site_answer_use_shape_checked_args() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    let tool = register_sum_tool(&mut cx);

    let denied = cx
        .call_function(
            &Symbol::qualified("agent", "call-tool"),
            sim_kernel::Args::new(vec![
                cx.factory()
                    .symbol(Symbol::qualified("test", "sum"))
                    .unwrap(),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "2".to_owned())
                    .unwrap(),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "3".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap_err();
    assert!(matches!(
        denied,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new("math")
    ));
    cx.grant_named("math");

    let sum = cx
        .call_function(
            &Symbol::qualified("agent", "call-tool"),
            sim_kernel::Args::new(vec![
                cx.factory()
                    .symbol(Symbol::qualified("test", "sum"))
                    .unwrap(),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "2".to_owned())
                    .unwrap(),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "3".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    assert_eq!(
        sum.object().as_expr(&mut cx).unwrap(),
        Expr::Number(sim_kernel::NumberLiteral {
            domain: Symbol::qualified("numbers", "f64"),
            canonical: "5".to_owned(),
        })
    );

    let request = sim_kernel::EvalRequest {
        expr: Expr::List(vec![
            Expr::Number(sim_kernel::NumberLiteral {
                domain: Symbol::qualified("numbers", "f64"),
                canonical: "4".to_owned(),
            }),
            Expr::Number(sim_kernel::NumberLiteral {
                domain: Symbol::qualified("numbers", "f64"),
                canonical: "6".to_owned(),
            }),
        ]),
        mode: sim_kernel::EvalMode::Eval,
        result_shape: None,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        required_capabilities: Vec::new(),
        deadline: None,
        consistency: sim_kernel::Consistency::LocalFirst,
        trace: false,
    };
    let frame = sim_lib_server::server_frame_from_request(
        &mut cx,
        &Symbol::qualified("codec", "binary"),
        request,
    )
    .unwrap();
    let reply = tool.answer(&mut cx, frame).unwrap();
    let reply = sim_lib_server::eval_reply_from_frame(&mut cx, &reply).unwrap();
    assert_eq!(
        reply.value.object().as_expr(&mut cx).unwrap(),
        Expr::Number(sim_kernel::NumberLiteral {
            domain: Symbol::qualified("numbers", "f64"),
            canonical: "10".to_owned(),
        })
    );
}

#[test]
fn tool_metadata_includes_shape_values() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    let tool = register_sum_tool(&mut cx);

    let entries = tool.metadata_entries(&mut cx).unwrap();
    let args = entries
        .iter()
        .find_map(|(key, value)| (*key == Symbol::new("args")).then(|| value.clone()))
        .unwrap();
    assert!(args.object().as_shape().is_some());
}

#[test]
fn working_memory_surface_and_frame_requests_round_trip_messages() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let memory = cx
        .call_function(
            &Symbol::qualified("memory", "working"),
            sim_kernel::Args::default(),
        )
        .unwrap();
    cx.call_function(
        &Symbol::qualified("memory", "append"),
        sim_kernel::Args::new(vec![
            memory.clone(),
            cx.factory().string("alpha".to_owned()).unwrap(),
        ]),
    )
    .unwrap();
    cx.call_function(
        &Symbol::qualified("memory", "append"),
        sim_kernel::Args::new(vec![
            memory.clone(),
            cx.factory().string("beta signal".to_owned()).unwrap(),
        ]),
    )
    .unwrap();

    let recent = cx
        .call_function(
            &Symbol::qualified("memory", "recent"),
            sim_kernel::Args::new(vec![
                memory.clone(),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "1".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    assert_eq!(
        recent.object().as_expr(&mut cx).unwrap(),
        Expr::List(vec![Expr::String("beta signal".to_owned())])
    );

    let request = sim_kernel::EvalRequest {
        expr: Expr::List(vec![
            Expr::Symbol(Symbol::new("search")),
            Expr::List(vec![
                Expr::Symbol(Symbol::new("query")),
                Expr::String("signal".to_owned()),
            ]),
            Expr::Number(sim_kernel::NumberLiteral {
                domain: Symbol::qualified("numbers", "f64"),
                canonical: "3".to_owned(),
            }),
        ]),
        mode: sim_kernel::EvalMode::Eval,
        result_shape: None,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        required_capabilities: Vec::new(),
        deadline: None,
        consistency: sim_kernel::Consistency::LocalFirst,
        trace: false,
    };
    let frame = sim_lib_server::server_frame_from_request(
        &mut cx,
        &Symbol::qualified("codec", "binary"),
        request,
    )
    .unwrap();
    let reply = memory
        .object()
        .downcast_ref::<WorkingMemory>()
        .unwrap()
        .answer(&mut cx, frame)
        .unwrap();
    let reply = sim_lib_server::eval_reply_from_frame(&mut cx, &reply).unwrap();
    assert_eq!(
        reply.value.object().as_expr(&mut cx).unwrap(),
        Expr::List(vec![Expr::String("beta signal".to_owned())])
    );
}

#[test]
fn file_memory_surface_requires_file_write_capability() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let path = temp_memory_path("requires-file-write");
    let denied = cx
        .call_function(
            &Symbol::qualified("memory", "file"),
            sim_kernel::Args::new(vec![
                cx.factory().string(path.display().to_string()).unwrap(),
            ]),
        )
        .unwrap_err();
    assert!(matches!(
        denied,
        Error::CapabilityDenied { capability }
            if capability == fs_write_capability()
    ));

    cx.grant_named("file-write");
    let memory = cx
        .call_function(
            &Symbol::qualified("memory", "file"),
            sim_kernel::Args::new(vec![
                cx.factory().string(path.display().to_string()).unwrap(),
            ]),
        )
        .unwrap();
    assert!(memory.object().downcast_ref::<FileMemory>().is_some());
    let _ = std::fs::remove_file(path);
}

#[test]
fn file_memory_persists_and_blackboard_shares_state() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let path = temp_memory_path("episodic");
    let memory = FileMemory::open(&path, vec![Symbol::qualified("codec", "binary")]).unwrap();
    let persisted = cx.factory().string("persisted".to_owned()).unwrap();
    cx.grant_named("file-write");
    memory.append(&mut cx, persisted).unwrap();
    let reopened = FileMemory::open(&path, vec![Symbol::qualified("codec", "binary")]).unwrap();
    assert_eq!(
        reopened.snapshot(&mut cx).unwrap(),
        Expr::List(vec![Expr::String("persisted".to_owned())])
    );

    let left = crate::BlackboardMemory::new(
        "ephemeral".to_owned(),
        vec![Symbol::qualified("codec", "binary")],
    );
    let right = crate::BlackboardMemory::new(
        "ephemeral".to_owned(),
        vec![Symbol::qualified("codec", "binary")],
    );
    let shared = cx.factory().string("shared".to_owned()).unwrap();
    left.append(&mut cx, shared).unwrap();
    assert_eq!(
        right.snapshot(&mut cx).unwrap(),
        Expr::List(vec![Expr::String("shared".to_owned())])
    );

    let _ = std::fs::remove_file(path);
}
