use std::sync::Arc;

use sim_kernel::{
    AbiVersion, Args, Callable, Cx, Error, Export, Expr, Lib, LibManifest, LibTarget, Linker,
    LoadCx, Object, ObjectCompat, Result, Symbol, Value, Version,
};
use sim_lib_agent_runner_core::{ModelRequest, ModelRunner};

use crate::{LOCAL_MODEL_SITE_KEY, LocalModelBackend};

/// Loadable host library that registers the local model placement site.
pub struct LocalModelLib;

impl Lib for LocalModelLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("model", "local"),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Site {
                symbol: local_model_site_symbol(),
                runtime_id: None,
            }],
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        let site = cx
            .factory()
            .opaque(Arc::new(LocalModelSite::new(LocalModelBackend::new())))?;
        linker.site_value(local_model_site_symbol(), site)?;
        Ok(())
    }
}

/// Returns the placement symbol used by the loadable site export.
pub fn local_model_site_symbol() -> Symbol {
    Symbol::new(LOCAL_MODEL_SITE_KEY)
}

/// Realizes a native-site request argument list into a model response expression.
pub fn realize_site_args(args: Vec<Expr>) -> Result<Expr> {
    let request = request_from_site_args(args)?;
    let mut cx = Cx::new(
        Arc::new(sim_kernel::NoopEvalPolicy),
        Arc::new(sim_kernel::DefaultFactory),
    );
    let response = LocalModelBackend::new().infer(&mut cx, request)?;
    Ok(response.into())
}

#[derive(Clone, Debug)]
struct LocalModelSite {
    runner: LocalModelBackend,
}

impl LocalModelSite {
    fn new(runner: LocalModelBackend) -> Self {
        Self { runner }
    }
}

impl Object for LocalModelSite {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<model-site {}>", self.runner.placement_key()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for LocalModelSite {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }

    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(Expr::Symbol(local_model_site_symbol()))
    }

    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        let card: Expr = self.runner.card().into();
        cx.factory().table(vec![
            (
                Symbol::new("symbol"),
                cx.factory().symbol(local_model_site_symbol())?,
            ),
            (Symbol::new("card"), cx.factory().expr(card)?),
        ])
    }
}

impl Callable for LocalModelSite {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let request = request_from_values(cx, args)?;
        let response = self.runner.infer(cx, request)?;
        cx.factory().expr(response.into())
    }
}

fn request_from_values(cx: &mut Cx, args: Args) -> Result<ModelRequest> {
    let [request] = args.values() else {
        return Err(Error::Eval(
            "local model site expects one EvalRequest argument".to_owned(),
        ));
    };
    let request = request.object().as_expr(cx)?;
    request_from_eval_request_expr(&request)
}

fn request_from_site_args(args: Vec<Expr>) -> Result<ModelRequest> {
    let [request] = args.as_slice() else {
        return Err(Error::Eval(
            "local model native site expects one EvalRequest argument".to_owned(),
        ));
    };
    request_from_eval_request_expr(request)
}

fn request_from_eval_request_expr(request: &Expr) -> Result<ModelRequest> {
    let expr = match request {
        Expr::Map(entries) => entries.iter().find_map(|(key, value)| match key {
            Expr::Symbol(symbol) if symbol.name.as_ref() == "expr" => Some(value),
            _ => None,
        }),
        _ => None,
    }
    .ok_or_else(|| Error::Eval("local model site request missing expr".to_owned()))?;
    ModelRequest::try_from(expr.clone())
}
