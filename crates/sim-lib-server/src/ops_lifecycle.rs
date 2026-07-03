use std::sync::Arc;

use sim_kernel::{Cx, Error, Expr, RawArgs, Result, Symbol, Value};

use crate::helpers::{
    clone_server_cx, connect_target_value, default_server_codec, ensure_installed_codec,
    installed_server_codecs, keyword, symbol_of, usize_from_value,
};
use crate::isolation::IsolatedEvalSite;
use crate::transport::{open_server_transport, require_start_capabilities, start_server_transport};
use crate::{
    Coroutine, CoroutineEvalSite, EvalSite, LocalEvalSite, Server, ServerAddress, ServerRuntime,
    ThreadMode, isolation::IsolationPolicy,
};

pub(crate) fn server_start(cx: &mut Cx, args: RawArgs) -> Result<Value> {
    let options = args.into_exprs();
    if !options.len().is_multiple_of(2) {
        return Err(Error::Eval(
            "server/start options must be key/value pairs".to_owned(),
        ));
    }

    let mut address = ServerAddress::Local;
    let mut codec: Option<Symbol> = None;
    let mut thread = ThreadMode::Coop;
    let mut isolation = IsolationPolicy::default();
    let mut name = None;
    let mut site = None;
    let mut max_inflight = crate::transport::DEFAULT_MAX_INFLIGHT_FRAMES;
    let mut saw_address = false;
    let mut saw_codec = false;
    let mut saw_thread = false;
    let mut saw_isolate = false;
    let mut saw_max_inflight = false;
    let mut spec = Vec::new();

    for pair in options.chunks(2) {
        let key = keyword(&pair[0])?;
        match key.as_str() {
            "address" => {
                address = ServerAddress::from_expr(&pair[1])?;
                saw_address = true;
            }
            "codec" => {
                let parsed = symbol_of(&pair[1], "server/start :codec expects a symbol")?;
                ensure_installed_codec(cx, &parsed)?;
                codec = Some(parsed);
                saw_codec = true;
            }
            "thread" => {
                thread = ThreadMode::from_expr(&pair[1])?;
                saw_thread = true;
            }
            "isolate" => {
                isolation = IsolationPolicy::from_expr(&pair[1])?;
                saw_isolate = true;
            }
            "name" => {
                name = Some(symbol_of(&pair[1], "server/start :name expects a symbol")?);
            }
            "site" => {
                site = Some(cx.eval_expr(pair[1].clone())?);
            }
            "max-inflight" => {
                let value = cx.eval_expr(pair[1].clone())?;
                max_inflight =
                    usize_from_value(cx, value, "server/start :max-inflight expects an integer")?;
                saw_max_inflight = true;
            }
            "capable" | "session" => {}
            other => {
                return Err(Error::Eval(format!(
                    "server/start: unknown option :{other}"
                )));
            }
        }
        spec.push((Symbol::new(key), pair[1].clone()));
    }

    require_start_capabilities(cx, &address)?;
    if !address.transport_available() {
        return Err(Error::Eval(
            "server/start: no transport for that address yet".to_owned(),
        ));
    }
    if !thread.is_available_now() {
        return Err(Error::Eval(
            "server/start: coroutine thread mode supports only main or coop base".to_owned(),
        ));
    }

    let supported = installed_server_codecs(cx);
    let default_codec = match codec {
        Some(codec) => codec,
        None => default_server_codec(&supported)?,
    };
    if !saw_address {
        spec.push((Symbol::new("address"), Expr::Symbol(Symbol::new("local"))));
    }
    if !saw_codec {
        spec.push((Symbol::new("codec"), Expr::Symbol(default_codec.clone())));
    }
    if !saw_thread {
        spec.push((Symbol::new("thread"), thread.as_expr()));
    }
    if !saw_isolate {
        spec.push((Symbol::new("isolate"), Expr::Nil));
    }
    if !saw_max_inflight {
        spec.push((
            Symbol::new("max-inflight"),
            Expr::String(max_inflight.to_string()),
        ));
    }

    let runtime_transport = open_server_transport(address.clone())?;
    if let Some(transport) = runtime_transport.as_ref() {
        address = transport.address().clone();
    }
    let coroutine_backed =
        matches!(address, ServerAddress::Coroutine { .. }) || thread.is_coroutine();
    let site: Arc<dyn EvalSite> = if coroutine_backed {
        let handler = site.ok_or_else(|| {
            Error::Eval("server/start: coroutine servers require :site".to_owned())
        })?;
        if handler.object().as_callable().is_none() {
            return Err(Error::TypeMismatch {
                expected: "callable",
                found: "non-callable",
            });
        }
        let coroutine = Arc::new(Coroutine::new(address.clone(), handler));
        Arc::new(CoroutineEvalSite::new(
            address.clone(),
            supported.clone(),
            coroutine,
        ))
    } else {
        if let Some(target) = site {
            connect_target_value(
                cx,
                target,
                None,
                &[],
                None,
                false,
                IsolationPolicy::default(),
            )?
            .site()
            .clone()
        } else {
            Arc::new(LocalEvalSite::new(address.clone(), supported.clone()))
        }
    };
    let site = IsolatedEvalSite::wrap(site, isolation.clone());
    let runtime = runtime_transport.map(|transport| {
        Arc::new(ServerRuntime::new_with_isolation(
            transport,
            clone_server_cx(cx),
            thread.clone(),
            max_inflight,
            isolation.clone(),
        ))
    });
    let server = Server::with_runtime(
        address,
        default_codec,
        supported,
        thread,
        isolation,
        name,
        site,
        spec,
        runtime,
    )?;
    start_server_transport(&server)?;
    cx.factory().opaque(Arc::new(server))
}
