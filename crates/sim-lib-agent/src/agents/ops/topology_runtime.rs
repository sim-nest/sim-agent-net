use crate::{Agent, AgentComponent, installed_codecs};
use sim_kernel::{
    Args, Callable, ClassRef, Cx, Error, EvalRequest, Expr, NumberLiteral, Object, Result, Symbol,
    Value,
};
use sim_lib_server::{
    Connection, EvalSite, FrameEnvelope, FrameKind, ServerAddress, ServerFrame,
    eval_reply_from_frame, server_frame_from_request, stream_frame_to_expr,
};
use std::{any::Any, sync::Arc};

pub(super) struct TopologyUntilFn;

pub(super) fn local_connection(
    cx: &mut Cx,
    site: Arc<dyn EvalSite>,
    role: Option<Symbol>,
) -> Result<Connection> {
    let codecs = installed_codecs(cx);
    Connection::with_session(
        ServerAddress::Local,
        first_codec(&codecs),
        codecs,
        site,
        role,
        sim_lib_server::IsolationPolicy::default(),
    )
}

pub(super) fn evaluate_connection(
    cx: &mut Cx,
    connection: &Connection,
    expr: Expr,
    role: Option<Symbol>,
    parent: &FrameEnvelope,
) -> Result<ServerFrame> {
    let request = EvalRequest {
        expr,
        mode: sim_kernel::EvalMode::Eval,
        result_shape: None,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        required_capabilities: parent.required_capabilities.clone(),
        deadline: parent.deadline,
        consistency: parent.consistency,
        trace: parent.trace,
    };
    let mut frame = server_frame_from_request(cx, connection.default_codec(), request)?;
    frame.envelope.reply_codec_hint = parent.reply_codec_hint.clone();
    frame.envelope.trigger_source = parent.trigger_source.clone();
    frame.envelope.role = role.or_else(|| connection.role().cloned());
    frame.envelope.hop = parent.hop.saturating_add(1);
    connection.site().answer(cx, frame)
}

pub(super) fn reply_expr(cx: &mut Cx, frame: &ServerFrame) -> Result<Expr> {
    eval_reply_from_frame(cx, frame)?.value.object().as_expr(cx)
}

pub(super) fn is_stream_passthrough_frame(cx: &mut Cx, frame: &ServerFrame) -> Result<bool> {
    match frame.kind {
        FrameKind::Request | FrameKind::Response => Ok(false),
        _ => match stream_frame_to_expr(cx, frame) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        },
    }
}

pub(super) fn first_codec(codecs: &[Symbol]) -> Symbol {
    codecs
        .first()
        .cloned()
        .unwrap_or_else(|| Symbol::qualified("codec", "binary"))
}

pub(super) fn until_value(cx: &mut Cx) -> Result<Value> {
    cx.factory().opaque(Arc::new(TopologyUntilFn))
}

pub(super) fn number_expr(value: impl ToString) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

pub(super) use sim_value::access::field as map_field;

pub(super) fn bool_field(expr: &Expr, key: &str) -> bool {
    matches!(map_field(expr, key), Some(Expr::Bool(true)))
}

pub(super) fn expr_field(expr: &Expr, key: &str) -> Result<Expr> {
    map_field(expr, key)
        .cloned()
        .ok_or_else(|| Error::Eval(format!("missing {key} field")))
}

pub(super) fn number_field(expr: &Expr, key: &str) -> Result<f64> {
    match expr_field(expr, key)? {
        Expr::Number(number) => number
            .canonical
            .parse::<f64>()
            .map_err(|_| Error::Eval(format!("field {key} was not numeric"))),
        Expr::String(text) => text
            .parse::<f64>()
            .map_err(|_| Error::Eval(format!("field {key} was not numeric"))),
        other => Err(Error::Eval(format!(
            "field {key} was not numeric: {other:?}"
        ))),
    }
}

pub(super) fn text_label(value: &Value) -> String {
    if let Some(agent) = value.object().downcast_ref::<Agent>() {
        return agent.name.to_string();
    }
    if let Some(component) = value.object().downcast_ref::<AgentComponent>() {
        return component.symbol.to_string();
    }
    if let Some(connection) = value.object().downcast_ref::<Connection>()
        && let Some(role) = connection.role()
    {
        return role.to_string();
    }
    "member".to_owned()
}

impl Callable for TopologyUntilFn {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let [state] = args.values() else {
            return Err(Error::Eval(
                "topology until predicate expects one state".to_owned(),
            ));
        };
        let done = bool_field(&state.object().as_expr(cx)?, "done");
        cx.factory().bool(done)
    }
}

impl Object for TopologyUntilFn {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<topology-until>".to_owned())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for TopologyUntilFn {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory().class_stub(
            sim_kernel::ClassId(0),
            Symbol::qualified("topology", "Until"),
        )
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}
