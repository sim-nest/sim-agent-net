use std::collections::BTreeMap;

use sim_kernel::{ContentId, Cx, Error, Expr, Result};

use crate::{
    capabilities::ai_runner_cache_capability,
    content_id::{content_id_for_expr, request_expr_content_id},
    objects::content_id_expr,
};

/// Identifies a cached plan evaluation by the content ids of its request and plan.
///
/// Two evaluations collide in the plan cache only when both the request and the
/// plan hash to the same [`ContentId`], so the same plan run against different
/// requests (or different plans for the same request) stay distinct entries.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlanCacheKey {
    request_content_id: ContentId,
    plan_content_id: ContentId,
}

impl PlanCacheKey {
    /// Returns a key built directly from precomputed request and plan content ids.
    pub fn new(request_content_id: ContentId, plan_content_id: ContentId) -> Self {
        Self {
            request_content_id,
            plan_content_id,
        }
    }

    /// Returns the key derived by hashing the request and plan expressions.
    pub fn for_request_plan(request: &Expr, plan: &Expr) -> Result<Self> {
        Ok(Self::new(
            request_expr_content_id(request)?,
            content_id_for_expr(plan)?,
        ))
    }

    /// Returns the content id of the request portion of the key.
    pub fn request_content_id(&self) -> &ContentId {
        &self.request_content_id
    }

    /// Returns the content id of the plan portion of the key.
    pub fn plan_content_id(&self) -> &ContentId {
        &self.plan_content_id
    }

    /// Returns the key as a map expression with request and plan content ids.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field(
                "request-content-id",
                content_id_expr(&self.request_content_id),
            ),
            field("plan-content-id", content_id_expr(&self.plan_content_id)),
        ])
    }
}

/// Controls whether a plan evaluation reads from and/or writes to the plan cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlanCacheMode {
    /// Bypasses the cache entirely: no reads and no writes.
    Disabled,
    /// Reads cached responses and writes fresh ones back (the default).
    #[default]
    ReadThrough,
    /// Reads cached responses but never writes new entries.
    ReadOnly,
    /// Writes fresh responses but never serves cached ones.
    WriteOnly,
    /// Ignores existing entries on read yet writes the recomputed response.
    Refresh,
}

impl PlanCacheMode {
    /// Parses a cache mode from a symbol, string, or `plan/atom` expression.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        match expr {
            Expr::String(name) => Self::from_name(name),
            Expr::Symbol(symbol) if symbol.namespace.is_none() => Self::from_name(&symbol.name),
            Expr::List(items) => match items.as_slice() {
                [Expr::Symbol(head), Expr::String(name)]
                    if head.namespace.is_none() && head.name.as_ref() == "plan/atom" =>
                {
                    Self::from_name(name)
                }
                _ => Err(Error::Eval(
                    "plan/cache mode must be a symbol, string, or atom".to_owned(),
                )),
            },
            _ => Err(Error::Eval(
                "plan/cache mode must be a symbol, string, or atom".to_owned(),
            )),
        }
    }

    /// Parses a cache mode from its textual name (e.g. `"read-through"`).
    pub fn from_name(name: &str) -> Result<Self> {
        match name {
            "off" | "none" | "disabled" => Ok(Self::Disabled),
            "read-through" | "read-write" | "default" => Ok(Self::ReadThrough),
            "read" | "read-only" => Ok(Self::ReadOnly),
            "write" | "write-only" => Ok(Self::WriteOnly),
            "refresh" => Ok(Self::Refresh),
            other => Err(Error::Eval(format!("unsupported plan/cache mode {other}"))),
        }
    }

    /// Returns `true` when the mode serves cached responses.
    pub fn reads(self) -> bool {
        matches!(self, Self::ReadThrough | Self::ReadOnly)
    }

    /// Returns `true` when the mode stores recomputed responses.
    pub fn writes(self) -> bool {
        matches!(self, Self::ReadThrough | Self::WriteOnly | Self::Refresh)
    }
}

/// Selects where a cache write lands and which capability it requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanCacheWriteTarget {
    /// Stores the entry in process memory only; requires no capability.
    Memory,
    /// Persists the entry beyond the process; requires the runner cache capability.
    Persistent,
}

/// In-memory store of plan evaluation responses keyed by [`PlanCacheKey`].
#[derive(Clone, Debug, Default)]
pub struct OpenAiPlanCache {
    entries: BTreeMap<PlanCacheKey, Expr>,
}

impl OpenAiPlanCache {
    /// Returns an empty plan cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the cached response for `key`, or `None` when absent.
    pub fn get(&self, key: &PlanCacheKey) -> Option<&Expr> {
        self.entries.get(key)
    }

    /// Inserts a response under `key`, checking the cache capability when persistent.
    ///
    /// Writes to [`PlanCacheWriteTarget::Persistent`] require the runner cache
    /// capability and fail closed if it is not granted; memory writes always
    /// succeed.
    pub fn put(
        &mut self,
        cx: &mut Cx,
        target: PlanCacheWriteTarget,
        key: PlanCacheKey,
        response: Expr,
    ) -> Result<()> {
        if target == PlanCacheWriteTarget::Persistent {
            cx.require(&ai_runner_cache_capability())?;
        }
        self.entries.insert(key, response);
        Ok(())
    }

    /// Returns the number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when the cache holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

use sim_value::build::entry as field;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_modes_report_read_and_write_behavior() {
        let cases = [
            ("disabled", false, false),
            ("read-through", true, true),
            ("read-only", true, false),
            ("write-only", false, true),
            ("refresh", false, true),
        ];

        for (name, reads, writes) in cases {
            let mode = PlanCacheMode::from_name(name).unwrap();
            assert_eq!(mode.reads(), reads, "{name} read behavior");
            assert_eq!(mode.writes(), writes, "{name} write behavior");
        }
    }
}
