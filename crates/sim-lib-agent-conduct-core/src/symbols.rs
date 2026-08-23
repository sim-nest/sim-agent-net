//! Reviewed standard symbols for the open conduct vocabulary.

use sim_kernel::Symbol;

fn in_namespace(value: &Symbol, namespace: &str) -> bool {
    value.namespace.as_deref() == Some(namespace)
}

macro_rules! vocabulary {
    ($module:ident, $namespace:literal, [$($name:ident => $text:literal),+ $(,)?]) => {
        /// Standard symbols and membership predicate for this open vocabulary.
        pub mod $module {
            use super::{Symbol, in_namespace};
            $(
                #[doc = concat!("Standard `", $namespace, "/", $text, "` symbol.")]
                #[allow(non_snake_case)]
                pub fn $name() -> Symbol { Symbol::qualified($namespace, $text) }
            )+
            /// Returns true for any symbol in the vocabulary namespace,
            /// including independently defined extensions.
            pub fn is_kind(value: &Symbol) -> bool { in_namespace(value, $namespace) }
        }
    };
}

vocabulary!(step, "agent.step", [MODEL_TURN => "model-turn", TOOL_BATCH => "tool-batch", REVIEW => "review", FINISH => "finish", STOP => "stop"]);
vocabulary!(role, "agent.role", [RUNNER => "runner", TOOLS => "tools", JUDGE => "judge"]);
vocabulary!(outcome, "agent.outcome", [CONTINUE => "continue", TOOL_CALLS => "tool-calls", FINAL => "final", ACCEPT => "accept", REVISE => "revise", REJECT => "reject", ERROR => "error"]);
vocabulary!(event, "agent.event", [RUN_STARTED => "run-started", STEP_COMPLETED => "step-completed", EFFECT_REQUESTED => "effect-requested", EFFECT_COMMITTED => "effect-committed", EFFECT_ABORTED => "effect-aborted", RUN_STOPPED => "run-stopped"]);
vocabulary!(usage, "agent.usage", [MODEL_TURN => "model-turn", TOOL_CALL => "tool-call", DELEGATION => "delegation", INPUT_TOKEN => "input-token", OUTPUT_TOKEN => "output-token"]);
vocabulary!(stop, "agent.stop", [COMPLETED => "completed", BUDGET_EXHAUSTED => "budget-exhausted", UNCERTAIN_EFFECT => "uncertain-effect", INVALID_FRAME => "invalid-frame", CORRUPT_JOURNAL => "corrupt-journal"]);

/// Creates the standard currency-qualified micro-unit dimension.
pub fn currency_micro_units(currency: &str) -> Symbol {
    Symbol::qualified(
        "agent.usage",
        format!("currency/{}/micro-unit", currency.to_ascii_uppercase()),
    )
}
