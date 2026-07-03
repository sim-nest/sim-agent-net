use std::sync::{Arc, Mutex, MutexGuard};

use sim_kernel::{
    AbiVersion, Args, Callable, CapabilityName, Cx, Error, Export, Expr, Lib, LibManifest,
    LibTarget, Linker, LoadCx, Object, ObjectCompat, Result, Symbol, Value, Version,
};
use sim_lib_agent_runner_core::{ModelCard, ModelRequest, ModelResponse, ModelRunner};
use sim_wasm_abi::model::{
    WasmModelAbiLimits, WasmModelInstance, decode_model_expr_frame, encode_model_expr_frame,
};

use crate::LOCAL_WASM_MODEL_SITE_KEY;

/// Capability required to instantiate a local model runner.
pub fn ai_runner_local_capability() -> CapabilityName {
    CapabilityName::new("ai-runner-local")
}

/// Capability required to instantiate a wasm model runner.
pub fn ai_runner_wasm_capability() -> CapabilityName {
    CapabilityName::new("ai-runner-wasm")
}

/// Runtime limits applied to a local wasm model guest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WasmModelLimits {
    /// Fuel granted to each model-card or infer call.
    pub fuel_per_infer: u64,
    /// Maximum guest memory size in wasm pages.
    pub max_memory_pages: u32,
}

impl Default for WasmModelLimits {
    fn default() -> Self {
        Self {
            fuel_per_infer: 5_000_000_000,
            max_memory_pages: 4096,
        }
    }
}

impl From<WasmModelLimits> for WasmModelAbiLimits {
    fn from(value: WasmModelLimits) -> Self {
        Self {
            fuel_per_infer: value.fuel_per_infer,
            max_memory_pages: value.max_memory_pages,
            ..WasmModelAbiLimits::default()
        }
    }
}

/// Instantiates a local wasm model after checking the required capabilities.
pub fn load_wasm_model(
    cx: &Cx,
    wasm_bytes: &[u8],
    limits: WasmModelLimits,
) -> Result<WasmModelBackend> {
    cx.require(&ai_runner_local_capability())?;
    cx.require(&ai_runner_wasm_capability())?;
    WasmModelBackend::from_bytes_with_limits(wasm_bytes, limits)
}

/// Local wasm model backend exposed as a provider-neutral [`ModelRunner`].
pub struct WasmModelBackend {
    placement_key: String,
    card: ModelCard,
    instance: Mutex<WasmModelInstance>,
}

impl WasmModelBackend {
    /// Instantiates a backend from wasm bytes under the supplied limits.
    pub fn from_bytes_with_limits(wasm_bytes: &[u8], limits: WasmModelLimits) -> Result<Self> {
        let mut instance = WasmModelInstance::from_bytes_with_limits(wasm_bytes, limits.into())
            .map_err(|err| wasm_model_error("load", err))?;
        let card_frame = instance
            .model_card_frame()
            .map_err(|err| wasm_model_error("load card", err))?;
        let card_expr = decode_model_expr_frame(&card_frame)
            .map_err(|err| wasm_model_error("load card", err))?;
        let card =
            ModelCard::try_from(card_expr).map_err(|err| wasm_model_error("load card", err))?;
        Ok(Self {
            placement_key: LOCAL_WASM_MODEL_SITE_KEY.to_owned(),
            card,
            instance: Mutex::new(instance),
        })
    }

    /// Returns the placement key this backend registers.
    pub fn placement_key(&self) -> &str {
        &self.placement_key
    }

    fn lock_instance(&self) -> Result<MutexGuard<'_, WasmModelInstance>> {
        self.instance
            .lock()
            .map_err(|_| Error::Eval("wasm model instance lock poisoned".to_owned()))
    }
}

impl ModelRunner for WasmModelBackend {
    fn card(&self) -> ModelCard {
        let mut card = self.card.clone();
        if !has_extra_key(&card, "placement-key") {
            card.extra.push(key_expr(
                "placement-key",
                Expr::String(self.placement_key.clone()),
            ));
        }
        card
    }

    fn infer(&self, _cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        let request_frame = encode_model_expr_frame(&request.into())
            .map_err(|err| wasm_model_error("encode request", err))?;
        let response_frame = self
            .lock_instance()?
            .infer_frame(request_frame)
            .map_err(|err| wasm_model_error("inference", err))?;
        let response_expr = decode_model_expr_frame(&response_frame)
            .map_err(|err| wasm_model_error("decode response", err))?;
        ModelResponse::try_from(response_expr)
            .map_err(|err| wasm_model_error("decode response", err))
    }
}

/// Loadable host library that registers a local wasm model placement site.
pub struct WasmModelLib {
    backend: Arc<WasmModelBackend>,
}

impl WasmModelLib {
    /// Builds a loadable site library from an instantiated wasm backend.
    pub fn new(backend: WasmModelBackend) -> Self {
        Self {
            backend: Arc::new(backend),
        }
    }
}

impl Lib for WasmModelLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("model", "local-wasm"),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: vec![ai_runner_local_capability(), ai_runner_wasm_capability()],
            exports: vec![Export::Site {
                symbol: local_wasm_model_site_symbol(),
                runtime_id: None,
            }],
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        let site = cx
            .factory()
            .opaque(Arc::new(WasmModelSite::new(Arc::clone(&self.backend))))?;
        linker.site_value(local_wasm_model_site_symbol(), site)?;
        Ok(())
    }
}

/// Returns the placement symbol used by the local wasm model site export.
pub fn local_wasm_model_site_symbol() -> Symbol {
    Symbol::new(LOCAL_WASM_MODEL_SITE_KEY)
}

#[derive(Clone)]
struct WasmModelSite {
    runner: Arc<WasmModelBackend>,
}

impl WasmModelSite {
    fn new(runner: Arc<WasmModelBackend>) -> Self {
        Self { runner }
    }
}

impl Object for WasmModelSite {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<model-site {}>", self.runner.placement_key()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for WasmModelSite {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }

    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(Expr::Symbol(local_wasm_model_site_symbol()))
    }

    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        let card: Expr = self.runner.card().into();
        cx.factory().table(vec![
            (
                Symbol::new("symbol"),
                cx.factory().symbol(local_wasm_model_site_symbol())?,
            ),
            (Symbol::new("card"), cx.factory().expr(card)?),
        ])
    }
}

impl Callable for WasmModelSite {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let request = request_from_values(cx, args)?;
        let response = self.runner.infer(cx, request)?;
        cx.factory().expr(response.into())
    }
}

fn request_from_values(cx: &mut Cx, args: Args) -> Result<ModelRequest> {
    let [request] = args.values() else {
        return Err(Error::Eval(
            "local wasm model site expects one EvalRequest argument".to_owned(),
        ));
    };
    let request = request.object().as_expr(cx)?;
    request_from_eval_request_expr(&request)
}

fn request_from_eval_request_expr(request: &Expr) -> Result<ModelRequest> {
    let expr = match request {
        Expr::Map(entries) => entries.iter().find_map(|(key, value)| match key {
            Expr::Symbol(symbol) if symbol.name.as_ref() == "expr" => Some(value),
            _ => None,
        }),
        _ => None,
    }
    .ok_or_else(|| Error::Eval("local wasm model site request missing expr".to_owned()))?;
    ModelRequest::try_from(expr.clone())
}

fn key_expr(key: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(key)), value)
}

fn has_extra_key(card: &ModelCard, key: &str) -> bool {
    card.extra.iter().any(|(entry_key, _)| match entry_key {
        Expr::Symbol(symbol) => symbol.name.as_ref() == key,
        _ => false,
    })
}

fn wasm_model_error(action: &str, err: Error) -> Error {
    match err {
        Error::CapabilityDenied { .. } | Error::TrustDenied { .. } => err,
        other => Error::Eval(format!("wasm model {action} failed: {other}")),
    }
}
