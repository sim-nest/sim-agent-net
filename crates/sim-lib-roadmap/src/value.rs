use std::collections::BTreeMap;

use sim_kernel::{Error, Expr, Result, Symbol};

/// Hard admission bounds applied before cloning an expression body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoadmapValueLimits {
    /// Maximum fields in one value.
    pub fields: usize,
    /// Maximum nodes in the complete expression graph.
    pub expr_nodes: usize,
    /// Maximum bytes across strings, symbols, numbers, and byte strings.
    pub scalar_bytes: usize,
    /// Maximum expression nesting depth.
    pub depth: usize,
}

impl Default for RoadmapValueLimits {
    fn default() -> Self {
        Self {
            fields: 256,
            expr_nodes: 16_384,
            scalar_bytes: 1_048_576,
            depth: 64,
        }
    }
}

/// The closed set of structural roadmap value variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoadmapValueKind {
    SourceDeck,
    Evidence,
    Roadmap,
    RoadmapRevision,
    Phase,
    Guide,
    Promise,
    Profile,
    Atomicity,
    Grounding,
    Refinement,
    Certificate,
    Diff,
    Plan,
    Explanation,
}

impl RoadmapValueKind {
    /// Canonical lower-case wire name.
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::SourceDeck => "source-deck",
            Self::Evidence => "evidence",
            Self::Roadmap => "roadmap",
            Self::RoadmapRevision => "roadmap-revision",
            Self::Phase => "phase",
            Self::Guide => "guide",
            Self::Promise => "promise",
            Self::Profile => "profile",
            Self::Atomicity => "atomicity",
            Self::Grounding => "grounding",
            Self::Refinement => "refinement",
            Self::Certificate => "certificate",
            Self::Diff => "diff",
            Self::Plan => "plan",
            Self::Explanation => "explanation",
        }
    }

    pub(crate) fn parse(name: &str) -> Option<Self> {
        ALL_KINDS.into_iter().find(|kind| kind.wire_name() == name)
    }
}

/// Every admitted structural variant.
pub const ALL_KINDS: [RoadmapValueKind; 15] = [
    RoadmapValueKind::SourceDeck,
    RoadmapValueKind::Evidence,
    RoadmapValueKind::Roadmap,
    RoadmapValueKind::RoadmapRevision,
    RoadmapValueKind::Phase,
    RoadmapValueKind::Guide,
    RoadmapValueKind::Promise,
    RoadmapValueKind::Profile,
    RoadmapValueKind::Atomicity,
    RoadmapValueKind::Grounding,
    RoadmapValueKind::Refinement,
    RoadmapValueKind::Certificate,
    RoadmapValueKind::Diff,
    RoadmapValueKind::Plan,
    RoadmapValueKind::Explanation,
];

/// A validated native roadmap-family value.
#[derive(Clone, Debug)]
pub struct RoadmapValue {
    kind: RoadmapValueKind,
    semantic_id: String,
    fields: BTreeMap<Symbol, Expr>,
}

impl PartialEq for RoadmapValue {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.semantic_id == other.semantic_id
            && self.fields.len() == other.fields.len()
            && self
                .fields
                .iter()
                .all(|(key, value)| other.fields.get(key).is_some_and(|v| value.canonical_eq(v)))
    }
}
impl Eq for RoadmapValue {}

impl RoadmapValue {
    /// Admit a field map under default limits.
    pub fn new(kind: RoadmapValueKind, fields: BTreeMap<Symbol, Expr>) -> Result<Self> {
        Self::with_limits(kind, fields, RoadmapValueLimits::default())
    }

    /// Admit a field map under caller-selected hard limits.
    pub fn with_limits(
        kind: RoadmapValueKind,
        fields: BTreeMap<Symbol, Expr>,
        limits: RoadmapValueLimits,
    ) -> Result<Self> {
        validate_fields(kind, &fields, limits)?;
        let semantic_id = semantic_id(kind, &fields);
        Ok(Self {
            kind,
            semantic_id,
            fields,
        })
    }

    pub fn kind(&self) -> RoadmapValueKind {
        self.kind
    }
    pub fn semantic_id(&self) -> &str {
        &self.semantic_id
    }
    pub fn fields(&self) -> &BTreeMap<Symbol, Expr> {
        &self.fields
    }
}

fn validate_fields(
    kind: RoadmapValueKind,
    fields: &BTreeMap<Symbol, Expr>,
    limits: RoadmapValueLimits,
) -> Result<()> {
    if fields.len() > limits.fields {
        return Err(Error::Eval("roadmap value has too many fields".into()));
    }
    for required in required_fields(kind) {
        if !fields.contains_key(&Symbol::new(*required)) {
            return Err(Error::Eval(format!(
                "{} requires field {required}",
                kind.wire_name()
            )));
        }
    }
    for name in fields.keys() {
        if !allowed_fields(kind).contains(&name.name.as_ref()) && !name.name.starts_with("x-") {
            return Err(Error::Eval(format!(
                "unknown required structural field {} for {}",
                name,
                kind.wire_name()
            )));
        }
    }
    let mut budget = Budget::default();
    for expr in fields.values() {
        measure(expr, 1, &mut budget, limits)?;
    }
    if matches!(kind, RoadmapValueKind::Grounding)
        && !matches!(fields.get(&Symbol::new("verified")), Some(Expr::Bool(true)))
    {
        return Err(Error::Eval("grounding must carry verified=true".into()));
    }
    if matches!(kind, RoadmapValueKind::Promise)
        && matches!(fields.get(&Symbol::new("conclusion")), Some(Expr::String(value)) if value == "inconclusive")
    {
        return Err(Error::Eval(
            "an inconclusive promise is not admissible".into(),
        ));
    }
    Ok(())
}

fn required_fields(kind: RoadmapValueKind) -> &'static [&'static str] {
    match kind {
        RoadmapValueKind::SourceDeck => &["repositories", "evidence"],
        RoadmapValueKind::Evidence => &["subject", "limitations"],
        RoadmapValueKind::Roadmap => &["revision", "phases"],
        RoadmapValueKind::RoadmapRevision => &["roadmap", "revision", "root"],
        RoadmapValueKind::Phase => &["id", "title", "body"],
        RoadmapValueKind::Guide => &["queries", "targets", "promises"],
        RoadmapValueKind::Promise => &["id", "conclusion"],
        RoadmapValueKind::Profile => &["phase", "rank"],
        RoadmapValueKind::Atomicity => &["phase", "atomic"],
        RoadmapValueKind::Grounding => &["deck", "roadmap", "verified"],
        RoadmapValueKind::Refinement => &["parent", "children"],
        RoadmapValueKind::Certificate => &["parent", "children", "coverage"],
        RoadmapValueKind::Diff => &["from", "to", "changes"],
        RoadmapValueKind::Plan => &["roadmap", "ready"],
        RoadmapValueKind::Explanation => &["subject", "prose"],
    }
}

fn allowed_fields(kind: RoadmapValueKind) -> &'static [&'static str] {
    match kind {
        RoadmapValueKind::SourceDeck => &[
            "repositories",
            "fragments",
            "files",
            "evidence",
            "limitations",
        ],
        RoadmapValueKind::Evidence => &["subject", "claims", "witnesses", "limitations"],
        RoadmapValueKind::Roadmap => &["revision", "charter", "phases", "imports", "metadata"],
        RoadmapValueKind::RoadmapRevision => &["roadmap", "revision", "root", "content"],
        RoadmapValueKind::Phase => &[
            "id",
            "parent",
            "title",
            "intent",
            "body",
            "dependencies",
            "owners",
            "resources",
            "effects",
            "capabilities",
            "changes",
            "acceptance",
            "coverage",
            "outputs",
            "guide",
            "origin",
        ],
        RoadmapValueKind::Guide => &["queries", "targets", "promises", "sketches"],
        RoadmapValueKind::Promise => &["id", "query", "target", "conclusion", "evidence"],
        RoadmapValueKind::Profile => &[
            "phase",
            "rank",
            "owners",
            "resources",
            "effects",
            "capabilities",
            "changes",
        ],
        RoadmapValueKind::Atomicity => &["phase", "atomic", "reasons"],
        RoadmapValueKind::Grounding => &["deck", "roadmap", "verified", "claims", "limitations"],
        RoadmapValueKind::Refinement => &["parent", "children", "proposal", "rank"],
        RoadmapValueKind::Certificate => {
            &["parent", "children", "ordering", "coverage", "limitations"]
        }
        RoadmapValueKind::Diff => &["from", "to", "changes", "evidence"],
        RoadmapValueKind::Plan => &["roadmap", "ready", "blocked", "observations", "grounding"],
        RoadmapValueKind::Explanation => &["subject", "prose", "reasons", "evidence"],
    }
}

#[derive(Default)]
struct Budget {
    nodes: usize,
    bytes: usize,
}
fn measure(
    expr: &Expr,
    depth: usize,
    budget: &mut Budget,
    limits: RoadmapValueLimits,
) -> Result<()> {
    if depth > limits.depth {
        return Err(Error::Eval("roadmap value expression is too deep".into()));
    }
    budget.nodes += 1;
    let add = match expr {
        Expr::String(s) => s.len(),
        Expr::Bytes(s) => s.len(),
        Expr::Symbol(s) | Expr::Local(s) => s.to_string().len(),
        Expr::Number(n) => n.canonical.len() + n.domain.to_string().len(),
        _ => 0,
    };
    budget.bytes = budget.bytes.saturating_add(add);
    if budget.nodes > limits.expr_nodes || budget.bytes > limits.scalar_bytes {
        return Err(Error::Eval(
            "roadmap value expression exceeds admission budget".into(),
        ));
    }
    match expr {
        Expr::List(v) | Expr::Vector(v) | Expr::Set(v) | Expr::Block(v) => {
            for x in v {
                measure(x, depth + 1, budget, limits)?;
            }
        }
        Expr::Map(v) => {
            for (k, x) in v {
                measure(k, depth + 1, budget, limits)?;
                measure(x, depth + 1, budget, limits)?;
            }
        }
        Expr::Call { operator, args } => {
            measure(operator, depth + 1, budget, limits)?;
            for x in args {
                measure(x, depth + 1, budget, limits)?;
            }
        }
        Expr::Infix { left, right, .. } => {
            measure(left, depth + 1, budget, limits)?;
            measure(right, depth + 1, budget, limits)?;
        }
        Expr::Prefix { arg, .. }
        | Expr::Postfix { arg, .. }
        | Expr::Quote { expr: arg, .. }
        | Expr::Extension { payload: arg, .. } => measure(arg, depth + 1, budget, limits)?,
        Expr::Annotated { expr, annotations } => {
            measure(expr, depth + 1, budget, limits)?;
            for (_, x) in annotations {
                measure(x, depth + 1, budget, limits)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn semantic_id(kind: RoadmapValueKind, fields: &BTreeMap<Symbol, Expr>) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hash = DefaultHasher::new();
    kind.hash(&mut hash);
    for (key, value) in fields {
        key.hash(&mut hash);
        value.hash(&mut hash);
    }
    format!("roadmap:{}:{:016x}", kind.wire_name(), hash.finish())
}
