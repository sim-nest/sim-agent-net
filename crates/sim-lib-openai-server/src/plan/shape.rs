use sim_kernel::{Error, Expr, Result, Symbol};

use crate::plan::combinators::combinator_by_name;

/// Structural limits enforced when checking a plan expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanLimits {
    /// Maximum nesting depth permitted for the plan tree.
    pub max_depth: usize,
    /// Maximum number of children any single combinator may have.
    pub max_fan_out: usize,
}

impl Default for PlanLimits {
    fn default() -> Self {
        Self {
            max_depth: 4,
            max_fan_out: 4,
        }
    }
}

/// Validates a plan expression against the default [`PlanLimits`].
pub fn check_plan(plan: &Expr) -> Result<()> {
    check_plan_with_limits(plan, PlanLimits::default())
}

/// Validates a plan expression against the supplied structural limits.
pub fn check_plan_with_limits(plan: &Expr, limits: PlanLimits) -> Result<()> {
    check_plan_at(plan, &limits, 1)
}

/// Returns an indented, human-readable outline of a validated plan tree.
pub fn explain_plan(plan: &Expr) -> Result<String> {
    check_plan(plan)?;
    let mut lines = Vec::new();
    explain_plan_at(plan, 0, &mut lines)?;
    Ok(lines.join("\n"))
}

fn check_plan_at(plan: &Expr, limits: &PlanLimits, depth: usize) -> Result<()> {
    if depth > limits.max_depth {
        return Err(Error::Eval(format!(
            "plan nesting exceeds maximum depth {}",
            limits.max_depth
        )));
    }
    let (name, args) = plan_parts(plan)?;
    if name == "atom" {
        return check_atom(args);
    }

    let Some(combinator) = combinator_by_name(name) else {
        return Err(Error::Eval(format!("unknown plan combinator {name}")));
    };
    let children = args.iter().filter(|arg| !is_keyword_arg(arg)).count();
    if children < combinator.min_children || children > combinator.max_children {
        return Err(Error::Eval(format!(
            "plan/{name} expects {}..{} children, found {children}",
            combinator.min_children, combinator.max_children
        )));
    }
    if children > limits.max_fan_out {
        return Err(Error::Eval(format!(
            "plan/{name} fan-out {children} exceeds maximum {}",
            limits.max_fan_out
        )));
    }
    for arg in args {
        if let Some(keyword) = keyword_name(arg) {
            if !combinator.keywords.contains(&keyword) {
                return Err(Error::Eval(format!(
                    "unknown keyword {keyword} for plan/{name}"
                )));
            }
            continue;
        }
        check_plan_at(arg, limits, depth + 1)?;
    }
    Ok(())
}

fn check_atom(args: &[Expr]) -> Result<()> {
    match args {
        [Expr::String(address)] if !address.trim().is_empty() => Ok(()),
        [_] => Err(Error::Eval("plan/atom address must be a string".to_owned())),
        _ => Err(Error::Eval("plan/atom expects one address".to_owned())),
    }
}

fn explain_plan_at(plan: &Expr, indent: usize, lines: &mut Vec<String>) -> Result<()> {
    let (name, args) = plan_parts(plan)?;
    let prefix = "  ".repeat(indent);
    if name == "atom" {
        let [Expr::String(address)] = args else {
            return Err(Error::Eval("plan/atom expects one address".to_owned()));
        };
        lines.push(format!("{prefix}plan/atom {address}"));
        return Ok(());
    }
    lines.push(format!("{prefix}plan/{name}"));
    for arg in args {
        if let Some(keyword) = keyword_name(arg) {
            lines.push(format!("{prefix}  keyword {keyword}"));
        } else {
            explain_plan_at(arg, indent + 1, lines)?;
        }
    }
    Ok(())
}

pub(crate) fn plan_parts(plan: &Expr) -> Result<(&str, &[Expr])> {
    let Expr::List(items) = plan else {
        return Err(Error::Eval("plan must be a list expression".to_owned()));
    };
    let Some((Expr::Symbol(symbol), args)) = items.split_first() else {
        return Err(Error::Eval("plan must have a symbolic head".to_owned()));
    };
    plan_name(symbol).map(|name| (name, args))
}

fn plan_name(symbol: &Symbol) -> Result<&str> {
    let name = symbol.name.as_ref();
    name.strip_prefix("plan/")
        .ok_or_else(|| Error::Eval(format!("plan head must use plan/*, found {name}")))
}

fn is_keyword_arg(expr: &Expr) -> bool {
    keyword_name(expr).is_some()
}

fn keyword_name(expr: &Expr) -> Option<&str> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(key, value)| match (key, value) {
        (Expr::Symbol(key), Expr::Symbol(name))
            if key.namespace.is_none() && key.name.as_ref() == "keyword" =>
        {
            Some(name.name.as_ref())
        }
        _ => None,
    })
}
