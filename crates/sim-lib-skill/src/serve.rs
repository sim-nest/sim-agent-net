use sim_kernel::{Args, CapabilityName, Cx, Error, Expr, Result, Symbol, Value};

use crate::{SkillCard, SkillRole};

const DEFAULT_TRANSPORT: &str = "in-process";

#[derive(Clone, Debug)]
struct ServeOptions {
    allow_names: Vec<String>,
    deny_names: Vec<String>,
    granted_capabilities: Vec<CapabilityName>,
    transport: String,
}

impl ServeOptions {
    fn from_args(cx: &mut Cx, args: Args) -> Result<Self> {
        let mut options = Self::from_cx(cx);
        let mut values = args.into_vec();
        match values.len() {
            0 => Ok(options),
            1 => {
                options.apply_expr(&values.remove(0).object().as_expr(cx)?)?;
                options.normalize();
                Ok(options)
            }
            _ => Err(Error::Eval(
                "skill/serve-mcp expects zero arguments or one options map".to_owned(),
            )),
        }
    }

    fn from_cx(cx: &Cx) -> Self {
        let mut options = Self {
            allow_names: Vec::new(),
            deny_names: Vec::new(),
            granted_capabilities: cx.capabilities().iter().cloned().collect(),
            transport: DEFAULT_TRANSPORT.to_owned(),
        };
        options.normalize();
        options
    }

    fn apply_expr(&mut self, expr: &Expr) -> Result<()> {
        let Expr::Map(fields) = expr else {
            return Err(Error::TypeMismatch {
                expected: "skill/serve-mcp options map",
                found: "non-map",
            });
        };
        if let Some(value) = optional_field(fields, &["allow", "allow-names"]) {
            self.allow_names = string_list(value, "allow-names")?;
        }
        if let Some(value) = optional_field(fields, &["deny", "deny-names"]) {
            self.deny_names = string_list(value, "deny-names")?;
        }
        if let Some(value) = optional_field(fields, &["capabilities", "granted-capabilities"]) {
            self.granted_capabilities = capability_list(value)?;
        }
        if let Some(value) = optional_field(fields, &["transport"]) {
            self.transport = string_or_symbol(value, "transport")?;
        }
        Ok(())
    }

    fn normalize(&mut self) {
        self.allow_names.sort();
        self.allow_names.dedup();
        self.deny_names.sort();
        self.deny_names.dedup();
        self.granted_capabilities.sort();
        self.granted_capabilities.dedup();
    }

    fn allows_name(&self, name: &str) -> bool {
        if matches_any(&self.deny_names, name) {
            return false;
        }
        self.allow_names.is_empty() || matches_any(&self.allow_names, name)
    }

    fn grants_card(&self, card: &SkillCard) -> bool {
        card.capabilities.iter().all(|required| {
            self.granted_capabilities
                .iter()
                .any(|granted| granted == required)
        })
    }
}

pub(crate) fn serve_mcp(cx: &mut Cx, args: Args) -> Result<Value> {
    cx.require(&crate::skill_serve_capability())?;
    let options = ServeOptions::from_args(cx, args)?;
    let registry = crate::skill_registry(cx)?;
    let cards = registry.cards()?;
    cx.factory().expr(Expr::Map(vec![
        field(
            "kind",
            Expr::Symbol(Symbol::qualified("skill", "mcp-server")),
        ),
        field("transport", Expr::String(options.transport.clone())),
        field("allow-names", string_exprs(&options.allow_names)),
        field("deny-names", string_exprs(&options.deny_names)),
        field("granted-capabilities", capability_exprs(&options)),
        field("methods", method_exprs()),
        field("tools", selected_names(&cards, SkillRole::Tool, &options)),
        field(
            "resources",
            selected_names(&cards, SkillRole::Resource, &options),
        ),
        field(
            "prompts",
            selected_names(&cards, SkillRole::Prompt, &options),
        ),
    ]))
}

/// Returns the symbol for the `skill/serve-mcp` operation.
pub fn skill_serve_mcp_symbol() -> Symbol {
    Symbol::qualified("skill", "serve-mcp")
}

fn selected_names(cards: &[SkillCard], role: SkillRole, options: &ServeOptions) -> Expr {
    let mut names = cards
        .iter()
        .filter(|card| card.roles.contains(&role))
        .filter(|card| options.allows_name(&card.id))
        .filter(|card| options.grants_card(card))
        .map(|card| card.id.clone())
        .collect::<Vec<_>>();
    names.sort();
    Expr::List(names.into_iter().map(Expr::String).collect())
}

fn method_exprs() -> Expr {
    Expr::List(
        [
            "initialize",
            "tools/list",
            "tools/call",
            "resources/list",
            "resources/read",
            "prompts/list",
            "prompts/get",
        ]
        .into_iter()
        .map(|method| Expr::String(method.to_owned()))
        .collect(),
    )
}

fn string_exprs(values: &[String]) -> Expr {
    Expr::List(values.iter().cloned().map(Expr::String).collect())
}

fn capability_exprs(options: &ServeOptions) -> Expr {
    Expr::List(
        options
            .granted_capabilities
            .iter()
            .map(|capability| Expr::String(capability.as_str().to_owned()))
            .collect(),
    )
}

fn optional_field<'a>(fields: &'a [(Expr, Expr)], names: &[&str]) -> Option<&'a Expr> {
    fields.iter().find_map(|(key, value)| {
        let name = field_name(key)?;
        names.contains(&name.as_str()).then_some(value)
    })
}

fn field_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Symbol(symbol) if symbol.namespace.is_none() => Some(symbol.name.to_string()),
        Expr::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn string_list(expr: &Expr, expected: &'static str) -> Result<Vec<String>> {
    match expr {
        Expr::List(items) => items
            .iter()
            .map(|item| string_or_symbol(item, expected))
            .collect(),
        Expr::Nil => Ok(Vec::new()),
        _ => Err(Error::TypeMismatch {
            expected,
            found: "non-list",
        }),
    }
}

fn capability_list(expr: &Expr) -> Result<Vec<CapabilityName>> {
    string_list(expr, "capability list")
        .map(|items| items.into_iter().map(CapabilityName::new).collect())
}

fn string_or_symbol(expr: &Expr, expected: &'static str) -> Result<String> {
    match expr {
        Expr::String(value) => Ok(value.clone()),
        Expr::Symbol(symbol) if symbol.namespace.as_deref() == Some("capability") => {
            Ok(symbol.name.to_string())
        }
        Expr::Symbol(symbol) if symbol.namespace.is_none() => Ok(symbol.name.to_string()),
        _ => Err(Error::TypeMismatch {
            expected,
            found: "invalid value",
        }),
    }
}

fn matches_any(patterns: &[String], name: &str) -> bool {
    patterns.iter().any(|pattern| glob_matches(pattern, name))
}

fn glob_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" || pattern == name {
        return true;
    }
    if !pattern.contains('*') {
        return false;
    }
    let mut rest = name;
    let anchored_start = !pattern.starts_with('*');
    let anchored_end = !pattern.ends_with('*');
    let parts = pattern.split('*').filter(|part| !part.is_empty());
    let mut first = true;
    for part in parts {
        if first && anchored_start {
            let Some(tail) = rest.strip_prefix(part) else {
                return false;
            };
            rest = tail;
        } else {
            let Some(index) = rest.find(part) else {
                return false;
            };
            rest = &rest[index + part.len()..];
        }
        first = false;
    }
    !anchored_end || rest.is_empty()
}

use sim_value::build::entry as field;

#[cfg(test)]
mod tests;
