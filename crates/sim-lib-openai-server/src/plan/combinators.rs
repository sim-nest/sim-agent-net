use sim_kernel::{Expr, Symbol};

/// Static description of a plan combinator: its name and structural constraints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanCombinator {
    /// Combinator name without the `plan/` prefix (for example `race`).
    pub name: &'static str,
    /// Minimum number of child plans the combinator accepts.
    pub min_children: usize,
    /// Maximum number of child plans the combinator accepts.
    pub max_children: usize,
    /// Keyword argument names the combinator recognizes.
    pub keywords: &'static [&'static str],
    /// Whether the combinator can be executed by the current evaluator.
    pub executable_now: bool,
}

/// Returns the full table of known plan combinators.
pub fn plan_combinators() -> &'static [PlanCombinator] {
    &PLAN_COMBINATORS
}

/// Returns the combinator table encoded as a list of map [`Expr`] records.
pub fn plan_combinators_expr() -> Expr {
    Expr::List(PLAN_COMBINATORS.iter().map(plan_combinator_expr).collect())
}

/// Returns the `plan/<name>` head symbol for a combinator name.
pub fn plan_symbol(name: &str) -> Symbol {
    Symbol::new(format!("plan/{name}"))
}

/// Looks up a combinator by its name, returning `None` if it is not known.
pub fn combinator_by_name(name: &str) -> Option<&'static PlanCombinator> {
    PLAN_COMBINATORS
        .iter()
        .find(|combinator| combinator.name == name)
}

fn plan_combinator_expr(combinator: &PlanCombinator) -> Expr {
    Expr::Map(vec![
        field("name", Expr::Symbol(plan_symbol(combinator.name))),
        field(
            "min-children",
            Expr::String(combinator.min_children.to_string()),
        ),
        field(
            "max-children",
            Expr::String(combinator.max_children.to_string()),
        ),
        field(
            "keywords",
            Expr::List(
                combinator
                    .keywords
                    .iter()
                    .map(|keyword| Expr::Symbol(Symbol::new(*keyword)))
                    .collect(),
            ),
        ),
        field("executable-now", Expr::Bool(combinator.executable_now)),
    ])
}

use sim_value::build::entry as field;

const PLAN_COMBINATORS: [PlanCombinator; 12] = [
    PlanCombinator {
        name: "atom",
        min_children: 1,
        max_children: 1,
        keywords: &[],
        executable_now: true,
    },
    PlanCombinator {
        name: "race",
        min_children: 1,
        max_children: 4,
        keywords: &["deadline"],
        executable_now: true,
    },
    PlanCombinator {
        name: "market",
        min_children: 1,
        max_children: 1,
        keywords: &[],
        executable_now: true,
    },
    PlanCombinator {
        name: "verify",
        min_children: 2,
        max_children: 2,
        keywords: &["on-fail"],
        executable_now: true,
    },
    PlanCombinator {
        name: "debate",
        min_children: 2,
        max_children: 4,
        keywords: &["judge", "rounds"],
        executable_now: true,
    },
    PlanCombinator {
        name: "chain",
        min_children: 1,
        max_children: 4,
        keywords: &[],
        executable_now: true,
    },
    PlanCombinator {
        name: "fallback",
        min_children: 1,
        max_children: 4,
        keywords: &[],
        executable_now: true,
    },
    PlanCombinator {
        name: "cache",
        min_children: 1,
        max_children: 1,
        keywords: &["mode", "ttl"],
        executable_now: true,
    },
    PlanCombinator {
        name: "budget",
        min_children: 1,
        max_children: 1,
        keywords: &["max-cost", "max-latency-ms", "max-tokens"],
        executable_now: true,
    },
    PlanCombinator {
        name: "local",
        min_children: 1,
        max_children: 1,
        keywords: &[],
        executable_now: true,
    },
    PlanCombinator {
        name: "remote",
        min_children: 1,
        max_children: 1,
        keywords: &[],
        executable_now: true,
    },
    PlanCombinator {
        name: "trace",
        min_children: 1,
        max_children: 1,
        keywords: &[],
        executable_now: true,
    },
];
