use crate::tools::{Tool, register_tool};
use sim_kernel::{Args, Cx, Expr, Object, Result, Symbol, Value};
use sim_lib_server::{
    Connection, EvalSite, FrameKind, ServerAddress, ServerFrame, eval_request_from_frame,
};
use sim_shape::{AnyShape, shape_value};
use std::{any::Any, sync::Arc};

pub(super) fn tagged_append_connection(label: &'static str) -> Connection {
    Connection::new(
        ServerAddress::Local,
        Symbol::qualified("codec", "binary"),
        vec![Symbol::qualified("codec", "binary")],
        Arc::new(AppendRoleSite { label }),
    )
    .unwrap()
}

pub(super) fn register_connection(cx: &mut Cx, symbol: Symbol, connection: Connection) {
    let value = cx.factory().opaque(Arc::new(connection)).unwrap();
    cx.registry_mut().register_value(symbol, value).unwrap();
}

pub(super) fn fixed_reply_connection(reply: &'static str) -> Connection {
    Connection::new(
        ServerAddress::Local,
        Symbol::qualified("codec", "binary"),
        vec![Symbol::qualified("codec", "binary")],
        Arc::new(FixedReplySite { reply }),
    )
    .unwrap()
}

pub(super) fn star_hub_connection() -> Connection {
    Connection::new(
        ServerAddress::Local,
        Symbol::qualified("codec", "binary"),
        vec![Symbol::qualified("codec", "binary")],
        Arc::new(StarHubSite),
    )
    .unwrap()
}

pub(super) fn verifier_connection(first: &'static str, second: &'static str) -> Connection {
    Connection::new(
        ServerAddress::Local,
        Symbol::qualified("codec", "binary"),
        vec![Symbol::qualified("codec", "binary")],
        Arc::new(VerifierSite { first, second }),
    )
    .unwrap()
}

#[derive(Clone)]
struct AppendRoleSite {
    label: &'static str,
}

impl EvalSite for AppendRoleSite {
    fn site_kind(&self) -> &'static str {
        "append-role"
    }

    fn address(&self) -> &ServerAddress {
        static LOCAL: ServerAddress = ServerAddress::Local;
        &LOCAL
    }

    fn codecs(&self) -> &[Symbol] {
        &[]
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let mut items = match eval_request_from_frame(cx, &frame)?.expr {
            Expr::List(items) => items,
            other => vec![other],
        };
        let role = frame
            .envelope
            .role
            .clone()
            .unwrap_or_else(|| Symbol::new("none"));
        items.push(Expr::String(format!("{}:{}", self.label, role)));
        let value = cx.factory().expr(Expr::List(items))?;
        let diagnostics = cx.take_diagnostics();
        sim_lib_server::server_frame_from_reply(
            cx,
            &frame.codec,
            sim_kernel::EvalReply {
                value,
                diagnostics,
                trace: None,
            },
            frame.envelope.consistency,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
struct FixedReplySite {
    reply: &'static str,
}

impl EvalSite for FixedReplySite {
    fn site_kind(&self) -> &'static str {
        "fixed-reply"
    }

    fn address(&self) -> &ServerAddress {
        static LOCAL: ServerAddress = ServerAddress::Local;
        &LOCAL
    }

    fn codecs(&self) -> &[Symbol] {
        &[]
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        if frame.kind != FrameKind::Request {
            return Err(sim_kernel::Error::Eval("expected request".to_owned()));
        }
        let value = cx.factory().string(self.reply.to_owned())?;
        let diagnostics = cx.take_diagnostics();
        sim_lib_server::server_frame_from_reply(
            cx,
            &frame.codec,
            sim_kernel::EvalReply {
                value,
                diagnostics,
                trace: None,
            },
            frame.envelope.consistency,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
struct StarHubSite;

impl EvalSite for StarHubSite {
    fn site_kind(&self) -> &'static str {
        "star-hub"
    }

    fn address(&self) -> &ServerAddress {
        static LOCAL: ServerAddress = ServerAddress::Local;
        &LOCAL
    }

    fn codecs(&self) -> &[Symbol] {
        &[]
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let expr = eval_request_from_frame(cx, &frame)?.expr;
        let reply = match map_expr_field_opt(&expr, "spoke-replies") {
            Some(Expr::List(items)) => Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("merge-count")),
                    Expr::String(items.len().to_string()),
                ),
                (Expr::Symbol(Symbol::new("seen")), Expr::List(items.clone())),
            ]),
            _ => Expr::Map(vec![(Expr::Symbol(Symbol::new("hub")), expr)]),
        };
        let value = cx.factory().expr(reply)?;
        let diagnostics = cx.take_diagnostics();
        sim_lib_server::server_frame_from_reply(
            cx,
            &frame.codec,
            sim_kernel::EvalReply {
                value,
                diagnostics,
                trace: None,
            },
            frame.envelope.consistency,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
struct VerifierSite {
    first: &'static str,
    second: &'static str,
}

impl EvalSite for VerifierSite {
    fn site_kind(&self) -> &'static str {
        "verifier"
    }

    fn address(&self) -> &ServerAddress {
        static LOCAL: ServerAddress = ServerAddress::Local;
        &LOCAL
    }

    fn codecs(&self) -> &[Symbol] {
        &[]
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let expr = eval_request_from_frame(cx, &frame)?.expr;
        let reply = if map_expr_field_opt(&expr, "answer").is_some() {
            self.first
        } else {
            self.second
        };
        let value = cx.factory().string(reply.to_owned())?;
        let diagnostics = cx.take_diagnostics();
        sim_lib_server::server_frame_from_reply(
            cx,
            &frame.codec,
            sim_kernel::EvalReply {
                value,
                diagnostics,
                trace: None,
            },
            frame.envelope.consistency,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
struct BidWorkerFn {
    bid: f64,
    reply: &'static str,
}

impl Object for BidWorkerFn {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<function test/bid-worker>".to_owned())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for BidWorkerFn {
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

impl sim_kernel::Callable for BidWorkerFn {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let is_bid = args.values().first().is_some_and(|value| {
            matches!(value.object().as_expr(cx), Ok(Expr::Symbol(symbol)) if symbol.name.as_ref() == "bid")
        });
        if is_bid {
            return cx
                .factory()
                .number_literal(Symbol::qualified("numbers", "f64"), self.bid.to_string());
        }
        cx.factory().string(self.reply.to_owned())
    }
}

pub(super) fn register_bid_worker(
    cx: &mut Cx,
    symbol: Symbol,
    bid: f64,
    reply: &'static str,
) -> Arc<Tool> {
    let callable = cx
        .factory()
        .opaque(Arc::new(BidWorkerFn { bid, reply }))
        .unwrap();
    let tool = Arc::new(Tool {
        symbol: symbol.clone(),
        description: "bid worker".to_owned(),
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

pub(super) fn map_expr_field(expr: &Expr, key: &str) -> Expr {
    map_expr_field_opt(expr, key)
        .unwrap_or_else(|| panic!("missing key {key} in {expr:?}"))
        .clone()
}

fn map_expr_field_opt<'a>(expr: &'a Expr, key: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries
        .iter()
        .find_map(|(entry_key, entry_value)| match entry_key {
            Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(entry_value),
            _ => None,
        })
}
