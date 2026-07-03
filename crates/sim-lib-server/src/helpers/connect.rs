use std::sync::Arc;

use sim_kernel::{Cx, Error, EvalFabric, EvalReply, EvalRequest, Expr, Result, Symbol, Value};

use crate::{
    Connection, Coroutine, CoroutineEvalSite, FabricEvalSite, IsolatedEvalSite, IsolationPolicy,
    LocalEvalSite, LoopEvalSite, PipelineEvalSite, ResolvedAddress, Server, ServerAddress,
    address_resolvers, connect_transport_site, connect_transport_site_with_loopback,
};

use super::codecs::{
    choose_codec, default_connection_codec, installed_server_codecs, negotiation_offer_codecs,
};

pub(crate) fn resolve_on_target(cx: &mut Cx, target: Value) -> Result<Connection> {
    connect_target_value(
        cx,
        target,
        None,
        &[],
        None,
        false,
        IsolationPolicy::default(),
    )
}

pub(crate) fn connect_target_value(
    cx: &mut Cx,
    target: Value,
    codec: Option<Symbol>,
    preferred: &[Symbol],
    role: Option<Symbol>,
    loopback: bool,
    isolation: IsolationPolicy,
) -> Result<Connection> {
    if let Some(connection) = connection_from_value(&target) {
        return Ok(connection.clone());
    }
    if let Some(server) = server_from_value(&target) {
        if server.address().is_remote_like() {
            let mut offered = preferred.to_vec();
            if let Some(explicit) = codec.clone() {
                offered = vec![explicit];
            } else if offered.is_empty() {
                offered = negotiation_offer_codecs(&installed_server_codecs(cx));
            }
            let (site, selected_codec) = if loopback {
                connect_transport_site_with_loopback(
                    cx,
                    server.address().clone(),
                    offered.clone(),
                    true,
                )?
            } else {
                connect_transport_site(cx, server.address().clone(), offered.clone())?
            };
            let inherited_isolation = if isolation == IsolationPolicy::default() {
                server.isolation().clone()
            } else {
                isolation
            };
            return Connection::with_session(
                server.address().clone(),
                selected_codec,
                offered,
                site,
                role,
                inherited_isolation,
            );
        }
        let supported_codecs = server.supported_codecs().to_vec();
        let selected_codec = choose_codec(
            default_connection_codec(&supported_codecs)?,
            &supported_codecs,
            codec,
            preferred,
        )?;
        let inherited_isolation = if isolation == IsolationPolicy::default() {
            server.isolation().clone()
        } else {
            isolation
        };
        return Connection::with_session(
            server.address().clone(),
            selected_codec,
            supported_codecs,
            IsolatedEvalSite::wrap(server.site().clone(), inherited_isolation.clone()),
            role,
            inherited_isolation,
        );
    }
    if target.object().as_eval_fabric().is_some() {
        let supported_codecs = installed_server_codecs(cx);
        let default_codec = default_connection_codec(&supported_codecs)?;
        let selected_codec = choose_codec(default_codec, &supported_codecs, codec, preferred)?;
        let address = ServerAddress::Local;
        let site = Arc::new(FabricEvalSite::new(
            "fabric",
            address.clone(),
            supported_codecs.clone(),
            Arc::new(ValueEvalFabric {
                value: target.clone(),
            }),
        ));
        return Connection::with_session(
            address,
            selected_codec,
            supported_codecs,
            IsolatedEvalSite::wrap(site, isolation.clone()),
            role,
            isolation,
        );
    }

    let address = ServerAddress::from_expr(&target.object().as_expr(cx)?)?;
    if let Some(resolved) = resolve_registered_address(cx, &address, codec.clone(), preferred)? {
        return Connection::with_session(
            address,
            resolved.selected_codec,
            resolved.supported_codecs,
            IsolatedEvalSite::wrap(resolved.site, isolation.clone()),
            role,
            isolation,
        );
    }
    if let ServerAddress::Pipeline { steps } = &address {
        let connections = steps
            .iter()
            .map(|step| {
                connect_target_value(
                    cx,
                    cx.factory().expr(server_address_expr(step))?,
                    None,
                    &[],
                    None,
                    loopback,
                    isolation.clone(),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        return pipeline_connection_from_existing(
            cx,
            connections,
            codec,
            preferred,
            role,
            isolation,
        );
    }
    if address.is_remote_like() {
        let mut offered = preferred.to_vec();
        if let Some(explicit) = codec.clone() {
            offered = vec![explicit];
        } else if offered.is_empty() {
            offered = negotiation_offer_codecs(&installed_server_codecs(cx));
        }
        let (site, selected_codec) = if loopback {
            connect_transport_site_with_loopback(cx, address.clone(), offered.clone(), true)?
        } else {
            connect_transport_site(cx, address.clone(), offered.clone())?
        };
        return Connection::with_session(
            address,
            selected_codec,
            offered,
            IsolatedEvalSite::wrap(site, isolation.clone()),
            role,
            isolation,
        );
    }
    let supported_codecs = installed_server_codecs(cx);
    let default_codec = default_connection_codec(&supported_codecs)?;
    let selected_codec = choose_codec(default_codec, &supported_codecs, codec, preferred)?;
    let site = Arc::new(LocalEvalSite::new(
        address.clone(),
        supported_codecs.clone(),
    ));
    Connection::with_session(
        address,
        selected_codec,
        supported_codecs,
        IsolatedEvalSite::wrap(site, isolation.clone()),
        role,
        isolation,
    )
}

fn resolve_registered_address(
    cx: &mut Cx,
    address: &ServerAddress,
    codec: Option<Symbol>,
    preferred: &[Symbol],
) -> Result<Option<ResolvedAddress>> {
    let resolver = {
        let resolvers = address_resolvers()
            .lock()
            .map_err(|_| Error::HostError("address resolver registry mutex poisoned".to_owned()))?;
        resolvers.get(address.kind_symbol().name.as_ref()).copied()
    };
    let Some(resolver) = resolver else {
        return Ok(None);
    };
    let mut offered = preferred.to_vec();
    if let Some(explicit) = codec {
        offered = vec![explicit];
    } else if offered.is_empty() {
        offered = negotiation_offer_codecs(&installed_server_codecs(cx));
    }
    resolver(cx, address, &offered)
}

pub(crate) fn pipeline_connection_from_steps(
    cx: &mut Cx,
    steps: Vec<Connection>,
    codec: Option<Symbol>,
) -> Result<Connection> {
    pipeline_connection_from_existing(cx, steps, codec, &[], None, IsolationPolicy::default())
}

pub(crate) fn loop_connection_from_steps(
    cx: &mut Cx,
    steps: Vec<Connection>,
    codec: Option<Symbol>,
    max_iterations: usize,
    until: Value,
) -> Result<Connection> {
    if steps.is_empty() {
        return Err(Error::Eval(
            "server/loop requires at least one step".to_owned(),
        ));
    }
    let supported_codecs = installed_server_codecs(cx);
    let default_codec = default_connection_codec(&supported_codecs)?;
    let selected_codec = choose_codec(default_codec, &supported_codecs, codec, &[])?;
    let address = ServerAddress::Pipeline {
        steps: steps.iter().map(|step| step.address().clone()).collect(),
    };
    let site = Arc::new(LoopEvalSite::new(
        address.clone(),
        supported_codecs.clone(),
        steps,
        max_iterations,
        until,
    ));
    Connection::with_session(
        address,
        selected_codec,
        supported_codecs,
        site,
        None,
        IsolationPolicy::default(),
    )
}

fn pipeline_connection_from_existing(
    cx: &mut Cx,
    steps: Vec<Connection>,
    codec: Option<Symbol>,
    preferred: &[Symbol],
    role: Option<Symbol>,
    isolation: IsolationPolicy,
) -> Result<Connection> {
    if steps.is_empty() {
        return Err(Error::Eval(
            "server/pipeline requires at least one step".to_owned(),
        ));
    }
    let supported_codecs = installed_server_codecs(cx);
    let default_codec = default_connection_codec(&supported_codecs)?;
    let selected_codec = choose_codec(default_codec, &supported_codecs, codec, preferred)?;
    let address = ServerAddress::Pipeline {
        steps: steps.iter().map(|step| step.address().clone()).collect(),
    };
    let site = Arc::new(PipelineEvalSite::new(
        address.clone(),
        supported_codecs.clone(),
        steps,
    ));
    Connection::with_session(
        address,
        selected_codec,
        supported_codecs,
        site,
        role,
        isolation,
    )
}

pub(crate) fn pipeline_steps_from_expr(cx: &mut Cx, expr: Expr) -> Result<Vec<Connection>> {
    let items = match expr {
        Expr::List(items) | Expr::Vector(items) => items,
        Expr::Call { operator, args } if matches!(*operator, Expr::Symbol(ref symbol) if symbol.name.as_ref() == "list") => {
            args
        }
        _ => {
            return Err(Error::TypeMismatch {
                expected: "step list",
                found: "non-list",
            });
        }
    };
    items
        .into_iter()
        .map(|item| connection_step_from_expr(cx, item))
        .collect()
}

fn connection_step_from_expr(cx: &mut Cx, expr: Expr) -> Result<Connection> {
    let value = cx.eval_expr(expr)?;
    if let Some(connection) = connection_from_value(&value) {
        return Ok(connection.clone());
    }
    connect_target_value(
        cx,
        value,
        None,
        &[],
        None,
        false,
        IsolationPolicy::default(),
    )
}

fn server_address_expr(address: &ServerAddress) -> Expr {
    match address {
        ServerAddress::Local => Expr::Symbol(Symbol::new("local")),
        ServerAddress::InProcess { thread } => Expr::List(vec![
            Expr::Symbol(Symbol::new("in-process")),
            Expr::Symbol(Symbol::new(":thread")),
            Expr::String(thread.to_string()),
        ]),
        ServerAddress::Coroutine { id } => Expr::List(vec![
            Expr::Symbol(Symbol::new("coroutine")),
            Expr::Symbol(Symbol::new(":id")),
            Expr::String(id.to_string()),
        ]),
        ServerAddress::Tcp { host, port } => Expr::List(vec![
            Expr::Symbol(Symbol::new("tcp")),
            Expr::Symbol(Symbol::new(":host")),
            Expr::String(host.clone()),
            Expr::Symbol(Symbol::new(":port")),
            Expr::String(port.to_string()),
        ]),
        ServerAddress::Unix { path } => Expr::List(vec![
            Expr::Symbol(Symbol::new("unix")),
            Expr::Symbol(Symbol::new(":path")),
            Expr::String(path.display().to_string()),
        ]),
        ServerAddress::Wasm { region } => Expr::List(vec![
            Expr::Symbol(Symbol::new("wasm")),
            Expr::Symbol(Symbol::new(":region")),
            Expr::String(region.clone()),
        ]),
        ServerAddress::Http { url } => Expr::List(vec![
            Expr::Symbol(Symbol::new("http")),
            Expr::Symbol(Symbol::new(":url")),
            Expr::String(url.clone()),
        ]),
        ServerAddress::Ws { url } => Expr::List(vec![
            Expr::Symbol(Symbol::new("ws")),
            Expr::Symbol(Symbol::new(":url")),
            Expr::String(url.clone()),
        ]),
        ServerAddress::Sse { url } => Expr::List(vec![
            Expr::Symbol(Symbol::new("sse")),
            Expr::Symbol(Symbol::new(":url")),
            Expr::String(url.clone()),
        ]),
        ServerAddress::Smtp { address } => Expr::List(vec![
            Expr::Symbol(Symbol::new("smtp")),
            Expr::Symbol(Symbol::new(":address")),
            Expr::String(address.clone()),
        ]),
        ServerAddress::Imap { address, mailbox } => Expr::List(vec![
            Expr::Symbol(Symbol::new("imap")),
            Expr::Symbol(Symbol::new(":address")),
            Expr::String(address.clone()),
            Expr::Symbol(Symbol::new(":mailbox")),
            Expr::String(mailbox.clone()),
        ]),
        ServerAddress::Telegram { chat_id, bot } => Expr::List(vec![
            Expr::Symbol(Symbol::new("telegram")),
            Expr::Symbol(Symbol::new(":chat-id")),
            Expr::String(chat_id.clone()),
            Expr::Symbol(Symbol::new(":bot")),
            Expr::String(bot.clone()),
        ]),
        ServerAddress::Matrix { room_id } => Expr::List(vec![
            Expr::Symbol(Symbol::new("matrix")),
            Expr::Symbol(Symbol::new(":room-id")),
            Expr::String(room_id.clone()),
        ]),
        ServerAddress::Stdin => Expr::Symbol(Symbol::new("stdin")),
        ServerAddress::FileTail { path } => Expr::List(vec![
            Expr::Symbol(Symbol::new("file-tail")),
            Expr::Symbol(Symbol::new(":path")),
            Expr::String(path.display().to_string()),
        ]),
        ServerAddress::Cron { spec } => Expr::List(vec![
            Expr::Symbol(Symbol::new("cron")),
            Expr::Symbol(Symbol::new(":spec")),
            Expr::String(spec.clone()),
        ]),
        ServerAddress::Webhook { route } => Expr::List(vec![
            Expr::Symbol(Symbol::new("webhook")),
            Expr::Symbol(Symbol::new(":route")),
            Expr::String(route.clone()),
        ]),
        ServerAddress::Agent { agent } => Expr::List(vec![
            Expr::Symbol(Symbol::new("agent")),
            Expr::String(agent.clone()),
        ]),
        ServerAddress::Pipeline { steps } => Expr::List(
            std::iter::once(Expr::Symbol(Symbol::new("pipeline")))
                .chain(steps.iter().map(server_address_expr))
                .collect(),
        ),
        ServerAddress::Any => Expr::Symbol(Symbol::new("any")),
    }
}

fn connection_from_value(value: &Value) -> Option<&Connection> {
    value.object().downcast_ref::<Connection>()
}

#[derive(Clone)]
struct ValueEvalFabric {
    value: Value,
}

impl EvalFabric for ValueEvalFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        self.value
            .object()
            .as_eval_fabric()
            .ok_or(Error::TypeMismatch {
                expected: "eval fabric",
                found: "non-fabric",
            })?
            .realize(cx, request)
    }
}

pub(crate) fn server_from_value(value: &Value) -> Option<&Server> {
    value.object().downcast_ref::<Server>()
}

fn coroutine_from_value(value: &Value) -> Option<&Coroutine> {
    value.object().downcast_ref::<Coroutine>()
}

pub(crate) fn coroutine_target_from_value(value: &Value) -> Result<Arc<Coroutine>> {
    if let Some(coroutine) = coroutine_from_value(value) {
        return Ok(Arc::new(coroutine.clone()));
    }
    if let Some(server) = server_from_value(value) {
        if let Some(site) = server.site().as_any().downcast_ref::<CoroutineEvalSite>() {
            return Ok(site.coroutine().clone());
        }
        if let Some(site) = server.site().as_any().downcast_ref::<IsolatedEvalSite>()
            && let Some(inner) = site.inner().as_any().downcast_ref::<CoroutineEvalSite>()
        {
            return Ok(inner.coroutine().clone());
        }
    }
    Err(Error::TypeMismatch {
        expected: "coroutine or coroutine-backed server",
        found: "non-coroutine",
    })
}

pub(crate) fn evaluated_connection(cx: &mut Cx, expr: &Expr) -> Result<Connection> {
    let value = cx.eval_expr(expr.clone())?;
    value
        .object()
        .downcast_ref::<Connection>()
        .cloned()
        .ok_or(Error::TypeMismatch {
            expected: "connection",
            found: "non-connection",
        })
}
