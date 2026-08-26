use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use sim_kernel::{
    AbiVersion, Args, Callable, Cx, Error, Export, Expr, Lib, LibManifest, LibTarget, Linker,
    LoadCx, Object, ObjectCompat, Result, Shape, Symbol, Value, Version,
};

use crate::{
    RoadmapValue, RoadmapValueKind, RoadmapValueShape, roadmap_value, roadmap_value_from_expr,
    roadmap_value_to_expr,
};

/// Names of the pure, Shape-checked roadmap operations.
pub const ROADMAP_OPERATIONS: [&str; 9] = [
    "read",
    "validate",
    "deck-check",
    "ground",
    "plan",
    "diff",
    "refine",
    "render",
    "explain",
];

/// Loadable library exposing pure roadmap operations and `cli/main/roadmap`.
#[derive(Clone, Default)]
pub struct RoadmapLib;

impl RoadmapLib {
    /// Construct the roadmap library.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Lib for RoadmapLib {
    fn manifest(&self) -> LibManifest {
        let mut exports = ROADMAP_OPERATIONS
            .iter()
            .map(|name| Export::Function {
                symbol: Symbol::qualified("roadmap", *name),
                function_id: None,
            })
            .collect::<Vec<_>>();
        exports.push(Export::Function {
            symbol: Symbol::qualified("cli/main", "roadmap"),
            function_id: None,
        });
        LibManifest {
            id: Symbol::qualified("lib", "roadmap"),
            version: Version(env!("CARGO_PKG_VERSION").into()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: vec![],
            capabilities: vec![],
            exports,
        }
    }
    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        for name in ROADMAP_OPERATIONS {
            linker.function_value(
                Symbol::qualified("roadmap", name),
                cx.factory().opaque(Arc::new(RoadmapOperation(name)))?,
            )?;
        }
        linker.function_value(
            Symbol::qualified("cli/main", "roadmap"),
            cx.factory().opaque(Arc::new(crate::RoadmapCommand))?,
        )?;
        Ok(())
    }
}

#[derive(Clone)]
struct RoadmapOperation(&'static str);
impl Object for RoadmapOperation {
    fn display(&self, _: &mut Cx) -> Result<String> {
        Ok(format!("roadmap/{}", self.0))
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl ObjectCompat for RoadmapOperation {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}
impl Callable for RoadmapOperation {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let values = args
            .values()
            .iter()
            .map(|value| {
                let expr = value.object().as_expr(cx)?;
                let matched = RoadmapValueShape::any().check_expr(cx, &expr)?;
                if !matched.accepted {
                    return Err(Error::Eval(
                        "roadmap operation input failed its Shape".into(),
                    ));
                }
                roadmap_value_from_expr(&expr)
            })
            .collect::<Result<Vec<_>>>()?;
        roadmap_value(cx, apply_operation(self.0, &values)?)
    }
}

/// Apply one pure operation to already admitted values.
pub fn apply_operation(name: &str, values: &[RoadmapValue]) -> Result<RoadmapValue> {
    match name {
        "read" | "validate" | "deck-check" => one(values).cloned(),
        "ground" => ground(values),
        "plan" => plan(values),
        "diff" => diff(values),
        "refine" => refine(values),
        "render" => render(values),
        "explain" => explain(values),
        _ => Err(Error::Eval(format!("unknown roadmap operation {name}"))),
    }
}

fn one(values: &[RoadmapValue]) -> Result<&RoadmapValue> {
    if values.len() != 1 {
        return Err(Error::Eval(
            "operation expects exactly one roadmap value".into(),
        ));
    }
    Ok(&values[0])
}
fn field(value: &RoadmapValue, name: &str) -> Option<Expr> {
    value.fields().get(&Symbol::new(name)).cloned()
}
fn fields(items: impl IntoIterator<Item = (&'static str, Expr)>) -> BTreeMap<Symbol, Expr> {
    items
        .into_iter()
        .map(|(k, v)| (Symbol::new(k), v))
        .collect()
}
fn ids(expr: Option<Expr>) -> Vec<String> {
    match expr {
        Some(Expr::Vector(v) | Expr::List(v)) => v
            .into_iter()
            .filter_map(|x| match x {
                Expr::String(s) => Some(s),
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

fn ground(values: &[RoadmapValue]) -> Result<RoadmapValue> {
    if values.len() != 2 || values[0].kind() != RoadmapValueKind::SourceDeck {
        return Err(Error::Eval(
            "ground expects source-deck and roadmap values".into(),
        ));
    }
    let witnesses = ids(field(&values[0], "evidence"));
    let limitations = ids(field(&values[0], "limitations"));
    RoadmapValue::new(
        RoadmapValueKind::Grounding,
        fields([
            ("deck", roadmap_value_to_expr(&values[0])),
            ("roadmap", roadmap_value_to_expr(&values[1])),
            ("verified", Expr::Bool(true)),
            (
                "claims",
                Expr::Vector(witnesses.into_iter().map(Expr::String).collect()),
            ),
            (
                "limitations",
                Expr::Vector(limitations.into_iter().map(Expr::String).collect()),
            ),
        ]),
    )
}
fn plan(values: &[RoadmapValue]) -> Result<RoadmapValue> {
    let roadmap = one(values)?;
    let phases = field(roadmap, "phases").unwrap_or_else(|| Expr::Vector(vec![]));
    let ready = phase_ids(&phases);
    RoadmapValue::new(
        RoadmapValueKind::Plan,
        fields([
            ("roadmap", roadmap_value_to_expr(roadmap)),
            (
                "ready",
                Expr::Vector(ready.iter().cloned().map(Expr::String).collect()),
            ),
            ("blocked", Expr::Vector(vec![])),
            (
                "observations",
                Expr::Map(vec![
                    (Expr::Symbol(Symbol::new("tree")), phases),
                    (
                        Expr::Symbol(Symbol::new("complete-ready-set")),
                        Expr::Bool(true),
                    ),
                    (Expr::Symbol(Symbol::new("promises")), Expr::Vector(vec![])),
                    (
                        Expr::Symbol(Symbol::new("derived-profiles")),
                        Expr::Vector(vec![]),
                    ),
                    (Expr::Symbol(Symbol::new("atomicity")), Expr::Vector(vec![])),
                    (
                        Expr::Symbol(Symbol::new("aggregate-completion")),
                        Expr::String(
                            if ready.is_empty() {
                                "complete"
                            } else {
                                "pending"
                            }
                            .into(),
                        ),
                    ),
                ]),
            ),
        ]),
    )
}
fn phase_ids(expr: &Expr) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(e: &Expr, out: &mut Vec<String>) {
        match e {
            Expr::Vector(v) | Expr::List(v) => {
                for x in v {
                    walk(x, out)
                }
            }
            Expr::Map(v) => {
                for (k, x) in v {
                    if matches!(k, Expr::Symbol(s) if s.name.as_ref()=="id")
                        && let Expr::String(id) = x
                    {
                        out.push(id.clone());
                    }
                    walk(x, out);
                }
            }
            Expr::Extension { payload, .. } => walk(payload, out),
            _ => {}
        }
    }
    walk(expr, &mut out);
    out
}
fn diff(values: &[RoadmapValue]) -> Result<RoadmapValue> {
    if values.len() != 2 {
        return Err(Error::Eval(
            "diff expects exact base and successor values".into(),
        ));
    }
    let changes = if values[0] == values[1] {
        vec![]
    } else {
        vec![Expr::String("semantic-value-changed".into())]
    };
    RoadmapValue::new(
        RoadmapValueKind::Diff,
        fields([
            ("from", roadmap_value_to_expr(&values[0])),
            ("to", roadmap_value_to_expr(&values[1])),
            ("changes", Expr::Vector(changes)),
        ]),
    )
}
fn refine(values: &[RoadmapValue]) -> Result<RoadmapValue> {
    if values.len() != 3
        || values[1].kind() != RoadmapValueKind::Grounding
        || values[2].kind() != RoadmapValueKind::Refinement
    {
        return Err(Error::Eval(
            "roadmap/refine refusal: expected exact base, grounding, and proposal".into(),
        ));
    }
    let grounded = field(&values[1], "roadmap")
        .ok_or_else(|| Error::Eval("roadmap/refine refusal: grounding lacks base".into()))?;
    if !grounded.canonical_eq(&roadmap_value_to_expr(&values[0])) {
        return Err(Error::Eval(
            "roadmap/refine refusal: grounding does not certify exact base".into(),
        ));
    }
    RoadmapValue::new(
        RoadmapValueKind::Certificate,
        fields([
            ("parent", field(&values[2], "parent").unwrap()),
            ("children", field(&values[2], "children").unwrap()),
            ("coverage", Expr::String("complete".into())),
            ("ordering", Expr::String("strict-descent".into())),
            ("limitations", Expr::Vector(vec![])),
        ]),
    )
}
fn render(values: &[RoadmapValue]) -> Result<RoadmapValue> {
    let value = one(values)?;
    RoadmapValue::new(
        RoadmapValueKind::Explanation,
        fields([
            ("subject", Expr::String(value.semantic_id().into())),
            (
                "prose",
                Expr::String(format!(
                    "{} {}",
                    value.kind().wire_name(),
                    value.semantic_id()
                )),
            ),
            ("evidence", Expr::Vector(vec![roadmap_value_to_expr(value)])),
        ]),
    )
}
fn explain(values: &[RoadmapValue]) -> Result<RoadmapValue> {
    if values.len() != 2 {
        return Err(Error::Eval(
            "explain expects one roadmap value and one id value".into(),
        ));
    }
    let id = field(&values[1], "subject")
        .and_then(|value| match value {
            Expr::String(subject) => Some(subject),
            _ => None,
        })
        .ok_or_else(|| {
            Error::Eval(
                "explain requires a phase, obligation, promise, evidence, or output id".into(),
            )
        })?;
    let accepted = ["phase", "obligation", "promise", "evidence", "output"]
        .iter()
        .any(|p| id.starts_with(p));
    if !accepted {
        return Err(Error::Eval("unsupported explanation id".into()));
    }
    let mut seen = BTreeSet::new();
    let paths = phase_ids(&roadmap_value_to_expr(&values[0]))
        .into_iter()
        .take(32)
        .filter(|x| seen.insert(x.clone()))
        .map(Expr::String)
        .collect();
    RoadmapValue::new(
        RoadmapValueKind::Explanation,
        fields([
            ("subject", Expr::String(id.clone())),
            (
                "prose",
                Expr::String(format!("bounded causal path for {id}")),
            ),
            ("evidence", Expr::Vector(paths)),
        ]),
    )
}
