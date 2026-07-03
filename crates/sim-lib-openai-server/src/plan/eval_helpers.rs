use sim_kernel::Expr;
use sim_lib_agent_runner_core::ModelResponse;

use crate::plan::{fixtures::response_text, shape::plan_parts};

pub(super) fn child_args(args: &[Expr]) -> Vec<&Expr> {
    args.iter()
        .filter(|arg| keyword_name(arg).is_none())
        .collect()
}

pub(super) fn keyword_atom<'a>(args: &'a [Expr], name: &str) -> Option<&'a str> {
    keyword_value(args, name).and_then(atom_address)
}

pub(super) fn keyword_value<'a>(args: &'a [Expr], name: &str) -> Option<&'a Expr> {
    args.iter().find_map(|arg| {
        let Expr::Map(entries) = arg else {
            return None;
        };
        let found_name = entries.iter().find_map(|(key, value)| match (key, value) {
            (Expr::Symbol(key), Expr::Symbol(value))
                if key.namespace.is_none() && key.name.as_ref() == "keyword" =>
            {
                Some(value.name.as_ref())
            }
            _ => None,
        })?;
        if found_name != name {
            return None;
        }
        entries.iter().find_map(|(key, value)| match key {
            Expr::Symbol(key) if key.namespace.is_none() && key.name.as_ref() == "value" => {
                Some(value)
            }
            _ => None,
        })
    })
}

pub(super) fn is_slow_fixture(plan: &Expr) -> bool {
    atom_address(plan) == Some("fixture/slow-echo")
}

pub(super) fn verifier_accepts(response: &ModelResponse) -> bool {
    let text = response_text(response);
    text.contains("ok") || text.contains("accept")
}

pub(super) fn request_field_text(expr: &Expr, name: &str) -> Option<String> {
    request_field(expr, name).and_then(|value| match value {
        Expr::String(value) => Some(value.clone()),
        Expr::Symbol(value) => Some(value.name.as_ref().to_owned()),
        _ => None,
    })
}

pub(super) fn request_field<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(key, value)| match (key, value) {
        (Expr::Symbol(key), value) if key.namespace.is_none() && key.name.as_ref() == name => {
            Some(value)
        }
        _ => None,
    })
}

pub(super) fn branch_id(branch: u64) -> Expr {
    Expr::String(format!("branch-{branch}"))
}

pub(super) use sim_value::build::entry as field;

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

fn atom_address(plan: &Expr) -> Option<&str> {
    let Ok(("atom", [Expr::String(address)])) = plan_parts(plan) else {
        return None;
    };
    Some(address)
}
