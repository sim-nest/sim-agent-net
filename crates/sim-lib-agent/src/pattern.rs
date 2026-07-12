//! Agent-pattern descriptors used by deterministic recipes and browse surfaces.

use crate::{keyword, stringish_from_value, symbol_from_value, value_from_expr};
use sim_kernel::{Args, ClassRef, Cx, Error, Expr, Object, Result, Symbol, Value};
use std::{any::Any, collections::BTreeMap, sync::Arc};

const PATTERN_KIND: &str = "agent-pattern";
const FIELD_NAMES: [&str; 10] = [
    "sense",
    "model",
    "plan",
    "act",
    "memory",
    "tools",
    "policy",
    "evaluation",
    "guardrail",
    "trace",
];

/// Descriptor of an agent pattern: an id, description, and named slot groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentPattern {
    /// Stable identifier for the pattern.
    pub id: Symbol,
    /// Optional human-readable description.
    pub description: Option<String>,
    /// Slots describing how the pattern senses its environment.
    pub sense: Vec<AgentPatternSlot>,
    /// Slots describing the models the pattern uses.
    pub model: Vec<AgentPatternSlot>,
    /// Slots describing planning behavior.
    pub plan: Vec<AgentPatternSlot>,
    /// Slots describing actions the pattern takes.
    pub act: Vec<AgentPatternSlot>,
    /// Slots describing memory backends.
    pub memory: Vec<AgentPatternSlot>,
    /// Slots describing available tools.
    pub tools: Vec<AgentPatternSlot>,
    /// Slots describing policy constraints.
    pub policy: Vec<AgentPatternSlot>,
    /// Slots describing evaluation steps.
    pub evaluation: Vec<AgentPatternSlot>,
    /// Slots describing guardrails.
    pub guardrail: Vec<AgentPatternSlot>,
    /// Slots describing trace and observability.
    pub trace: Vec<AgentPatternSlot>,
    /// Extra caller-supplied table entries preserved verbatim.
    pub extra: Vec<(Symbol, Expr)>,
}

/// One named entry within an agent-pattern slot group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentPatternSlot {
    /// Slot name.
    pub name: Symbol,
    /// Optional human-readable description.
    pub description: Option<String>,
    /// Extra caller-supplied table entries preserved verbatim.
    pub extra: Vec<(Symbol, Expr)>,
}

impl AgentPattern {
    /// Encodes the pattern as a table expression for recipes and browse surfaces.
    pub fn as_expr(&self) -> Expr {
        let mut entries = vec![
            key_expr("kind", Expr::Symbol(Symbol::new(PATTERN_KIND))),
            key_expr("id", Expr::Symbol(self.id.clone())),
        ];
        if let Some(description) = &self.description {
            entries.push(key_expr("description", Expr::String(description.clone())));
        }
        entries.push(key_expr("card", self.card_expr()));
        for (name, slots) in self.slot_groups() {
            entries.push(key_expr(name, slots_expr(slots)));
        }
        entries.extend(
            self.extra
                .iter()
                .cloned()
                .map(|(key, value)| (Expr::Symbol(key), value)),
        );
        Expr::Map(entries)
    }

    fn card_expr(&self) -> Expr {
        let mut entries = vec![
            key_expr("kind", Expr::Symbol(Symbol::new(PATTERN_KIND))),
            key_expr("id", Expr::Symbol(self.id.clone())),
            key_expr(
                "fields",
                Expr::List(
                    FIELD_NAMES
                        .iter()
                        .map(|name| Expr::Symbol(Symbol::new(*name)))
                        .collect(),
                ),
            ),
        ];
        if let Some(description) = &self.description {
            entries.push(key_expr("description", Expr::String(description.clone())));
        }
        Expr::Map(entries)
    }

    fn slot_groups(&self) -> [(&'static str, &[AgentPatternSlot]); 10] {
        [
            ("sense", &self.sense),
            ("model", &self.model),
            ("plan", &self.plan),
            ("act", &self.act),
            ("memory", &self.memory),
            ("tools", &self.tools),
            ("policy", &self.policy),
            ("evaluation", &self.evaluation),
            ("guardrail", &self.guardrail),
            ("trace", &self.trace),
        ]
    }
}

impl Object for AgentPattern {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<agent-pattern {}>", self.id))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for AgentPattern {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Table"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            sim_kernel::CORE_TABLE_CLASS_ID,
            Symbol::qualified("core", "Table"),
        )
    }

    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(self.as_expr())
    }

    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        value_from_expr(cx, &self.as_expr())
    }
}

impl AgentPatternSlot {
    fn as_expr(&self) -> Expr {
        let mut entries = vec![key_expr("name", Expr::Symbol(self.name.clone()))];
        if let Some(description) = &self.description {
            entries.push(key_expr("description", Expr::String(description.clone())));
        }
        entries.extend(
            self.extra
                .iter()
                .cloned()
                .map(|(key, value)| (Expr::Symbol(key), value)),
        );
        Expr::Map(entries)
    }
}

pub(crate) fn agent_pattern_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let mut options = parse_pattern_options(cx, args)?;
    let id = required_symbol_option(cx, &mut options, "id", "agent/pattern requires :id")?;
    let description = optional_string(cx, &mut options, "description")?;
    let pattern = AgentPattern {
        id,
        description,
        sense: slot_option(cx, &mut options, "sense")?,
        model: slot_option(cx, &mut options, "model")?,
        plan: slot_option(cx, &mut options, "plan")?,
        act: slot_option(cx, &mut options, "act")?,
        memory: slot_option(cx, &mut options, "memory")?,
        tools: slot_option(cx, &mut options, "tools")?,
        policy: slot_option(cx, &mut options, "policy")?,
        evaluation: slot_option(cx, &mut options, "evaluation")?,
        guardrail: slot_option(cx, &mut options, "guardrail")?,
        trace: slot_option(cx, &mut options, "trace")?,
        extra: extra_options(cx, options)?,
    };
    cx.factory().opaque(Arc::new(pattern))
}

fn parse_pattern_options(cx: &mut Cx, args: Args) -> Result<BTreeMap<String, Value>> {
    if !args.values().len().is_multiple_of(2) {
        return Err(Error::Eval(
            "agent/pattern options must be key/value pairs".to_owned(),
        ));
    }
    let mut options = BTreeMap::new();
    for pair in args.values().chunks(2) {
        let key = keyword(&pair[0].object().as_expr(cx)?)?;
        options.insert(key, pair[1].clone());
    }
    Ok(options)
}

fn required_symbol_option(
    cx: &mut Cx,
    options: &mut BTreeMap<String, Value>,
    key: &str,
    message: &'static str,
) -> Result<Symbol> {
    let Some(value) = options.remove(key) else {
        return Err(Error::Eval(message.to_owned()));
    };
    symbol_from_value(cx, value, "expected symbol option")
}

fn optional_string(
    cx: &mut Cx,
    options: &mut BTreeMap<String, Value>,
    key: &str,
) -> Result<Option<String>> {
    options
        .remove(key)
        .map(|value| stringish_from_value(cx, value, "expected string or symbol option"))
        .transpose()
}

fn slot_option(
    cx: &mut Cx,
    options: &mut BTreeMap<String, Value>,
    key: &str,
) -> Result<Vec<AgentPatternSlot>> {
    let Some(value) = options.remove(key) else {
        return Ok(Vec::new());
    };
    match value.object().as_expr(cx)? {
        Expr::Nil => Ok(Vec::new()),
        Expr::List(items) | Expr::Vector(items) => items.iter().map(slot_from_expr).collect(),
        expr => Ok(vec![slot_from_expr(&expr)?]),
    }
}

fn slot_from_expr(expr: &Expr) -> Result<AgentPatternSlot> {
    match expr {
        Expr::Symbol(symbol) => Ok(slot(symbol.clone(), None, Vec::new())),
        Expr::String(text) => Ok(slot(Symbol::new(text.clone()), None, Vec::new())),
        Expr::Map(entries) => slot_from_map(entries),
        _ => Err(Error::TypeMismatch {
            expected: "agent-pattern slot",
            found: "non-slot",
        }),
    }
}

fn slot_from_map(entries: &[(Expr, Expr)]) -> Result<AgentPatternSlot> {
    let name = map_field(entries, "name")
        .map(symbol_from_expr)
        .transpose()?
        .ok_or_else(|| Error::Eval("agent-pattern slot requires name".to_owned()))?;
    let description = map_field(entries, "description")
        .map(stringish_from_expr)
        .transpose()?;
    let extra = entries
        .iter()
        .filter_map(|(key, value)| {
            let Expr::Symbol(symbol) = key else {
                return Some(Err(Error::TypeMismatch {
                    expected: "symbol table key",
                    found: "non-symbol",
                }));
            };
            if symbol.name.as_ref() == "name" || symbol.name.as_ref() == "description" {
                return None;
            }
            Some(Ok((symbol.clone(), value.clone())))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(slot(name, description, extra))
}

fn extra_options(cx: &mut Cx, options: BTreeMap<String, Value>) -> Result<Vec<(Symbol, Expr)>> {
    options
        .into_iter()
        .map(|(key, value)| Ok((Symbol::new(key), value.object().as_expr(cx)?)))
        .collect()
}

fn slot(name: Symbol, description: Option<String>, extra: Vec<(Symbol, Expr)>) -> AgentPatternSlot {
    AgentPatternSlot {
        name,
        description,
        extra,
    }
}

fn slots_expr(slots: &[AgentPatternSlot]) -> Expr {
    Expr::List(slots.iter().map(AgentPatternSlot::as_expr).collect())
}

fn key_expr(key: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(key)), value)
}

use sim_value::access::entry_field as map_field;

fn symbol_from_expr(expr: &Expr) -> Result<Symbol> {
    match expr {
        Expr::Symbol(symbol) => Ok(symbol.clone()),
        Expr::String(text) => Ok(Symbol::new(text.clone())),
        _ => Err(Error::Eval("expected symbol or string".to_owned())),
    }
}

fn stringish_from_expr(expr: &Expr) -> Result<String> {
    match expr {
        Expr::String(text) => Ok(text.clone()),
        Expr::Symbol(symbol) => Ok(symbol.to_string()),
        _ => Err(Error::Eval("expected string or symbol".to_owned())),
    }
}
