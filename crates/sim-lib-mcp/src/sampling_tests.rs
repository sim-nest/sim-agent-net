use std::sync::Arc;

use sim_kernel::{Args, Expr, Symbol};
use sim_lib_agent_runner_core::{ModelRequest, ModelRunner, VecEventSink};

use crate::{
    FixtureSamplingHost, McpSamplingRunner, McpSamplingRunnerValue, install_mcp_lib,
    mcp_sampling_capability, mcp_sampling_runner_symbol,
};

#[test]
fn sampling_runner_infer_uses_host_create_message() {
    let mut cx = cx();
    cx.grant(mcp_sampling_capability());
    let host = Arc::new(FixtureSamplingHost::text("sampled answer"));
    let runner =
        McpSamplingRunner::new(Symbol::qualified("mcp", "host"), "host-model", host.clone());

    let response = runner.infer(&mut cx, request()).unwrap();

    assert_eq!(host.call_count(), 1);
    assert_eq!(host.methods().unwrap(), vec!["sampling/createMessage"]);
    assert_eq!(response.runner, Symbol::qualified("mcp", "host"));
    assert_eq!(response.model, "host-model");
    assert_eq!(text_content(&response.content), Some("sampled answer"));
    assert_eq!(runner.card().provider, Symbol::new("mcp"));
}

#[test]
fn sampling_runner_requires_sampling_capability() {
    let mut cx = cx();
    let host = Arc::new(FixtureSamplingHost::text("denied"));
    let runner =
        McpSamplingRunner::new(Symbol::qualified("mcp", "host"), "host-model", host.clone());

    let error = runner.infer(&mut cx, request()).unwrap_err();

    assert!(format!("{error}").contains("mcp.sampling"));
    assert_eq!(host.call_count(), 0);
}

#[test]
fn sampling_runner_streams_model_events_and_final_response() {
    let mut cx = cx();
    cx.grant(mcp_sampling_capability());
    let host = Arc::new(FixtureSamplingHost::streamed(
        vec!["hel".to_owned(), "lo".to_owned()],
        "hello",
    ));
    let runner = McpSamplingRunner::new(Symbol::qualified("mcp", "host"), "host-model", host);
    let mut sink = VecEventSink::new();

    let response = runner.infer_stream(&mut cx, request(), &mut sink).unwrap();

    let events = sink.into_events();
    assert_eq!(events[0].event, Symbol::new("start"));
    assert_eq!(events[1].event, Symbol::new("delta"));
    assert_eq!(events[2].event, Symbol::new("delta"));
    assert_eq!(events[3].event, Symbol::new("final"));
    assert_eq!(text_content(&response.content), Some("hello"));
}

#[cfg(feature = "stream")]
#[test]
fn sampling_stream_events_carry_existing_stream_packet_exprs() {
    let mut cx = cx();
    cx.grant(mcp_sampling_capability());
    let host = Arc::new(FixtureSamplingHost::streamed(
        vec!["chunk".to_owned()],
        "chunk",
    ));
    let runner = McpSamplingRunner::new(Symbol::qualified("mcp", "host"), "host-model", host);
    let mut sink = VecEventSink::new();

    runner.infer_stream(&mut cx, request(), &mut sink).unwrap();

    let delta = sink
        .events()
        .iter()
        .find(|event| event.event == Symbol::new("delta"))
        .unwrap();
    assert!(event_field(delta, "stream-packet").is_some());
}

#[test]
fn sampling_runner_function_returns_runner_object() {
    let mut cx = cx();
    install_mcp_lib(&mut cx).unwrap();

    let value = cx
        .call_function(&mcp_sampling_runner_symbol(), Args::default())
        .unwrap();

    assert!(
        value
            .object()
            .downcast_ref::<McpSamplingRunnerValue>()
            .is_some()
    );
}

fn request() -> ModelRequest {
    ModelRequest::new(
        Expr::String("answer".to_owned()),
        vec![Expr::String("question".to_owned())],
    )
}

fn text_content(content: &[Expr]) -> Option<&str> {
    content
        .first()
        .and_then(|expr| match event_field_expr(expr, "text") {
            Some(Expr::String(text)) => Some(text.as_str()),
            _ => None,
        })
}

#[cfg(feature = "stream")]
fn event_field<'a>(
    event: &'a sim_lib_agent_runner_core::ModelEvent,
    name: &str,
) -> Option<&'a Expr> {
    event.extra.iter().find_map(|(key, value)| {
        let Expr::Symbol(symbol) = key else {
            return None;
        };
        (symbol.namespace.is_none() && symbol.name.as_ref() == name).then_some(value)
    })
}

fn event_field_expr<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(fields) = expr else {
        return None;
    };
    fields.iter().find_map(|(key, value)| {
        let Expr::Symbol(symbol) = key else {
            return None;
        };
        (symbol.namespace.is_none() && symbol.name.as_ref() == name).then_some(value)
    })
}

use sim_kernel::testing::eager_cx as cx;
