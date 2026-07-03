use std::sync::Arc;

use crate::{
    LoopEvalSite, ServerStatus, connection_arg, coroutine_target_from_value, server_arg,
    server_cancel_coroutine, server_connect, server_coroutine_status, server_from_value,
    server_lisp, server_loop, server_notify, server_pipeline, server_realize, server_receive,
    server_repl, server_request, server_resume_exprs, server_send, server_start, server_start_loop,
    server_stream, server_stream_next, server_trigger, server_trigger_poll, server_wasm_region,
    server_yield, shutdown_server_transport, stream_handle_arg,
};
use sim_kernel::{
    Args, CORE_FUNCTION_CLASS_ID, Callable, ClassRef, Cx, Error, Linker, Object, RawArgs, Result,
    Symbol, Value,
};

#[derive(Clone, Copy)]
enum ServerFnKind {
    Start,
    Stop,
    StopForce,
    Close,
    Suspend,
    Resume,
    Connect,
    Pipeline,
    Loop,
    StartLoop,
    Yield,
    Send,
    Request,
    Notify,
    Receive,
    CoroutineStatus,
    CancelCoroutine,
    Stream,
    StreamNext,
    StreamDone,
    StreamCancel,
    AsFabric,
    Realize,
    Health,
    Sessions,
    Lisp,
    Reflect,
    Repl,
    Trigger,
    TriggerPoll,
    WasmRegion,
}

#[derive(Clone)]
struct ServerFunction {
    symbol: Symbol,
    kind: ServerFnKind,
}

pub(crate) fn server_exports() -> Vec<sim_kernel::Export> {
    [
        Symbol::qualified("server", "start"),
        Symbol::qualified("server", "stop"),
        Symbol::qualified("server", "stop!"),
        Symbol::qualified("server", "close"),
        Symbol::qualified("server", "suspend"),
        Symbol::qualified("server", "resume"),
        Symbol::qualified("server", "connect"),
        Symbol::qualified("server", "pipeline"),
        Symbol::qualified("server", "loop"),
        Symbol::qualified("server", "start-loop"),
        Symbol::qualified("server", "yield"),
        Symbol::qualified("server", "send"),
        Symbol::qualified("server", "request"),
        Symbol::qualified("server", "notify"),
        Symbol::qualified("server", "receive"),
        Symbol::qualified("server", "coroutine-status"),
        Symbol::qualified("server", "cancel-coroutine"),
        Symbol::qualified("server", "stream"),
        Symbol::qualified("server", "stream-next"),
        Symbol::qualified("server", "stream-done?"),
        Symbol::qualified("server", "stream-cancel"),
        Symbol::qualified("server", "as-fabric"),
        Symbol::qualified("server", "realize"),
        Symbol::qualified("server", "health"),
        Symbol::qualified("server", "sessions"),
        Symbol::qualified("server", "lisp"),
        Symbol::qualified("server", "reflect"),
        Symbol::qualified("server", "repl"),
        Symbol::qualified("server", "trigger"),
        Symbol::qualified("server", "trigger-poll"),
        Symbol::qualified("server", "wasm-region"),
    ]
    .into_iter()
    .map(|symbol| sim_kernel::Export::Function {
        symbol,
        function_id: None,
    })
    .collect()
}

pub(crate) fn register_server_functions(
    cx: &mut sim_kernel::LoadCx,
    linker: &mut Linker<'_>,
) -> Result<()> {
    for (symbol, kind) in [
        (Symbol::qualified("server", "start"), ServerFnKind::Start),
        (Symbol::qualified("server", "stop"), ServerFnKind::Stop),
        (
            Symbol::qualified("server", "stop!"),
            ServerFnKind::StopForce,
        ),
        (Symbol::qualified("server", "close"), ServerFnKind::Close),
        (
            Symbol::qualified("server", "suspend"),
            ServerFnKind::Suspend,
        ),
        (Symbol::qualified("server", "resume"), ServerFnKind::Resume),
        (
            Symbol::qualified("server", "connect"),
            ServerFnKind::Connect,
        ),
        (
            Symbol::qualified("server", "pipeline"),
            ServerFnKind::Pipeline,
        ),
        (Symbol::qualified("server", "loop"), ServerFnKind::Loop),
        (
            Symbol::qualified("server", "start-loop"),
            ServerFnKind::StartLoop,
        ),
        (Symbol::qualified("server", "yield"), ServerFnKind::Yield),
        (Symbol::qualified("server", "send"), ServerFnKind::Send),
        (
            Symbol::qualified("server", "request"),
            ServerFnKind::Request,
        ),
        (Symbol::qualified("server", "notify"), ServerFnKind::Notify),
        (
            Symbol::qualified("server", "receive"),
            ServerFnKind::Receive,
        ),
        (
            Symbol::qualified("server", "coroutine-status"),
            ServerFnKind::CoroutineStatus,
        ),
        (
            Symbol::qualified("server", "cancel-coroutine"),
            ServerFnKind::CancelCoroutine,
        ),
        (Symbol::qualified("server", "stream"), ServerFnKind::Stream),
        (
            Symbol::qualified("server", "stream-next"),
            ServerFnKind::StreamNext,
        ),
        (
            Symbol::qualified("server", "stream-done?"),
            ServerFnKind::StreamDone,
        ),
        (
            Symbol::qualified("server", "stream-cancel"),
            ServerFnKind::StreamCancel,
        ),
        (
            Symbol::qualified("server", "as-fabric"),
            ServerFnKind::AsFabric,
        ),
        (
            Symbol::qualified("server", "realize"),
            ServerFnKind::Realize,
        ),
        (Symbol::qualified("server", "health"), ServerFnKind::Health),
        (
            Symbol::qualified("server", "sessions"),
            ServerFnKind::Sessions,
        ),
        (Symbol::qualified("server", "lisp"), ServerFnKind::Lisp),
        (
            Symbol::qualified("server", "reflect"),
            ServerFnKind::Reflect,
        ),
        (Symbol::qualified("server", "repl"), ServerFnKind::Repl),
        (
            Symbol::qualified("server", "trigger"),
            ServerFnKind::Trigger,
        ),
        (
            Symbol::qualified("server", "trigger-poll"),
            ServerFnKind::TriggerPoll,
        ),
        (
            Symbol::qualified("server", "wasm-region"),
            ServerFnKind::WasmRegion,
        ),
    ] {
        let function_id = linker.function(symbol.clone())?;
        let value = cx
            .factory()
            .opaque(Arc::new(ServerFunction { symbol, kind }))?;
        linker.bind_function_value(function_id, value)?;
    }
    Ok(())
}

impl Object for ServerFunction {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<function {}>", self.symbol))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for ServerFunction {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for ServerFunction {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        match self.kind {
            ServerFnKind::Start => Err(Error::Eval(
                "server/start must be called with keyword expressions".to_owned(),
            )),
            ServerFnKind::Stop | ServerFnKind::StopForce => {
                let server = server_arg(&args, 0, "server/stop expects one server value")?;
                server.set_status(ServerStatus::Stopped);
                if let Some(runtime) = server.runtime() {
                    runtime.begin_stop();
                }
                server.stop_triggers()?;
                shutdown_server_transport(server)?;
                Ok(args.values()[0].clone())
            }
            ServerFnKind::Close => {
                let connection =
                    connection_arg(&args, 0, "server/close expects one connection value")?;
                connection.close(cx)?;
                Ok(args.values()[0].clone())
            }
            ServerFnKind::Suspend => {
                let server = server_arg(&args, 0, "server/suspend expects one server value")?;
                server.set_status(ServerStatus::Suspended);
                Ok(args.values()[0].clone())
            }
            ServerFnKind::Resume => {
                let Some(target) = args.values().first() else {
                    return Err(Error::Eval(
                        "server/resume expects a server or coroutine value".to_owned(),
                    ));
                };
                if let Some(server) = server_from_value(target) {
                    server.set_status(ServerStatus::Running);
                    return Ok(target.clone());
                }
                let coroutine = coroutine_target_from_value(target)?;
                if let Some(input) = args.values().get(1) {
                    return coroutine.resume(cx, input.clone());
                }
                Err(Error::Eval(
                    "server/resume expects an expression when targeting a coroutine".to_owned(),
                ))
            }
            ServerFnKind::Connect
            | ServerFnKind::Yield
            | ServerFnKind::Send
            | ServerFnKind::Request
            | ServerFnKind::Notify
            | ServerFnKind::Receive
            | ServerFnKind::CoroutineStatus
            | ServerFnKind::CancelCoroutine
            | ServerFnKind::Stream
            | ServerFnKind::StreamNext
            | ServerFnKind::Realize
            | ServerFnKind::Repl
            | ServerFnKind::Trigger
            | ServerFnKind::TriggerPoll
            | ServerFnKind::WasmRegion => Err(Error::Eval(format!(
                "{} must be called with expression arguments",
                self.symbol
            ))),
            ServerFnKind::StreamDone => {
                let handle =
                    stream_handle_arg(&args, 0, "server/stream-done? expects one stream value")?;
                cx.factory().bool(handle.is_done())
            }
            ServerFnKind::StreamCancel => {
                let handle =
                    stream_handle_arg(&args, 0, "server/stream-cancel expects one stream value")?;
                handle.cancel();
                cx.factory().nil()
            }
            ServerFnKind::AsFabric => {
                let connection =
                    connection_arg(&args, 0, "server/as-fabric expects one connection value")?;
                Ok(cx.factory().opaque(Arc::new(connection.clone()))?)
            }
            ServerFnKind::Health => {
                let server = server_arg(&args, 0, "server/health expects one server value")?;
                server.health_value(cx)
            }
            ServerFnKind::Sessions => {
                let server = server_arg(&args, 0, "server/sessions expects one server value")?;
                server.sessions_value(cx)
            }
            ServerFnKind::Lisp => {
                let server = server_arg(&args, 0, "server/lisp expects one server value")?;
                cx.factory().expr(server_lisp(server))
            }
            ServerFnKind::Reflect => {
                let server = server_arg(&args, 0, "server/reflect expects one server value")?;
                server.reflect_value(cx)
            }
            ServerFnKind::Pipeline | ServerFnKind::Loop => Err(Error::Eval(format!(
                "{} expects unevaluated option expressions",
                self.symbol
            ))),
            ServerFnKind::StartLoop => {
                let connection =
                    connection_arg(&args, 0, "server/start-loop expects one loop connection")?;
                if connection
                    .site()
                    .as_any()
                    .downcast_ref::<LoopEvalSite>()
                    .is_none()
                {
                    return Err(Error::TypeMismatch {
                        expected: "loop connection",
                        found: "non-loop connection",
                    });
                }
                Ok(cx.factory().opaque(Arc::new(connection.clone()))?)
            }
        }
    }

    fn call_exprs(&self, cx: &mut Cx, args: RawArgs) -> Result<Value> {
        match self.kind {
            ServerFnKind::Start => server_start(cx, args),
            ServerFnKind::Resume => server_resume_exprs(cx, args),
            ServerFnKind::Connect => server_connect(cx, args),
            ServerFnKind::Pipeline => server_pipeline(cx, args),
            ServerFnKind::Loop => server_loop(cx, args),
            ServerFnKind::StartLoop => server_start_loop(cx, args),
            ServerFnKind::Yield => server_yield(cx, args),
            ServerFnKind::Send => server_send(cx, args),
            ServerFnKind::Request => server_request(cx, args),
            ServerFnKind::Notify => server_notify(cx, args),
            ServerFnKind::Receive => server_receive(cx, args),
            ServerFnKind::CoroutineStatus => server_coroutine_status(cx, args),
            ServerFnKind::CancelCoroutine => server_cancel_coroutine(cx, args),
            ServerFnKind::Stream => server_stream(cx, args),
            ServerFnKind::StreamNext => server_stream_next(cx, args),
            ServerFnKind::Realize => server_realize(cx, args),
            ServerFnKind::Repl => server_repl(cx, args),
            ServerFnKind::Trigger => server_trigger(cx, args),
            ServerFnKind::TriggerPoll => server_trigger_poll(cx, args),
            ServerFnKind::WasmRegion => server_wasm_region(cx, args),
            _ => {
                let values = args
                    .into_exprs()
                    .into_iter()
                    .map(|expr| cx.eval_expr(expr))
                    .collect::<Result<Vec<_>>>()?;
                self.call(cx, Args::new(values))
            }
        }
    }
}
