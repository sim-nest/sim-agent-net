use super::market_cards::{runner_name_expr, runner_symbol};
use super::market_policy::key_expr;
use super::model::{AgentComponent, ComponentBackend, RunnerBackend, component_value};
use super::options::{parse_component_options, string_option, symbol_option};
use crate::agents::{ensure_task_id, first_codec, site_from_value, with_task_id};
use crate::{Agent, AgentFabric, ComponentKind, installed_codecs};
use sim_kernel::{
    Args, Consistency, Cx, Error, EvalMode, EvalRequest, Expr, Result, Symbol, Value,
};
use sim_lib_agent_runner_core::{ModelBid, ModelCard, ModelRequest, ModelResponse, ModelRunner};
use sim_lib_server::{
    EvalSite, ServerAddress, eval_reply_from_frame, parse_duration, server_frame_from_request,
};
use std::{collections::HashMap, sync::Arc, time::Duration};

pub(crate) fn runner_agent_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "runner/agent")?;
    let symbol = symbol_option(cx, &options, "name", Symbol::qualified("runner", "agent"))?;
    let target = options
        .get("agent")
        .cloned()
        .ok_or_else(|| Error::Eval("runner/agent requires :agent".to_owned()))?;
    let target_label = target_label(cx, &target)?;
    let model = string_option(cx, &options, "model", &format!("sim-agent/{target_label}"))?;
    let timeout = timeout_option(cx, &options)?;
    let site = site_from_value(&target)?;
    let mut spec = vec![
        (Symbol::new("backend"), Expr::Symbol(Symbol::new("agent"))),
        (Symbol::new("model"), Expr::String(model.clone())),
        (Symbol::new("agent"), Expr::String(target_label.clone())),
    ];
    if let Some(timeout) = timeout {
        spec.push((
            Symbol::new("timeout"),
            Expr::String(format!("{}ms", timeout.as_millis())),
        ));
    }
    spec.extend(runner_metadata_spec(cx, &options)?);
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
                runner: Arc::new(AgentModelRunner {
                    symbol,
                    model,
                    site,
                    target_label,
                    timeout,
                }),
            }),
        },
    )
}

pub(crate) fn runner_debate_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "runner/debate")?;
    let symbol = symbol_option(cx, &options, "name", Symbol::qualified("runner", "debate"))?;
    let model = string_option(cx, &options, "model", "runner/debate")?;
    let runners = values_option(cx, &options, "runners")?;
    if runners.is_empty() {
        return Err(Error::Eval(
            "runner/debate :runners expects at least one runner".to_owned(),
        ));
    }
    let judge = options
        .get("judge")
        .cloned()
        .ok_or_else(|| Error::Eval("runner/debate requires :judge".to_owned()))?;
    let timeout = timeout_option(cx, &options)?;
    let mut spec = vec![
        (Symbol::new("backend"), Expr::Symbol(Symbol::new("debate"))),
        (Symbol::new("model"), Expr::String(model.clone())),
        (
            Symbol::new("runners"),
            Expr::List(runners.iter().map(runner_name_expr).collect()),
        ),
        (Symbol::new("judge"), runner_name_expr(&judge)),
    ];
    if let Some(timeout) = timeout {
        spec.push((
            Symbol::new("timeout"),
            Expr::String(format!("{}ms", timeout.as_millis())),
        ));
    }
    spec.extend(runner_metadata_spec(cx, &options)?);
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
                runner: Arc::new(DebateRunner {
                    symbol,
                    model,
                    runners,
                    judge,
                    timeout,
                }),
            }),
        },
    )
}

#[derive(Clone)]
struct AgentModelRunner {
    symbol: Symbol,
    model: String,
    site: Arc<dyn EvalSite>,
    target_label: String,
    timeout: Option<Duration>,
}

impl ModelRunner for AgentModelRunner {
    fn card(&self) -> ModelCard {
        let mut card = ModelCard::new(
            self.symbol.clone(),
            self.model.clone(),
            Symbol::new("agent"),
            Symbol::new("agent"),
        );
        card.extra
            .push(key_expr("agent", Expr::String(self.target_label.clone())));
        if let Some(timeout) = self.timeout {
            card.extra.push(key_expr(
                "timeout",
                Expr::String(format!("{}ms", timeout.as_millis())),
            ));
        }
        card
    }

    fn infer(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        let expr = Expr::from(request);
        let mut frame = server_frame_from_request(
            cx,
            &first_codec(self.site.codecs()),
            model_eval_request(expr, self.timeout, true),
        )?;
        let task_id = ensure_task_id(&mut frame);
        let reply = with_task_id(task_id.clone(), || self.site.answer(cx, frame))?;
        let reply_expr = eval_reply_from_frame(cx, &reply)?
            .value
            .object()
            .as_expr(cx)?;
        Ok(self.normalize_reply(reply_expr, task_id))
    }

    fn bid(&self, _request: &ModelRequest) -> Result<ModelBid> {
        Ok(ModelBid {
            available: true,
            reason: None,
            score: Some(0.0),
            model: Some(self.model.clone()),
            extra: vec![key_expr("agent", Expr::String(self.target_label.clone()))],
        })
    }
}

impl AgentModelRunner {
    fn normalize_reply(&self, expr: Expr, task_id: String) -> ModelResponse {
        let mut response = ModelResponse::try_from(expr.clone()).unwrap_or_else(|_| {
            ModelResponse::new(
                self.symbol.clone(),
                self.model.clone(),
                vec![text_content(expr_text(&expr))],
                Symbol::new("stop"),
            )
        });
        if response.runner != self.symbol {
            response.extra.push(key_expr(
                "agent-inner-runner",
                Expr::Symbol(response.runner.clone()),
            ));
            response.runner = self.symbol.clone();
        }
        if response.model != self.model {
            response.extra.push(key_expr(
                "agent-inner-model",
                Expr::String(response.model.clone()),
            ));
            response.model = self.model.clone();
        }
        response
            .extra
            .push(key_expr("agent-task-id", Expr::String(task_id)));
        response.extra.push(key_expr(
            "agent-target",
            Expr::String(self.target_label.clone()),
        ));
        response
    }
}

#[derive(Clone)]
struct DebateRunner {
    symbol: Symbol,
    model: String,
    runners: Vec<Value>,
    judge: Value,
    timeout: Option<Duration>,
}

impl ModelRunner for DebateRunner {
    fn card(&self) -> ModelCard {
        let mut card = ModelCard::new(
            self.symbol.clone(),
            self.model.clone(),
            Symbol::new("debate"),
            Symbol::new("fabric"),
        );
        card.extra.push((
            Expr::Symbol(Symbol::new("runners")),
            Expr::List(self.runners.iter().map(runner_name_expr).collect()),
        ));
        card.extra
            .push(key_expr("judge", runner_name_expr(&self.judge)));
        if let Some(timeout) = self.timeout {
            card.extra.push(key_expr(
                "timeout",
                Expr::String(format!("{}ms", timeout.as_millis())),
            ));
        }
        card
    }

    fn infer(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        let answers = self
            .runners
            .iter()
            .map(|runner| realize_runner(cx, runner, &request, self.timeout))
            .collect::<Result<Vec<_>>>()?;
        let judge_request = debate_judge_request(&request, &answers);
        let mut response = realize_runner(cx, &self.judge, &judge_request, self.timeout)?;
        response.extra.push(key_expr(
            "debate-answers",
            Expr::List(answers.iter().cloned().map(Expr::from).collect()),
        ));
        response.extra.push((
            Expr::Symbol(Symbol::new("debate-runners")),
            Expr::List(self.runners.iter().map(runner_name_expr).collect()),
        ));
        if let Some(judge) = runner_symbol(&self.judge) {
            response
                .extra
                .push(key_expr("debate-judge", Expr::Symbol(judge)));
        }
        if response.runner != self.symbol {
            response.extra.push(key_expr(
                "debate-inner-runner",
                Expr::Symbol(response.runner.clone()),
            ));
            response.runner = self.symbol.clone();
        }
        if response.model != self.model {
            response.extra.push(key_expr(
                "debate-inner-model",
                Expr::String(response.model.clone()),
            ));
            response.model = self.model.clone();
        }
        Ok(response)
    }

    fn bid(&self, _request: &ModelRequest) -> Result<ModelBid> {
        Ok(ModelBid {
            available: true,
            reason: None,
            score: Some(0.0),
            model: Some(self.model.clone()),
            extra: Vec::new(),
        })
    }
}

fn realize_runner(
    cx: &mut Cx,
    runner: &Value,
    request: &ModelRequest,
    timeout: Option<Duration>,
) -> Result<ModelResponse> {
    let fabric = runner
        .object()
        .as_eval_fabric()
        .ok_or_else(|| Error::Eval("debate runner is not a realize target".to_owned()))?;
    let reply = fabric.realize(
        cx,
        model_eval_request(Expr::from(request.clone()), timeout, false),
    )?;
    ModelResponse::try_from(reply.value.object().as_expr(cx)?)
}

fn debate_judge_request(request: &ModelRequest, answers: &[ModelResponse]) -> ModelRequest {
    ModelRequest {
        task: Expr::Map(vec![
            (Expr::Symbol(Symbol::new("debate")), Expr::Bool(true)),
            (Expr::Symbol(Symbol::new("task")), request.task.clone()),
            (
                Expr::Symbol(Symbol::new("answers")),
                Expr::List(answers.iter().cloned().map(Expr::from).collect()),
            ),
        ]),
        messages: request.messages.clone(),
        extra: request.extra.clone(),
    }
}

fn model_eval_request(expr: Expr, timeout: Option<Duration>, trace: bool) -> EvalRequest {
    EvalRequest {
        expr,
        mode: EvalMode::Eval,
        result_shape: None,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        required_capabilities: Vec::new(),
        deadline: timeout,
        consistency: Consistency::LocalFirst,
        trace,
    }
}

fn values_option(cx: &mut Cx, options: &HashMap<String, Value>, key: &str) -> Result<Vec<Value>> {
    let Some(value) = options.get(key) else {
        return Ok(Vec::new());
    };
    if let Some(list) = value.object().as_list() {
        return list.to_vec(cx, None);
    }
    match value.object().as_expr(cx)? {
        Expr::Nil => Ok(Vec::new()),
        Expr::List(items) | Expr::Vector(items) => {
            items.into_iter().map(|expr| cx.eval_expr(expr)).collect()
        }
        _ => Ok(vec![value.clone()]),
    }
}

fn runner_metadata_spec(
    cx: &mut Cx,
    options: &HashMap<String, Value>,
) -> Result<Vec<(Symbol, Expr)>> {
    [
        "cost-usd",
        "context-tokens",
        "healthy",
        "health",
        "health-reason",
        "requires",
        "modalities",
        "modalities-in",
        "modalities-out",
        "privacy",
        "quality",
        "supports-stream",
        "supports-tools",
        "supports-json",
        "supports-shape",
    ]
    .into_iter()
    .filter_map(|key| {
        options.get(key).map(|value| {
            value
                .object()
                .as_expr(cx)
                .map(|expr| (Symbol::new(key), expr))
        })
    })
    .collect()
}

fn timeout_option(cx: &mut Cx, options: &HashMap<String, Value>) -> Result<Option<Duration>> {
    options
        .get("timeout")
        .map(|value| parse_duration(&value.object().as_expr(cx)?))
        .transpose()
}

fn target_label(cx: &mut Cx, value: &Value) -> Result<String> {
    if let Some(agent) = value.object().downcast_ref::<Agent>() {
        return Ok(agent.name.to_string());
    }
    if let Some(fabric) = value.object().downcast_ref::<AgentFabric>() {
        return Ok(fabric.name.to_string());
    }
    if let Some(component) = value.object().downcast_ref::<AgentComponent>() {
        return Ok(component.symbol.to_string());
    }
    value.object().display(cx)
}

fn text_content(text: String) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("type")),
            Expr::Symbol(Symbol::new("text")),
        ),
        (Expr::Symbol(Symbol::new("text")), Expr::String(text)),
    ])
}

fn expr_text(expr: &Expr) -> String {
    match expr {
        Expr::String(text) => text.clone(),
        Expr::Symbol(symbol) => symbol.to_string(),
        _ => format!("{expr:?}"),
    }
}
