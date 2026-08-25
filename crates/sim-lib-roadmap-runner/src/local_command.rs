//! Public, local-only command surface for the roadmap runner.
// conformance: loadable local-only roadmap command boundary.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use sim_kernel::{
    AbiVersion, Args, Callable, ClassRef, Cx, Error, Export, Expr, Lib, LibManifest, LibTarget,
    Linker, LoadCx, MatchScore, Object, ObjectCompat, Result, Shape, ShapeDoc, ShapeMatch,
    ShapeRef, Symbol, Value, Version,
};
use sim_shape::shape_value;

/// The complete public execution surface. Delivery is intentionally absent.
pub const LOCAL_ROADMAP_VERBS: [&str; 5] = ["run", "resume", "status", "explain", "replay"];
const FORBIDDEN: [&str; 11] = [
    "push",
    "pr",
    "publish",
    "tag",
    "ci",
    "branch-protection",
    "release",
    "roadmap-status",
    "closeout",
    "deliver",
    "merge",
];

/// Content identities pinned before the first effect and repeated in every receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalExecutionIdentity {
    pub execution: String,
    pub conduct: String,
    pub model_pick: String,
    pub proof_catalog: String,
    pub runner_generation: String,
}

/// Typed request passed from the command parser to the execution adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalRoadmapRequest {
    pub verb: String,
    pub observe: bool,
    pub disposable_checkout: Option<String>,
    pub local_authority_token: Option<String>,
    pub identity: LocalExecutionIdentity,
}

/// Bounded result record; identities are mandatory even for refusals.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalRoadmapReceipt {
    pub outcome: String,
    pub identity: LocalExecutionIdentity,
    pub journal_acknowledged: bool,
    pub detail: String,
}

/// A delivered generation lease. Dropping the final lease releases the generation.
#[derive(Clone, Debug)]
pub struct GenerationHandle(Arc<GenerationLease>);
#[derive(Debug)]
struct GenerationLease {
    identity: String,
}
impl GenerationHandle {
    #[must_use]
    pub fn acquire(identity: impl Into<String>) -> Self {
        Self(Arc::new(GenerationLease {
            identity: identity.into(),
        }))
    }
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.0.identity
    }
    #[must_use]
    pub fn retained(&self) -> usize {
        Arc::strong_count(&self.0)
    }
}

/// Host adapter. It receives already parsed, local-only requests and a retained generation.
pub trait LocalRoadmapPort: Send + Sync {
    fn invoke(
        &self,
        request: &LocalRoadmapRequest,
        generation: GenerationHandle,
    ) -> std::result::Result<LocalRoadmapReceipt, String>;
}

/// Deterministic public adapter used by a clean installed bootloader and no-model recipes.
#[derive(Default)]
pub struct PublicLocalRoadmapPort {
    executions: Mutex<BTreeMap<String, LocalRoadmapReceipt>>,
}
impl LocalRoadmapPort for PublicLocalRoadmapPort {
    fn invoke(
        &self,
        request: &LocalRoadmapRequest,
        generation: GenerationHandle,
    ) -> std::result::Result<LocalRoadmapReceipt, String> {
        if generation.identity() != request.identity.runner_generation {
            return Err("generation identity drift".into());
        }
        let mut executions = self
            .executions
            .lock()
            .map_err(|_| "execution store poisoned")?;
        let prior = executions.get(&request.identity.execution).cloned();
        let detail = match request.verb.as_str() {
            "run"
                if !request.observe
                    && (request.disposable_checkout.is_none()
                        || request.local_authority_token.is_none()) =>
            {
                return Err(
                    "mutation requires --disposable-checkout and --local-authority-token".into(),
                );
            }
            "run" => {
                if request.observe {
                    "observed without mutation"
                } else {
                    "mutation admitted in disposable checkout"
                }
            }
            "resume" => {
                if prior.is_some() {
                    "resumed retained execution"
                } else {
                    return Err("unknown execution identity".into());
                }
            }
            "status" => {
                if prior.is_some() {
                    "execution found"
                } else {
                    "execution not started"
                }
            }
            "explain" => "content identities and bounded policy explained",
            "replay" => {
                if prior.is_some() {
                    "journal replayed without effects"
                } else {
                    "empty no-model replay"
                }
            }
            _ => return Err("unsupported local roadmap verb".into()),
        };
        let receipt = LocalRoadmapReceipt {
            outcome: "ok".into(),
            identity: request.identity.clone(),
            journal_acknowledged: true,
            detail: detail.into(),
        };
        if matches!(request.verb.as_str(), "run" | "resume") {
            executions.insert(request.identity.execution.clone(), receipt.clone());
        }
        Ok(receipt)
    }
}

pub struct LocalRoadmapRunnerLib {
    port: Arc<dyn LocalRoadmapPort>,
}
impl Default for LocalRoadmapRunnerLib {
    fn default() -> Self {
        Self::new()
    }
}
impl LocalRoadmapRunnerLib {
    #[must_use]
    pub fn new() -> Self {
        Self::with_port(Arc::new(PublicLocalRoadmapPort::default()))
    }
    #[must_use]
    pub fn with_port(port: Arc<dyn LocalRoadmapPort>) -> Self {
        Self { port }
    }
}
impl Lib for LocalRoadmapRunnerLib {
    fn manifest(&self) -> LibManifest {
        let mut exports = LOCAL_ROADMAP_VERBS
            .iter()
            .flat_map(|verb| {
                [
                    Export::Function {
                        symbol: op_symbol(verb),
                        function_id: None,
                    },
                    Export::Shape {
                        symbol: shape_symbol(verb, "Args"),
                        shape_id: None,
                    },
                    Export::Shape {
                        symbol: shape_symbol(verb, "Result"),
                        shape_id: None,
                    },
                ]
            })
            .collect::<Vec<_>>();
        exports.push(Export::Function {
            symbol: Symbol::qualified("cli/main", "roadmap"),
            function_id: None,
        });
        Self::manifest_value(exports)
    }
    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        for verb in LOCAL_ROADMAP_VERBS {
            let args = contract_shape(verb, true);
            let result = contract_shape(verb, false);
            linker.shape_value(shape_symbol(verb, "Args"), args.clone())?;
            linker.shape_value(shape_symbol(verb, "Result"), result.clone())?;
            linker.function_value(
                op_symbol(verb),
                cx.factory().opaque(Arc::new(LocalFunction {
                    verb,
                    port: self.port.clone(),
                    args,
                    result,
                }))?,
            )?;
        }
        linker.function_value(
            Symbol::qualified("cli/main", "roadmap"),
            cx.factory().opaque(Arc::new(LocalCommand {
                port: self.port.clone(),
            }))?,
        )?;
        Ok(())
    }
}
impl LocalRoadmapRunnerLib {
    fn manifest_value(exports: Vec<Export>) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("lib", "roadmap-runner"),
            version: Version(env!("CARGO_PKG_VERSION").into()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: vec![],
            capabilities: vec![],
            exports,
        }
    }
}

struct LocalFunction {
    verb: &'static str,
    port: Arc<dyn LocalRoadmapPort>,
    args: Value,
    result: Value,
}
impl Object for LocalFunction {
    fn display(&self, _: &mut Cx) -> Result<String> {
        Ok(op_symbol(self.verb).to_string())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl ObjectCompat for LocalFunction {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.resolve_class(&Symbol::qualified("core", "Function"))
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}
impl Callable for LocalFunction {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let request = request_from_values(self.verb, args.values())?;
        invoke(cx, &*self.port, request)
    }
    fn browse_args_shape(&self, _: &mut Cx) -> Result<Option<ShapeRef>> {
        Ok(Some(self.args.clone()))
    }
    fn browse_result_shape(&self, _: &mut Cx) -> Result<Option<ShapeRef>> {
        Ok(Some(self.result.clone()))
    }
}
struct LocalCommand {
    port: Arc<dyn LocalRoadmapPort>,
}
impl Object for LocalCommand {
    fn display(&self, _: &mut Cx) -> Result<String> {
        Ok("cli/main/roadmap".into())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl ObjectCompat for LocalCommand {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}
impl Callable for LocalCommand {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let envelope = args
            .values()
            .first()
            .ok_or_else(|| Error::Eval("missing roadmap envelope".into()))?;
        let table = envelope
            .object()
            .as_table_impl()
            .ok_or_else(|| Error::Eval("roadmap envelope is not a table".into()))?;
        let Expr::List(argv) = table.get(cx, Symbol::new("args"))?.object().as_expr(cx)? else {
            return Err(Error::Eval("roadmap args are not a list".into()));
        };
        let text = argv
            .into_iter()
            .filter_map(|v| {
                if let Expr::String(s) = v {
                    Some(s)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let verb = text.get(1).map(String::as_str).unwrap_or("status");
        let request = parse_cli(verb, &text[2..])?;
        invoke(cx, &*self.port, request)
    }
}

fn parse_cli(verb: &str, argv: &[String]) -> Result<LocalRoadmapRequest> {
    if FORBIDDEN.contains(&verb) {
        return Err(Error::Eval(format!(
            "delivery verb {verb} is not part of the local roadmap service"
        )));
    }
    if !LOCAL_ROADMAP_VERBS.contains(&verb) {
        return Err(Error::Eval(format!("unknown roadmap runner verb {verb}")));
    }
    let value = |flag: &str| argv.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone());
    let execution = value("--execution").unwrap_or_else(|| "local/no-model".into());
    let required = |flag: &str| {
        value(flag).ok_or_else(|| Error::Eval(format!("missing content identity {flag}")))
    };
    Ok(LocalRoadmapRequest {
        verb: verb.into(),
        observe: argv.iter().any(|a| a == "--observe"),
        disposable_checkout: value("--disposable-checkout"),
        local_authority_token: value("--local-authority-token"),
        identity: LocalExecutionIdentity {
            execution,
            conduct: required("--conduct")?,
            model_pick: required("--model-pick")?,
            proof_catalog: required("--proof-catalog")?,
            runner_generation: required("--runner-generation")?,
        },
    })
}
fn request_from_values(verb: &str, values: &[Value]) -> Result<LocalRoadmapRequest> {
    let args = values
        .iter()
        .map(|v| v.object().display(&mut dummy_cx()))
        .collect::<Result<Vec<_>>>()?;
    parse_cli(verb, &args)
}
fn dummy_cx() -> Cx {
    Cx::new(
        Arc::new(sim_kernel::NoopEvalPolicy),
        Arc::new(sim_kernel::DefaultFactory),
        sim_kernel::HandleSeed::new(7),
    )
}
fn invoke(cx: &mut Cx, port: &dyn LocalRoadmapPort, request: LocalRoadmapRequest) -> Result<Value> {
    let generation = GenerationHandle::acquire(request.identity.runner_generation.clone());
    let receipt = port.invoke(&request, generation).map_err(Error::Eval)?;
    cx.factory().table(vec![
        (
            Symbol::new("outcome"),
            cx.factory().string(receipt.outcome)?,
        ),
        (
            Symbol::new("execution"),
            cx.factory().string(receipt.identity.execution)?,
        ),
        (
            Symbol::new("conduct"),
            cx.factory().string(receipt.identity.conduct)?,
        ),
        (
            Symbol::new("model-pick"),
            cx.factory().string(receipt.identity.model_pick)?,
        ),
        (
            Symbol::new("proof-catalog"),
            cx.factory().string(receipt.identity.proof_catalog)?,
        ),
        (
            Symbol::new("runner-generation"),
            cx.factory().string(receipt.identity.runner_generation)?,
        ),
        (
            Symbol::new("journal-acknowledged"),
            cx.factory().bool(receipt.journal_acknowledged)?,
        ),
        (Symbol::new("detail"), cx.factory().string(receipt.detail)?),
    ])
}

struct ContractShape {
    symbol: Symbol,
    args: bool,
}
impl Shape for ContractShape {
    fn symbol(&self) -> Option<Symbol> {
        Some(self.symbol.clone())
    }
    fn check_value(&self, cx: &mut Cx, value: Value) -> Result<ShapeMatch> {
        let expr = value.object().as_expr(cx)?;
        self.check_expr(cx, &expr)
    }
    fn check_expr(&self, _: &mut Cx, expr: &Expr) -> Result<ShapeMatch> {
        Ok(
            if matches!(
                (self.args, expr),
                (true, Expr::List(_)) | (false, Expr::Map(_))
            ) {
                ShapeMatch::accept(MatchScore::exact(100))
            } else {
                ShapeMatch::reject("roadmap runner contract mismatch")
            },
        )
    }
    fn describe(&self, _: &mut Cx) -> Result<ShapeDoc> {
        Ok(ShapeDoc::new(
            "local roadmap request/result with pinned content identities",
        ))
    }
}
fn contract_shape(verb: &str, args: bool) -> Value {
    let symbol = shape_symbol(verb, if args { "Args" } else { "Result" });
    shape_value(symbol.clone(), Arc::new(ContractShape { symbol, args }))
}
fn op_symbol(verb: &str) -> Symbol {
    Symbol::qualified("roadmap", verb)
}
fn shape_symbol(verb: &str, suffix: &str) -> Symbol {
    Symbol::qualified(format!("roadmap/{verb}"), suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_surface_has_only_local_verbs_and_explicit_shapes() {
        let m = LocalRoadmapRunnerLib::new().manifest();
        let text = format!("{:?}", m.exports);
        for verb in LOCAL_ROADMAP_VERBS {
            assert!(text.contains(verb));
            assert!(text.contains(&format!("{verb}/Args")));
            assert!(text.contains(&format!("{verb}/Result")));
        }
        for forbidden in FORBIDDEN {
            assert!(!text.contains(&format!("roadmap/{forbidden}")));
        }
    }
    #[test]
    fn mutation_needs_disposable_checkout_and_token() {
        let p = PublicLocalRoadmapPort::default();
        let r = parse_cli(
            "run",
            &[
                "--conduct",
                "sha:c",
                "--model-pick",
                "sha:m",
                "--proof-catalog",
                "sha:p",
                "--runner-generation",
                "sha:g",
            ]
            .map(str::to_owned),
        )
        .unwrap();
        assert!(
            p.invoke(&r, GenerationHandle::acquire("sha:g"))
                .unwrap_err()
                .contains("disposable")
        );
    }
    #[test]
    fn observe_and_replay_are_effect_bounded_and_identity_complete() {
        let p = PublicLocalRoadmapPort::default();
        let r = parse_cli(
            "run",
            &[
                "--observe",
                "--execution",
                "e",
                "--conduct",
                "sha:c",
                "--model-pick",
                "sha:m",
                "--proof-catalog",
                "sha:p",
                "--runner-generation",
                "sha:g",
            ]
            .map(str::to_owned),
        )
        .unwrap();
        let h = GenerationHandle::acquire("sha:g");
        let retained = h.clone();
        let receipt = p.invoke(&r, h).unwrap();
        assert_eq!(retained.retained(), 1);
        assert!(receipt.journal_acknowledged);
        assert_eq!(receipt.identity, r.identity);
        let mut replay = r;
        replay.verb = "replay".into();
        assert!(
            p.invoke(&replay, retained)
                .unwrap()
                .detail
                .contains("replayed")
        );
    }
}
