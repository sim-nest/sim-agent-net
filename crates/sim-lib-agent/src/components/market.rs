use super::market_cards::{health_expr, runner_card, runner_name_expr};
use super::market_execution::execute_market;
use super::market_policy::{MarketPolicy, key_expr};
use super::model::{AgentComponent, ComponentBackend, RunnerBackend, component_value};
use super::options::parse_component_options;
use crate::model_privacy::PrivacyPolicy;
use crate::{ComponentKind, installed_codecs, value_from_expr};
use sim_kernel::{Args, Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_agent_runner_core::{ModelBid, ModelCard, ModelRequest, ModelResponse, ModelRunner};
use sim_lib_server::ServerAddress;
use std::{collections::HashMap, sync::Arc};

pub(crate) fn model_policy_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "model-policy")?;
    let policy = MarketPolicy::from_options(cx, &options)?;
    value_from_expr(cx, &policy.to_expr())
}

pub(crate) fn runner_market_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "runner/market")?;
    let symbol = crate::symbol_option(cx, &options, "name", Symbol::qualified("runner", "market"))?;
    let model = crate::string_option(cx, &options, "model", "runner/market")?;
    let runners = values_option(cx, &options, "runners")?;
    if runners.is_empty() {
        return Err(Error::Eval(
            "runner/market :runners expects at least one runner".to_owned(),
        ));
    }
    let policy = match options.get("policy") {
        Some(value) => MarketPolicy::from_expr(&value.object().as_expr(cx)?)?,
        None => MarketPolicy::from_options(cx, &options)?,
    };
    let mut spec = vec![
        (Symbol::new("backend"), Expr::Symbol(Symbol::new("market"))),
        (Symbol::new("model"), Expr::String(model.clone())),
        (Symbol::new("policy"), policy.to_expr()),
        (
            Symbol::new("runners"),
            Expr::List(runners.iter().map(runner_name_expr).collect()),
        ),
    ];
    if let Some(fallback) = &policy.fallback {
        spec.push((Symbol::new("fallback"), Expr::Symbol(fallback.clone())));
    }
    component_value(
        cx,
        AgentComponent {
            symbol: symbol.clone(),
            kind: ComponentKind::Runner,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec,
            backend: ComponentBackend::Runner(RunnerBackend::External {
                runner: Arc::new(MarketRunner {
                    symbol,
                    model,
                    runners,
                    policy,
                }),
            }),
        },
    )
}

pub(crate) fn runner_card_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [runner] = args.values() else {
        return Err(Error::Eval("runner/card expects one runner".to_owned()));
    };
    let expr: Expr = runner_card(cx, runner)?.into();
    value_from_expr(cx, &expr)
}

pub(crate) fn runner_cards_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let runners = if let [one] = args.values() {
        values_from_value(cx, one)?
    } else {
        args.values().to_vec()
    };
    let cards = runners
        .iter()
        .map(|runner| runner_card(cx, runner).map(Expr::from))
        .collect::<Result<Vec<_>>>()?;
    value_from_expr(cx, &Expr::List(cards))
}

pub(crate) fn runner_health_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [runner] = args.values() else {
        return Err(Error::Eval("runner/health expects one runner".to_owned()));
    };
    let card = runner_card(cx, runner)?;
    value_from_expr(cx, &health_expr(&card))
}

#[derive(Clone)]
struct MarketRunner {
    symbol: Symbol,
    model: String,
    runners: Vec<Value>,
    policy: MarketPolicy,
}

impl ModelRunner for MarketRunner {
    fn card(&self) -> ModelCard {
        let mut card = ModelCard::new(
            self.symbol.clone(),
            self.model.clone(),
            Symbol::new("market"),
            Symbol::new("fabric"),
        );
        card.extra.push(key_expr("policy", self.policy.to_expr()));
        card.extra.push((
            Expr::Symbol(Symbol::new("runners")),
            Expr::List(self.runners.iter().map(runner_name_expr).collect()),
        ));
        card
    }

    fn infer(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        let privacy = PrivacyPolicy::from_model_request(&request)?;
        privacy.ensure_no_raw_refs(&request.clone().into())?;
        execute_market(cx, &self.runners, self.policy.clone(), request, privacy)
    }

    fn bid(&self, _request: &ModelRequest) -> Result<ModelBid> {
        Ok(ModelBid {
            available: true,
            reason: None,
            score: Some(0.0),
            model: Some(self.model.clone()),
            extra: vec![key_expr("policy", self.policy.to_expr())],
        })
    }
}

fn values_option(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
    key: &'static str,
) -> Result<Vec<Value>> {
    options
        .get(key)
        .map(|value| values_from_value(cx, value))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn values_from_value(cx: &mut Cx, value: &Value) -> Result<Vec<Value>> {
    if let Some(list) = value.object().as_list() {
        return list.to_vec(cx, Some(1024));
    }
    if matches!(value.object().as_expr(cx)?, Expr::Nil) {
        Ok(Vec::new())
    } else {
        Ok(vec![value.clone()])
    }
}
