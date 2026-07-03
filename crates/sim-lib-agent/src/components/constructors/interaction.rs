use super::super::{
    model::{
        AgentComponent, ComponentBackend, JudgeBackend, PersonaBackend, PlannerBackend,
        RouterBackend, component_value, number_expr, number_expr_from_f64,
    },
    options::{
        expr_option, maybe_f64_option, maybe_u32_option, parse_component_options, string_option,
        symbol_option, symbols_option,
    },
};
use super::interaction_support::{
    default_glossary, default_translation_table, glossary_option, judge_config_from_options,
    judge_spec, option_stringish, pair_table_expr, style_word_limit,
};
use crate::{ComponentKind, util::installed_codecs};
use sim_kernel::{Args, Cx, Expr, Result, Symbol, Value};
use sim_lib_server::{ServerAddress, parse_duration};
use std::sync::{Arc, Mutex};

pub(crate) fn planner_budget_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "planner/budget")?;
    let max_turns = maybe_u32_option(cx, &options, "max-turns")?;
    let max_cost = maybe_f64_option(cx, &options, "max-cost")?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("planner", "budget"),
            kind: ComponentKind::Planner,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![
                (Symbol::new("strategy"), Expr::Symbol(Symbol::new("budget"))),
                (
                    Symbol::new("max-turns"),
                    max_turns.map(number_expr).unwrap_or(Expr::Nil),
                ),
                (
                    Symbol::new("max-cost"),
                    max_cost.map(number_expr_from_f64).unwrap_or(Expr::Nil),
                ),
            ],
            backend: ComponentBackend::Planner(PlannerBackend::Budget {
                max_turns,
                max_cost,
            }),
        },
    )
}

pub(crate) fn planner_refine_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let _ = parse_component_options(cx, args, "planner/refine")?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("planner", "refine"),
            kind: ComponentKind::Planner,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![(Symbol::new("strategy"), Expr::Symbol(Symbol::new("refine")))],
            backend: ComponentBackend::Planner(PlannerBackend::Refine),
        },
    )
}

pub(crate) fn planner_parallel_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "planner/parallel")?;
    let branches = maybe_u32_option(cx, &options, "branches")?.unwrap_or(2);
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("planner", "parallel"),
            kind: ComponentKind::Planner,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![
                (
                    Symbol::new("strategy"),
                    Expr::Symbol(Symbol::new("parallel")),
                ),
                (Symbol::new("branches"), number_expr(branches)),
            ],
            backend: ComponentBackend::Planner(PlannerBackend::Parallel { branches }),
        },
    )
}

pub(crate) fn planner_chain_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let _ = parse_component_options(cx, args, "planner/chain")?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("planner", "chain"),
            kind: ComponentKind::Planner,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![(Symbol::new("strategy"), Expr::Symbol(Symbol::new("chain")))],
            backend: ComponentBackend::Planner(PlannerBackend::Chain),
        },
    )
}

pub(crate) fn judge_rubric_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "judge/rubric")?;
    let config = judge_config_from_options(cx, &options)?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("judge", "rubric"),
            kind: ComponentKind::Judge,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: judge_spec(&config),
            backend: ComponentBackend::Judge(JudgeBackend::Rubric { config }),
        },
    )
}

pub(crate) fn judge_ranked_vote_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "judge/ranked-vote")?;
    let config = judge_config_from_options(cx, &options)?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("judge", "ranked-vote"),
            kind: ComponentKind::Judge,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: {
                let mut spec = vec![(
                    Symbol::new("mode"),
                    Expr::Symbol(Symbol::new("ranked-vote")),
                )];
                spec.extend(judge_spec(&config));
                spec
            },
            backend: ComponentBackend::Judge(JudgeBackend::RankedVote { config }),
        },
    )
}

pub(crate) fn judge_threshold_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "judge/threshold")?;
    let threshold = maybe_f64_option(cx, &options, "threshold")?.unwrap_or(0.5);
    let config = judge_config_from_options(cx, &options)?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("judge", "threshold"),
            kind: ComponentKind::Judge,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: {
                let mut spec = vec![(Symbol::new("threshold"), number_expr_from_f64(threshold))];
                spec.extend(judge_spec(&config));
                spec
            },
            backend: ComponentBackend::Judge(JudgeBackend::Threshold { threshold, config }),
        },
    )
}

pub(crate) fn router_round_robin_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "router/round-robin")?;
    let targets = symbols_option(cx, &options, "targets")?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("router", "round-robin"),
            kind: ComponentKind::Router,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![(
                Symbol::new("targets"),
                Expr::List(targets.iter().cloned().map(Expr::Symbol).collect()),
            )],
            backend: ComponentBackend::Router(RouterBackend::RoundRobin {
                targets,
                cursor: Arc::new(Mutex::new(0)),
            }),
        },
    )
}

pub(crate) fn router_bid_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "router/bid")?;
    let metric = symbol_option(cx, &options, "metric", Symbol::new("estimated-cost"))?;
    let targets = symbols_option(cx, &options, "targets")?;
    let auction_window = expr_option(cx, &options, "auction-window")?
        .map(|expr| parse_duration(&expr))
        .transpose()?
        .unwrap_or_else(|| std::time::Duration::from_millis(200));
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("router", "bid"),
            kind: ComponentKind::Router,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![
                (
                    Symbol::new("targets"),
                    Expr::List(targets.iter().cloned().map(Expr::Symbol).collect()),
                ),
                (Symbol::new("metric"), Expr::Symbol(metric.clone())),
                (
                    Symbol::new("auction-window"),
                    Expr::String(format!("{}ms", auction_window.as_millis())),
                ),
            ],
            backend: ComponentBackend::Router(RouterBackend::Bid {
                targets,
                metric,
                auction_window,
            }),
        },
    )
}

pub(crate) fn router_sticky_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "router/sticky")?;
    let targets = symbols_option(cx, &options, "targets")?;
    let sticky_key = symbol_option(cx, &options, "sticky-key", Symbol::new("session"))?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("router", "sticky"),
            kind: ComponentKind::Router,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![
                (
                    Symbol::new("targets"),
                    Expr::List(targets.iter().cloned().map(Expr::Symbol).collect()),
                ),
                (Symbol::new("sticky-key"), Expr::Symbol(sticky_key.clone())),
            ],
            backend: ComponentBackend::Router(RouterBackend::Sticky {
                targets,
                sticky_key,
            }),
        },
    )
}

pub(crate) fn persona_style_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "persona/style")?;
    let voice = string_option(cx, &options, "voice", "terse")?;
    let prefix = string_option(cx, &options, "prefix", "")?;
    let suffix = string_option(cx, &options, "suffix", "")?;
    let max_words = style_word_limit(&voice);
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("persona", "style"),
            kind: ComponentKind::Persona,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![
                (Symbol::new("voice"), Expr::String(voice.clone())),
                (Symbol::new("prefix"), Expr::String(prefix.clone())),
                (Symbol::new("suffix"), Expr::String(suffix.clone())),
                (Symbol::new("max-words"), number_expr(max_words as u32)),
            ],
            backend: ComponentBackend::Persona(PersonaBackend::Style {
                voice,
                prefix,
                suffix,
                max_words,
            }),
        },
    )
}

pub(crate) fn persona_language_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "persona/language")?;
    let language = string_option(cx, &options, "language", "en")?;
    let glossary =
        glossary_option(cx, &options, "glossary")?.unwrap_or_else(|| default_glossary(&language));
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("persona", "language"),
            kind: ComponentKind::Persona,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![
                (Symbol::new("language"), Expr::String(language.clone())),
                (Symbol::new("glossary"), pair_table_expr(&glossary)),
            ],
            backend: ComponentBackend::Persona(PersonaBackend::Language { language, glossary }),
        },
    )
}

pub(crate) fn persona_translator_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "persona/translator")?;
    let from = string_option(cx, &options, "from", "en")?;
    let to = string_option(cx, &options, "to", "sv")?;
    let table = glossary_option(cx, &options, "table")?
        .unwrap_or_else(|| default_translation_table(&from, &to));
    let command = option_stringish(cx, &options, "command")?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("persona", "translator"),
            kind: ComponentKind::Persona,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![
                (Symbol::new("from"), Expr::String(from.clone())),
                (Symbol::new("to"), Expr::String(to.clone())),
                (Symbol::new("table"), pair_table_expr(&table)),
                (
                    Symbol::new("command"),
                    command.clone().map(Expr::String).unwrap_or(Expr::Nil),
                ),
            ],
            backend: ComponentBackend::Persona(PersonaBackend::Translator {
                from,
                to,
                table,
                command,
            }),
        },
    )
}
