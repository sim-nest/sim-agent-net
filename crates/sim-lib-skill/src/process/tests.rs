use std::{sync::Arc, time::Duration};

use sim_kernel::{Args, Cx, DefaultFactory, EagerPolicy, Expr, HandleSeed, Result, Symbol, Value};
use sim_lib_agent_runner_core::ModelEvent;
use sim_shape::{
    ExprKind, ExprKindShape, FieldShape, FieldSpec, ListShape, NumberValueShape, shape_value,
};

use super::{
    ProcessSkillProtocol, ProcessSkillSpec, ProcessSkillTransport, skill_process_capability,
};
use crate::{
    SkillCallable, SkillEventSink, SkillRole, SkillTransport, install_skill_lib,
    skill_call_capability, skill_call_symbol, skill_install_capability, skill_install_symbol,
    skill_specific_call_capability,
};

#[test]
fn json_stdio_process_fixture_returns_typed_value() {
    let mut cx = skill_cx();
    grant_call_caps(&mut cx, "proc.answer");
    let transport = install_process_skill(&mut cx, json_transport());
    let target = cx.factory().string("proc.answer".to_owned()).unwrap();
    let request = cx.factory().string("payload".to_owned()).unwrap();

    let result = cx
        .call_function(&skill_call_symbol(), Args::new(vec![target, request]))
        .unwrap();
    let expr = result.object().as_expr(&mut cx).unwrap();

    assert_eq!(map_number(&expr, "answer"), Some("42".to_owned()));
    assert_eq!(map_string(&expr, "status"), Some("ok".to_owned()));
    assert_eq!(transport.call_count(), 1);
}

#[test]
fn process_skill_fails_closed_without_process_capability() {
    let mut cx = skill_cx();
    cx.grant(skill_install_capability());
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability("proc.answer"));
    let transport = install_process_skill(&mut cx, json_transport());
    let target = cx.factory().string("proc.answer".to_owned()).unwrap();
    let request = cx.factory().string("payload".to_owned()).unwrap();

    let err = cx
        .call_function(&skill_call_symbol(), Args::new(vec![target, request]))
        .unwrap_err();

    assert!(format!("{err:?}").contains("skill.process"));
    assert_eq!(transport.call_count(), 0);
}

#[test]
fn process_errors_and_health_do_not_expose_command_or_inputs() {
    let mut cx = skill_cx();
    grant_call_caps(&mut cx, "proc.secret");
    let transport = install_process_skill(&mut cx, secret_transport());
    let target = cx.factory().string("proc.secret".to_owned()).unwrap();
    let secret_input = "secret-input";
    let request = cx.factory().string(secret_input.to_owned()).unwrap();

    let err = cx
        .call_function(&skill_call_symbol(), Args::new(vec![target, request]))
        .unwrap_err();
    let health = transport.health(&mut cx).unwrap();
    let rendered = format!("{err:?} {:?}", health.object().as_expr(&mut cx).unwrap());

    assert!(!rendered.contains("secret-command"));
    assert!(!rendered.contains(secret_input));
    assert_eq!(transport.call_count(), 1);
}

#[test]
fn line_text_model_role_emits_model_events_through_skill_callable() {
    let mut cx = skill_cx();
    grant_call_caps(&mut cx, "proc.lines");
    let transport = install_process_skill(&mut cx, line_transport());
    let card = transport.discover(&mut cx).unwrap().remove(0);
    let callable = cx
        .resolve_function(&card.symbol)
        .unwrap()
        .object()
        .downcast_ref::<SkillCallable>()
        .unwrap()
        .clone();
    let request = cx.factory().string("payload".to_owned()).unwrap();
    let mut sink = CollectingSink::default();

    let result = callable
        .call_values(&mut cx, vec![request], Some(&mut sink))
        .unwrap();
    let text = result.object().as_expr(&mut cx).unwrap();
    let events = sink.model_events();
    let kinds = events
        .iter()
        .map(|event| event.event.clone())
        .collect::<Vec<_>>();

    assert!(matches!(text, Expr::String(value) if value.contains("one") && value.contains("two")));
    assert_eq!(
        kinds,
        vec![
            Symbol::new("start"),
            Symbol::new("delta"),
            Symbol::new("delta"),
            Symbol::new("usage"),
        ]
    );
    assert!(format!("{:?}", events[1].extra).contains("one"));
    assert!(format!("{:?}", events[2].extra).contains("two"));
    assert_eq!(transport.call_count(), 1);
}

#[derive(Default)]
struct CollectingSink {
    events: Vec<Expr>,
}

impl CollectingSink {
    fn model_events(&self) -> Vec<ModelEvent> {
        self.events
            .iter()
            .cloned()
            .map(ModelEvent::try_from)
            .collect::<Result<Vec<_>>>()
            .unwrap()
    }
}

impl SkillEventSink for CollectingSink {
    fn emit(&mut self, cx: &mut Cx, event: Value) -> Result<()> {
        self.events.push(event.object().as_expr(cx)?);
        Ok(())
    }
}

fn skill_cx() -> Cx {
    let mut cx = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(0x5A11_0003),
    );
    install_skill_lib(&mut cx).unwrap();
    cx
}

fn grant_call_caps(cx: &mut Cx, id: &str) {
    cx.grant(skill_install_capability());
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability(id));
    cx.grant(skill_process_capability());
}

fn install_process_skill(
    cx: &mut Cx,
    transport: ProcessSkillTransport,
) -> Arc<ProcessSkillTransport> {
    cx.grant(skill_install_capability());
    let transport = Arc::new(transport);
    let transport_value = transport.clone().value(cx).unwrap();
    let cards = transport
        .discover(cx)
        .unwrap()
        .into_iter()
        .map(|card| card.value(cx))
        .collect::<Result<Vec<_>>>()
        .unwrap();
    let mut args = vec![transport_value];
    args.extend(cards);
    cx.call_function(&skill_install_symbol(), Args::new(args))
        .unwrap();
    transport
}

fn json_transport() -> ProcessSkillTransport {
    let mut transport = ProcessSkillTransport::new("proc");
    transport.insert(ProcessSkillSpec {
        id: "proc.answer".to_owned(),
        operation: "answer".to_owned(),
        title: "Process Answer".to_owned(),
        description: "Return a typed JSON result through process JSON stdio.".to_owned(),
        command_template: r#"printf '{"answer":42,"status":"ok"}'"#.to_owned(),
        protocol: ProcessSkillProtocol::JsonStdio,
        input_shape: string_arg_shape("process-json-args"),
        output_shape: answer_shape(),
        roles: vec![SkillRole::Tool],
        timeout: Duration::from_secs(1),
        max_output_bytes: 1024,
    });
    transport
}

fn secret_transport() -> ProcessSkillTransport {
    let mut transport = ProcessSkillTransport::new("proc");
    transport.insert(ProcessSkillSpec {
        id: "proc.secret".to_owned(),
        operation: "secret".to_owned(),
        title: "Secret Process".to_owned(),
        description: "Fail without exposing command or inputs.".to_owned(),
        command_template: "printf ignored >/dev/null; exit 7 # secret-command".to_owned(),
        protocol: ProcessSkillProtocol::JsonStdio,
        input_shape: string_arg_shape("process-secret-args"),
        output_shape: answer_shape(),
        roles: vec![SkillRole::Tool],
        timeout: Duration::from_secs(1),
        max_output_bytes: 1024,
    });
    transport
}

fn line_transport() -> ProcessSkillTransport {
    let mut transport = ProcessSkillTransport::new("proc");
    transport.insert(ProcessSkillSpec {
        id: "proc.lines".to_owned(),
        operation: "lines".to_owned(),
        title: "Line Process".to_owned(),
        description: "Stream line text as model events.".to_owned(),
        command_template: "printf 'one\\ntwo\\n'".to_owned(),
        protocol: ProcessSkillProtocol::LineText,
        input_shape: string_arg_shape("process-line-args"),
        output_shape: string_shape("process-line-result"),
        roles: vec![SkillRole::Tool, SkillRole::Model],
        timeout: Duration::from_secs(1),
        max_output_bytes: 1024,
    });
    transport
}

fn string_arg_shape(name: &str) -> Value {
    shape_value(
        Symbol::qualified("skill-process", name),
        Arc::new(ListShape::new(vec![Arc::new(ExprKindShape::new(
            ExprKind::String,
        ))])),
    )
}

fn string_shape(name: &str) -> Value {
    shape_value(
        Symbol::qualified("skill-process", name),
        Arc::new(ExprKindShape::new(ExprKind::String)),
    )
}

fn answer_shape() -> Value {
    shape_value(
        Symbol::qualified("skill-process", "answer-result"),
        Arc::new(FieldShape::anonymous(vec![
            FieldSpec::required(Symbol::new("answer"), Arc::new(NumberValueShape)),
            FieldSpec::required(
                Symbol::new("status"),
                Arc::new(ExprKindShape::new(ExprKind::String)),
            ),
        ])),
    )
}

fn map_string(expr: &Expr, key: &str) -> Option<String> {
    map_field(expr, key).and_then(|value| match value {
        Expr::String(text) => Some(text.clone()),
        _ => None,
    })
}

fn map_number(expr: &Expr, key: &str) -> Option<String> {
    map_field(expr, key).and_then(|value| match value {
        Expr::Number(number) => Some(number.canonical.clone()),
        _ => None,
    })
}

use sim_value::access::field as map_field;
