use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};

use crate::{SkillCacheMode, SkillCard, SkillCassetteMode};

#[derive(Clone, Default)]
pub(crate) struct SkillCallState {
    inner: Arc<Mutex<SkillCallStateInner>>,
}

#[derive(Default)]
struct SkillCallStateInner {
    cache: BTreeMap<String, Expr>,
    cassette: BTreeMap<String, Expr>,
}

impl SkillCallState {
    pub(crate) fn cache_lookup(&self, key: &str) -> Result<Option<Expr>> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| Error::PoisonedLock("skill call state"))?
            .cache
            .get(key)
            .cloned())
    }

    pub(crate) fn cache_store(&self, key: String, value: Expr) -> Result<()> {
        self.inner
            .lock()
            .map_err(|_| Error::PoisonedLock("skill call state"))?
            .cache
            .insert(key, value);
        Ok(())
    }

    pub(crate) fn cassette_lookup(&self, key: &str) -> Result<Option<Expr>> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| Error::PoisonedLock("skill call state"))?
            .cassette
            .get(key)
            .cloned())
    }

    pub(crate) fn cassette_store(&self, key: String, value: Expr) -> Result<()> {
        self.inner
            .lock()
            .map_err(|_| Error::PoisonedLock("skill call state"))?
            .cassette
            .insert(key, value);
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SkillAuditEntry {
    pub skill_id: String,
    pub transport_kind: String,
    pub outcome: &'static str,
    pub cache: &'static str,
    pub cassette: &'static str,
    pub privacy: String,
    pub capabilities: Vec<String>,
}

impl SkillAuditEntry {
    pub(crate) fn value(&self, cx: &mut Cx) -> Result<Value> {
        let capabilities = cx.factory().list(
            self.capabilities
                .iter()
                .map(|capability| cx.factory().string(capability.clone()))
                .collect::<Result<Vec<_>>>()?,
        )?;
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory()
                    .symbol(Symbol::qualified("skill", "audit-entry"))?,
            ),
            (
                Symbol::new("skill-id"),
                cx.factory().string(self.skill_id.clone())?,
            ),
            (
                Symbol::new("transport-kind"),
                cx.factory().string(self.transport_kind.clone())?,
            ),
            (
                Symbol::new("outcome"),
                cx.factory().symbol(Symbol::new(self.outcome))?,
            ),
            (
                Symbol::new("cache"),
                cx.factory().symbol(Symbol::new(self.cache))?,
            ),
            (
                Symbol::new("cassette"),
                cx.factory().symbol(Symbol::new(self.cassette))?,
            ),
            (
                Symbol::new("privacy"),
                cx.factory().string(self.privacy.clone())?,
            ),
            (Symbol::new("capabilities"), capabilities),
            (
                Symbol::new("input"),
                cx.factory()
                    .symbol(Symbol::qualified("skill", "redacted"))?,
            ),
            (
                Symbol::new("output"),
                cx.factory()
                    .symbol(Symbol::qualified("skill", "redacted"))?,
            ),
        ])
    }
}

pub(crate) fn call_key(cx: &mut Cx, card: &SkillCard, args: &Value) -> Result<String> {
    let args_expr = args.object().as_expr(cx)?;
    let input_shape = card.input_shape.object().as_expr(cx)?;
    let output_shape = card.output_shape.object().as_expr(cx)?;
    let mut capabilities = card
        .capabilities
        .iter()
        .map(|capability| capability.as_str().to_owned())
        .collect::<Vec<_>>();
    capabilities.sort();
    Ok(format!(
        "skill-call-v1;id={};semantic={};privacy={};caps={};input-shape={};output-shape={};args={}",
        card.id,
        card.policy.semantic_key.as_deref().unwrap_or(""),
        card.policy.privacy.as_symbol(),
        capabilities.join(","),
        normalized_expr(&input_shape),
        normalized_expr(&output_shape),
        normalized_expr(&args_expr)
    ))
}

pub(crate) fn value_from_recorded_expr(cx: &mut Cx, expr: Expr) -> Result<Value> {
    match expr {
        Expr::Nil => cx.factory().nil(),
        Expr::Bool(value) => cx.factory().bool(value),
        Expr::Number(number) => cx.factory().number_literal(number.domain, number.canonical),
        Expr::Symbol(symbol) => cx.factory().symbol(symbol),
        Expr::String(text) => cx.factory().string(text),
        Expr::Bytes(bytes) => cx.factory().bytes(bytes),
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) | Expr::Block(items) => {
            let values = items
                .into_iter()
                .map(|item| value_from_recorded_expr(cx, item))
                .collect::<Result<Vec<_>>>()?;
            cx.factory().list(values)
        }
        Expr::Map(entries) => {
            let mut values = Vec::new();
            for (key, value) in entries {
                let Expr::Symbol(key) = key else {
                    return cx.factory().expr(Expr::Map(vec![(key, value)]));
                };
                values.push((key, value_from_recorded_expr(cx, value)?));
            }
            cx.factory().table(values)
        }
        other => cx.factory().expr(other),
    }
}

pub(crate) fn cache_read_allowed(mode: &SkillCacheMode) -> bool {
    matches!(mode, SkillCacheMode::ReadThrough | SkillCacheMode::ReadOnly)
}

pub(crate) fn cache_write_allowed(mode: &SkillCacheMode) -> bool {
    matches!(
        mode,
        SkillCacheMode::ReadThrough | SkillCacheMode::WriteOnly | SkillCacheMode::Refresh
    )
}

pub(crate) fn cassette_replay_allowed(mode: &SkillCassetteMode) -> bool {
    matches!(
        mode,
        SkillCassetteMode::RecordReplay | SkillCassetteMode::ReplayOnly
    )
}

pub(crate) fn cassette_record_allowed(mode: &SkillCassetteMode) -> bool {
    matches!(
        mode,
        SkillCassetteMode::RecordReplay | SkillCassetteMode::RecordOnly
    )
}

fn normalized_expr(expr: &Expr) -> String {
    match expr {
        Expr::Nil => "nil".to_owned(),
        Expr::Bool(value) => format!("bool:{value}"),
        Expr::Number(number) => format!("number:{}:{}", number.domain, number.canonical),
        Expr::Symbol(symbol) => format!("symbol:{symbol}"),
        Expr::Local(symbol) => format!("local:{symbol}"),
        Expr::String(text) => format!("string:{text:?}"),
        Expr::Bytes(bytes) => format!("bytes:{bytes:?}"),
        Expr::List(items) => normalized_sequence("list", items),
        Expr::Vector(items) => normalized_sequence("vector", items),
        Expr::Set(items) => {
            let mut items = items.iter().map(normalized_expr).collect::<Vec<_>>();
            items.sort();
            format!("set:[{}]", items.join(","))
        }
        Expr::Map(entries) => {
            let mut entries = entries
                .iter()
                .map(|(key, value)| format!("{}=>{}", normalized_expr(key), normalized_expr(value)))
                .collect::<Vec<_>>();
            entries.sort();
            format!("map:{{{}}}", entries.join(","))
        }
        Expr::Call { operator, args } => {
            format!(
                "call:{}({})",
                normalized_expr(operator),
                args.iter()
                    .map(normalized_expr)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Expr::Infix {
            operator,
            left,
            right,
        } => format!(
            "infix:{}:{}:{}",
            operator,
            normalized_expr(left),
            normalized_expr(right)
        ),
        Expr::Prefix { operator, arg } => format!("prefix:{operator}:{}", normalized_expr(arg)),
        Expr::Postfix { operator, arg } => {
            format!("postfix:{operator}:{}", normalized_expr(arg))
        }
        Expr::Quote { mode, expr } => format!("quote:{mode:?}:{}", normalized_expr(expr)),
        Expr::Annotated { expr, annotations } => {
            let mut annotations = annotations
                .iter()
                .map(|(key, value)| format!("{key}:{}", normalized_expr(value)))
                .collect::<Vec<_>>();
            annotations.sort();
            format!(
                "annotated:{}:[{}]",
                normalized_expr(expr),
                annotations.join(",")
            )
        }
        Expr::Block(items) => normalized_sequence("block", items),
        Expr::Extension { tag, payload } => {
            format!("extension:{tag}:{}", normalized_expr(payload))
        }
    }
}

fn normalized_sequence(kind: &str, items: &[Expr]) -> String {
    format!(
        "{kind}:[{}]",
        items
            .iter()
            .map(normalized_expr)
            .collect::<Vec<_>>()
            .join(",")
    )
}
