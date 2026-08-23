use crate::memory::flatten_expr_text;
use crate::tools::register_tool;
use crate::{AgentComponent, Component, ComponentKind, Tool};
use sim_codec_binary::BinaryCodecLib;
use sim_codec_json::JsonCodecLib;
use sim_codec_lisp::LispCodecLib;
use sim_kernel::{
    Cx, DefaultFactory, EagerPolicy, Error, Expr, Object, ReadPolicy, Result, Symbol, Value,
};
use sim_lib_server::{EvalSite, FrameKind, ServerAddress, ServerFrame};
use sim_shape::{AnyShape, shape_value};
use std::{
    any::Any,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) use crate::install_agent_lib;

pub(super) fn eval_cx() -> Cx {
    Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(1),
    )
}

pub(super) fn install_test_codec(cx: &mut Cx) {
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
}

pub(super) fn install_roundtrip_codecs(cx: &mut Cx) {
    install_test_codec(cx);
    let json = JsonCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&json).unwrap();
    let lisp = LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
}

pub(super) fn temp_memory_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("sim-lib-agent-{label}-{nonce}.bin"))
}

pub(super) fn temp_text_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("sim-lib-agent-{label}-{nonce}.txt"))
}

pub(super) fn request_frame(cx: &mut Cx, expr: Expr) -> ServerFrame {
    sim_lib_server::server_frame_from_request(
        cx,
        &Symbol::qualified("codec", "binary"),
        sim_kernel::EvalRequest {
            expr,
            mode: sim_kernel::EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            required_capabilities: Vec::new(),
            deadline: None,
            consistency: sim_kernel::Consistency::LocalFirst,
            trace: false,
        },
    )
    .unwrap()
}

pub(super) fn as_component(value: &Value) -> &AgentComponent {
    value.object().downcast_ref::<AgentComponent>().unwrap()
}

pub(super) fn flatten_text(expr: &Expr) -> String {
    flatten_expr_text(expr)
}

#[derive(Clone)]
pub(super) struct SumFn;

impl Object for SumFn {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<function test/sum>".to_owned())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for SumFn {
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

impl sim_kernel::Callable for SumFn {
    fn call(&self, cx: &mut Cx, args: sim_kernel::Args) -> Result<Value> {
        let mut sum = 0.0;
        for value in args.values() {
            match value.object().as_expr(cx)? {
                Expr::Number(number) => {
                    sum += number.canonical.parse::<f64>().unwrap();
                }
                other => {
                    return Err(Error::Eval(format!("expected number arg, found {other:?}")));
                }
            }
        }
        cx.factory()
            .number_literal(Symbol::qualified("numbers", "f64"), sum.to_string())
    }
}

pub(super) struct DummyComponent {
    name: Symbol,
    capabilities: Vec<sim_kernel::CapabilityName>,
}

impl DummyComponent {
    pub(super) fn new() -> Self {
        Self {
            name: Symbol::qualified("test", "component"),
            capabilities: vec![crate::net_http_capability()],
        }
    }
}

impl EvalSite for DummyComponent {
    fn site_kind(&self) -> &'static str {
        "dummy"
    }

    fn address(&self) -> &ServerAddress {
        static LOCAL_ADDRESS: ServerAddress = ServerAddress::Local;
        &LOCAL_ADDRESS
    }

    fn codecs(&self) -> &[Symbol] {
        &[]
    }

    fn answer(&self, _cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        Ok(frame)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Component for DummyComponent {
    fn kind(&self) -> ComponentKind {
        ComponentKind::Tool
    }

    fn name(&self) -> &Symbol {
        &self.name
    }

    fn capabilities(&self) -> &[sim_kernel::CapabilityName] {
        &self.capabilities
    }

    fn reflect(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(Expr::Symbol(self.name.clone()))
    }
}

pub(super) fn register_sum_tool(cx: &mut Cx) -> Arc<Tool> {
    let callable = cx.factory().opaque(Arc::new(SumFn)).unwrap();
    let args_shape = shape_value(Symbol::qualified("test", "sum-args"), Arc::new(AnyShape));
    let result_shape = shape_value(Symbol::qualified("test", "sum-result"), Arc::new(AnyShape));
    let tool = Arc::new(Tool {
        symbol: Symbol::qualified("test", "sum"),
        description: "sum two numbers".to_owned(),
        args_shape,
        result_shape: Some(result_shape),
        category: Symbol::new("math"),
        capabilities: vec![sim_kernel::CapabilityName::new("math")],
        function: callable,
        address: ServerAddress::Local,
        codecs: vec![Symbol::qualified("codec", "binary")],
    });
    let value = cx.factory().opaque(tool.clone()).unwrap();
    register_tool(cx, tool.clone(), value).unwrap();
    tool
}

#[test]
fn support_install_helper_keeps_agent_lib_available() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    assert!(cx.registry().lib(&Symbol::new("agent")).is_some());
    assert_eq!(request_frame(&mut cx, Expr::Nil).kind, FrameKind::Request);
    assert_eq!(ReadPolicy::default(), ReadPolicy::default());
}
