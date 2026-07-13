//! Plan subsystem: parsing, checking, and evaluating SIM plan expressions that
//! orchestrate model atoms through combinators such as race, fallback, chain,
//! verify, debate, and cache.

/// Atom address resolution into backend descriptors.
pub mod address;
/// The plan combinator table and its metadata.
pub mod combinators;
/// Plan evaluation against a model request, producing a response and event trace.
pub mod eval;
pub(crate) mod eval_context;
mod eval_helpers;
pub(crate) mod fixtures;
/// Parser turning the textual plan surface syntax into plan expressions.
pub mod parse;
/// Structural plan validation, limits, and explanation.
pub mod shape;

pub use address::{BackendDescriptor, provider_prefixes, resolve_atom_address};
pub use combinators::{PlanCombinator, plan_combinators, plan_combinators_expr, plan_symbol};
pub use eval::{
    PlanEvalEvent, PlanEvalReport, eval_plan, eval_plan_report, eval_plan_report_with_cache,
    eval_plan_report_with_cache_and_runners, eval_plan_report_with_cache_runners_and_federation,
    eval_plan_report_with_federation,
};
pub use parse::parse_plan;
pub use shape::{PlanLimits, check_plan, check_plan_with_limits, explain_plan};
