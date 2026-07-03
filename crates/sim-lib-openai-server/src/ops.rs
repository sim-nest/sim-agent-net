use std::sync::Arc;

use sim_kernel::{
    Args, Callable, CapabilityName, CapabilitySet, ClassRef, Cx, Error, Expr, Object, ObjectCompat,
    RawArgs, Result, Symbol, Value, eval_fabric_capability,
};
use sim_value::kind::expr_kind;

use crate::{
    capabilities::{openai_gateway_admin_capability, openai_gateway_serve_capabilities},
    ops_admin::{call_admin_state_expr, call_run_get},
    plan::{check_plan, eval_plan, explain_plan, parse_plan, plan_combinators_expr},
    routes::{
        admin::{
            cache_stats_expr, capability_report_expr, events_expr, model_health_expr, runs_expr,
            storage_stats_expr,
        },
        health::health_response,
        models::models_response_for_runner_args,
    },
    runtime::{OpenAiGatewayFabric, global_openai_key_table},
    server::{GatewayResponseValue, GatewayRoutesValue, configure_routes},
};

/// Callable runtime function exposing one gateway operation to the runtime.
///
/// Each instance wraps a single operation kind (serve, health, plan handling,
/// key management, admin reporting, ...). It is registered under its
/// [`OpenAiGatewayFunction::symbol`] and dispatched through its [`Callable`]
/// implementation.
#[derive(Clone)]
pub struct OpenAiGatewayFunction {
    kind: OpenAiGatewayFunctionKind,
}

#[derive(Clone, Copy)]
enum OpenAiGatewayFunctionKind {
    Serve,
    Health,
    Models,
    PlanParse,
    PlanCheck,
    PlanRun,
    PlanExplain,
    PlanCombinators,
    Fabric,
    KeyAdd,
    KeyList,
    Runs,
    RunGet,
    Events,
    StorageStats,
    ModelHealth,
    CacheStats,
    CapabilityReport,
}

impl OpenAiGatewayFunction {
    fn new(kind: OpenAiGatewayFunctionKind) -> Self {
        Self { kind }
    }

    /// Returns the `openai-gateway/serve` function, which builds the configured
    /// gateway routes.
    pub fn serve() -> Self {
        Self::new(OpenAiGatewayFunctionKind::Serve)
    }

    /// Returns the `openai-gateway/health` function, which produces a health
    /// response.
    pub fn health() -> Self {
        Self::new(OpenAiGatewayFunctionKind::Health)
    }

    /// Returns the `openai-gateway/models` function, which lists available
    /// models.
    pub fn models() -> Self {
        Self::new(OpenAiGatewayFunctionKind::Models)
    }

    /// Returns the `openai-gateway/plan-parse` function, which parses plan
    /// source into an expression.
    pub fn plan_parse() -> Self {
        Self::new(OpenAiGatewayFunctionKind::PlanParse)
    }

    /// Returns the `openai-gateway/plan-check` function, which validates a plan
    /// expression.
    pub fn plan_check() -> Self {
        Self::new(OpenAiGatewayFunctionKind::PlanCheck)
    }

    /// Returns the `openai-gateway/plan-run` function, which evaluates a plan
    /// against a request.
    pub fn plan_run() -> Self {
        Self::new(OpenAiGatewayFunctionKind::PlanRun)
    }

    /// Returns the `openai-gateway/plan-explain` function, which renders a
    /// human-readable explanation of a plan.
    pub fn plan_explain() -> Self {
        Self::new(OpenAiGatewayFunctionKind::PlanExplain)
    }

    /// Returns the `openai-gateway/plan-combinators` function, which lists the
    /// available plan combinators.
    pub fn plan_combinators() -> Self {
        Self::new(OpenAiGatewayFunctionKind::PlanCombinators)
    }

    /// Returns the `openai-gateway/fabric` function, which constructs an
    /// in-memory eval fabric.
    pub fn fabric() -> Self {
        Self::new(OpenAiGatewayFunctionKind::Fabric)
    }

    /// Returns the `openai-gateway/key-add` function, which registers an API
    /// key with a capability set.
    pub fn key_add() -> Self {
        Self::new(OpenAiGatewayFunctionKind::KeyAdd)
    }

    /// Returns the `openai-gateway/key-list` function, which lists registered
    /// keys.
    pub fn key_list() -> Self {
        Self::new(OpenAiGatewayFunctionKind::KeyList)
    }

    /// Returns the `openai-gateway/runs` function, which reports stored runs.
    pub fn runs() -> Self {
        Self::new(OpenAiGatewayFunctionKind::Runs)
    }

    /// Returns the `openai-gateway/run-get` function, which fetches a single
    /// run by id.
    pub fn run_get() -> Self {
        Self::new(OpenAiGatewayFunctionKind::RunGet)
    }

    /// Returns the `openai-gateway/events` function, which reports stored
    /// events.
    pub fn events() -> Self {
        Self::new(OpenAiGatewayFunctionKind::Events)
    }

    /// Returns the `openai-gateway/storage-stats` function, which reports
    /// storage statistics.
    pub fn storage_stats() -> Self {
        Self::new(OpenAiGatewayFunctionKind::StorageStats)
    }

    /// Returns the `openai-gateway/model-health` function, which reports model
    /// health.
    pub fn model_health() -> Self {
        Self::new(OpenAiGatewayFunctionKind::ModelHealth)
    }

    /// Returns the `openai-gateway/cache-stats` function, which reports plan
    /// cache statistics.
    pub fn cache_stats() -> Self {
        Self::new(OpenAiGatewayFunctionKind::CacheStats)
    }

    /// Returns the `openai-gateway/capability-report` function, which reports
    /// the gateway capability set.
    pub fn capability_report() -> Self {
        Self::new(OpenAiGatewayFunctionKind::CapabilityReport)
    }

    /// Returns the qualified symbol this function is registered under.
    pub fn symbol(&self) -> Symbol {
        self.kind.symbol()
    }
}

impl Object for OpenAiGatewayFunction {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<function {}>", self.kind.symbol()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for OpenAiGatewayFunction {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.resolve_class(&Symbol::qualified("core", "Function"))
    }

    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for OpenAiGatewayFunction {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        self.kind.call(cx, args.into_vec())
    }

    fn call_exprs(&self, cx: &mut Cx, args: RawArgs) -> Result<Value> {
        let values = args
            .into_exprs()
            .into_iter()
            .map(|expr| cx.eval_expr(expr))
            .collect::<Result<Vec<_>>>()?;
        self.kind.call(cx, values)
    }
}

impl OpenAiGatewayFunctionKind {
    fn symbol(self) -> Symbol {
        match self {
            Self::Serve => serve_symbol(),
            Self::Health => health_symbol(),
            Self::Models => models_symbol(),
            Self::PlanParse => plan_parse_symbol(),
            Self::PlanCheck => plan_check_symbol(),
            Self::PlanRun => plan_run_symbol(),
            Self::PlanExplain => plan_explain_symbol(),
            Self::PlanCombinators => plan_combinators_symbol(),
            Self::Fabric => fabric_symbol(),
            Self::KeyAdd => key_add_symbol(),
            Self::KeyList => key_list_symbol(),
            Self::Runs => runs_symbol(),
            Self::RunGet => run_get_symbol(),
            Self::Events => events_symbol(),
            Self::StorageStats => storage_stats_symbol(),
            Self::ModelHealth => model_health_symbol(),
            Self::CacheStats => cache_stats_symbol(),
            Self::CapabilityReport => capability_report_symbol(),
        }
    }

    fn call(self, cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
        match self {
            Self::Serve | Self::Health | Self::PlanCombinators | Self::Fabric | Self::KeyList
                if !args.is_empty() =>
            {
                Err(Error::Eval(format!(
                    "{} expects no arguments",
                    self.symbol()
                )))
            }
            Self::Serve => call_serve(cx),
            Self::Health => call_health(cx),
            Self::Models => call_models(cx, args),
            Self::PlanParse => call_plan_parse(cx, args),
            Self::PlanCheck => call_plan_check(cx, args),
            Self::PlanRun => call_plan_run(cx, args),
            Self::PlanExplain => call_plan_explain(cx, args),
            Self::PlanCombinators => call_plan_combinators(cx),
            Self::Fabric => call_fabric(cx),
            Self::KeyAdd => call_key_add(cx, args),
            Self::KeyList => call_key_list(cx),
            Self::Runs => call_admin_state_expr(cx, self.symbol(), args, runs_expr),
            Self::RunGet => call_run_get(cx, self.symbol(), args),
            Self::Events => call_admin_state_expr(cx, self.symbol(), args, events_expr),
            Self::StorageStats => {
                call_admin_state_expr(cx, self.symbol(), args, storage_stats_expr)
            }
            Self::ModelHealth => call_admin_state_expr(cx, self.symbol(), args, model_health_expr),
            Self::CacheStats => call_admin_state_expr(cx, self.symbol(), args, cache_stats_expr),
            Self::CapabilityReport => {
                call_admin_state_expr(cx, self.symbol(), args, capability_report_expr)
            }
        }
    }
}

/// Returns the `openai-gateway/serve` operation symbol.
pub fn serve_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "serve")
}

/// Returns the `openai-gateway/health` operation symbol.
pub fn health_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "health")
}

/// Returns the `openai-gateway/models` operation symbol.
pub fn models_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "models")
}

/// Returns the `openai-gateway/plan-parse` operation symbol.
pub fn plan_parse_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "plan-parse")
}

/// Returns the `openai-gateway/plan-check` operation symbol.
pub fn plan_check_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "plan-check")
}

/// Returns the `openai-gateway/plan-run` operation symbol.
pub fn plan_run_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "plan-run")
}

/// Returns the `openai-gateway/plan-explain` operation symbol.
pub fn plan_explain_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "plan-explain")
}

/// Returns the `openai-gateway/plan-combinators` operation symbol.
pub fn plan_combinators_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "plan-combinators")
}

/// Returns the `openai-gateway/fabric` operation symbol.
pub fn fabric_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "fabric")
}

/// Returns the `openai-gateway/key-add` operation symbol.
pub fn key_add_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "key-add")
}

/// Returns the `openai-gateway/key-list` operation symbol.
pub fn key_list_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "key-list")
}

/// Returns the `openai-gateway/runs` operation symbol.
pub fn runs_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "runs")
}

/// Returns the `openai-gateway/run-get` operation symbol.
pub fn run_get_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "run-get")
}

/// Returns the `openai-gateway/events` operation symbol.
pub fn events_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "events")
}

/// Returns the `openai-gateway/storage-stats` operation symbol.
pub fn storage_stats_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "storage-stats")
}

/// Returns the `openai-gateway/model-health` operation symbol.
pub fn model_health_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "model-health")
}

/// Returns the `openai-gateway/cache-stats` operation symbol.
pub fn cache_stats_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "cache-stats")
}

/// Returns the `openai-gateway/capability-report` operation symbol.
pub fn capability_report_symbol() -> Symbol {
    Symbol::qualified("openai-gateway", "capability-report")
}

fn call_serve(cx: &mut Cx) -> Result<Value> {
    cx.require_all(&openai_gateway_serve_capabilities())?;
    cx.factory()
        .opaque(Arc::new(GatewayRoutesValue::new(configure_routes())))
}

fn call_health(cx: &mut Cx) -> Result<Value> {
    cx.factory()
        .opaque(Arc::new(GatewayResponseValue::new(health_response())))
}

fn call_models(cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
    let response = models_response_for_runner_args(cx, args)?;
    cx.factory()
        .opaque(Arc::new(GatewayResponseValue::new(response)))
}

fn call_plan_parse(cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
    let input = string_arg(cx, plan_parse_symbol(), args)?;
    cx.factory().expr(parse_plan(&input)?)
}

fn call_plan_check(cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
    let plan = expr_arg(cx, plan_check_symbol(), args)?;
    check_plan(&plan)?;
    cx.factory().bool(true)
}

fn call_plan_run(cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
    let mut args = expect_arg_count(plan_run_symbol(), args, 2)?.into_iter();
    let plan = value_expr(cx, args.next().expect("plan-run arg count checked"))?;
    let request = value_expr(cx, args.next().expect("plan-run arg count checked"))?;
    let response = eval_plan(cx, &plan, &request)?;
    cx.factory().expr(response)
}

fn call_plan_explain(cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
    let plan = expr_arg(cx, plan_explain_symbol(), args)?;
    cx.factory().string(explain_plan(&plan)?)
}

fn call_plan_combinators(cx: &mut Cx) -> Result<Value> {
    cx.factory().expr(plan_combinators_expr())
}

fn call_fabric(cx: &mut Cx) -> Result<Value> {
    cx.require(&eval_fabric_capability())?;
    cx.factory().opaque(Arc::new(OpenAiGatewayFabric::memory()))
}

fn call_key_add(cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
    cx.require(&openai_gateway_admin_capability())?;
    if args.is_empty() {
        return Err(Error::Eval(format!(
            "{} expects at least 1 argument",
            key_add_symbol()
        )));
    }
    let mut args = args.into_iter();
    let secret = value_string(
        cx,
        args.next().expect("key-add arg count checked as non-empty"),
    )?;
    let mut capabilities = CapabilitySet::new();
    for arg in args {
        capabilities.insert(value_capability(cx, arg)?);
    }
    let key = global_openai_key_table().add_secret(&secret, capabilities)?;
    cx.factory().opaque(Arc::new(key))
}

fn call_key_list(cx: &mut Cx) -> Result<Value> {
    cx.require(&openai_gateway_admin_capability())?;
    let keys = global_openai_key_table().list_keys()?;
    cx.factory().list(
        keys.into_iter()
            .map(|key| cx.factory().opaque(Arc::new(key)))
            .collect::<Result<Vec<_>>>()?,
    )
}

fn string_arg(cx: &mut Cx, symbol: Symbol, args: Vec<Value>) -> Result<String> {
    match expr_arg(cx, symbol.clone(), args)? {
        Expr::String(value) => Ok(value),
        other => Err(Error::TypeMismatch {
            expected: "string argument",
            found: expr_kind(&other),
        }),
    }
}

fn value_string(cx: &mut Cx, value: Value) -> Result<String> {
    match value_expr(cx, value)? {
        Expr::String(value) => Ok(value),
        other => Err(Error::TypeMismatch {
            expected: "string argument",
            found: expr_kind(&other),
        }),
    }
}

fn value_capability(cx: &mut Cx, value: Value) -> Result<CapabilityName> {
    match value_expr(cx, value)? {
        Expr::String(value) => Ok(CapabilityName::new(value)),
        Expr::Symbol(symbol) => Ok(CapabilityName::new(symbol.to_string())),
        other => Err(Error::TypeMismatch {
            expected: "capability string or symbol",
            found: expr_kind(&other),
        }),
    }
}

fn expr_arg(cx: &mut Cx, symbol: Symbol, args: Vec<Value>) -> Result<Expr> {
    let mut args = expect_arg_count(symbol, args, 1)?.into_iter();
    value_expr(cx, args.next().expect("arg count checked"))
}

fn expect_arg_count(symbol: Symbol, args: Vec<Value>, expected: usize) -> Result<Vec<Value>> {
    if args.len() == expected {
        Ok(args)
    } else {
        Err(Error::Eval(format!(
            "{symbol} expects {expected} argument(s), found {}",
            args.len()
        )))
    }
}

fn value_expr(cx: &mut Cx, value: Value) -> Result<Expr> {
    value.object().as_expr(cx)
}
