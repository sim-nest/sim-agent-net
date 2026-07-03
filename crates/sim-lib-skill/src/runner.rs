use std::sync::Arc;

use sim_kernel::{Args, Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_agent::{
    ExternalRunnerSpec, ModelBid, ModelCard, ModelEvent, ModelEventSink, ModelRequest,
    ModelResponse, ModelRunner,
};

use crate::registry::card_from_value;
use crate::{SkillCallable, SkillCard, SkillEventSink, SkillRole};

/// Model runner that drives a model-role skill as an agent runner.
///
/// Wraps a [`SkillCallable`] for a card with the [`SkillRole::Model`] role and
/// adapts the agent runner protocol (card, infer, streaming infer, and bid) to
/// skill calls, translating model requests and responses to and from skill
/// values.
#[derive(Clone)]
pub struct SkillModelRunner {
    id: String,
    symbol: Symbol,
    model: String,
    callable: SkillCallable,
}

impl SkillModelRunner {
    fn normalize_response(&self, response: &mut ModelResponse) {
        response.runner = self.symbol.clone();
        response.model = self.model.clone();
    }

    fn response_from_value(&self, cx: &mut Cx, value: Value) -> Result<ModelResponse> {
        let expr = value.object().as_expr(cx)?;
        if let Ok(mut response) = ModelResponse::try_from(expr.clone()) {
            self.normalize_response(&mut response);
            return Ok(response);
        }
        Ok(ModelResponse::new(
            self.symbol.clone(),
            self.model.clone(),
            vec![text_part(expr_text(&expr))],
            Symbol::new("stop"),
        ))
    }
}

impl ModelRunner for SkillModelRunner {
    fn card(&self) -> ModelCard {
        let mut card = ModelCard::new(
            self.symbol.clone(),
            self.model.clone(),
            Symbol::new("skill"),
            Symbol::new("local"),
        );
        card.extra.extend([
            key_expr("skill-id", Expr::String(self.id.clone())),
            key_expr("skill-symbol", Expr::Symbol(self.symbol.clone())),
            key_expr("supports-stream", Expr::Bool(true)),
            key_expr("supports-shape", Expr::Bool(true)),
        ]);
        card
    }

    fn infer(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        let request = request_value(cx, request)?;
        let value = self.callable.call_values(cx, vec![request], None)?;
        self.response_from_value(cx, value)
    }

    fn infer_stream(
        &self,
        cx: &mut Cx,
        request: ModelRequest,
        sink: &mut dyn ModelEventSink,
    ) -> Result<ModelResponse> {
        let request = request_value(cx, request)?;
        let (value, saw_final) = {
            let mut events =
                SkillModelEventSink::new(sink, self.symbol.clone(), self.model.clone());
            let value = self
                .callable
                .call_values(cx, vec![request], Some(&mut events))?;
            (value, events.saw_final)
        };
        let response = self.response_from_value(cx, value)?;
        if !saw_final {
            sink.emit(ModelEvent::final_of(&response))?;
        }
        Ok(response)
    }

    fn bid(&self, _request: &ModelRequest) -> Result<ModelBid> {
        Ok(ModelBid {
            available: true,
            reason: None,
            score: Some(0.0),
            model: Some(self.model.clone()),
            extra: vec![key_expr("skill-id", Expr::String(self.id.clone()))],
        })
    }
}

struct SkillModelEventSink<'a> {
    sink: &'a mut dyn ModelEventSink,
    runner: Symbol,
    model: String,
    saw_final: bool,
}

impl<'a> SkillModelEventSink<'a> {
    fn new(sink: &'a mut dyn ModelEventSink, runner: Symbol, model: String) -> Self {
        Self {
            sink,
            runner,
            model,
            saw_final: false,
        }
    }

    fn normalize_event(&self, event: &mut ModelEvent) {
        event.runner = self.runner.clone();
        event.model = self.model.clone();
        if let Some(response) = &mut event.response {
            response.runner = self.runner.clone();
            response.model = self.model.clone();
        }
    }
}

impl SkillEventSink for SkillModelEventSink<'_> {
    fn emit(&mut self, cx: &mut Cx, event: Value) -> Result<()> {
        let expr = event.object().as_expr(cx)?;
        let mut event = match ModelEvent::try_from(expr.clone()) {
            Ok(event) => event,
            Err(_) => ModelEvent::new(
                Symbol::qualified("skill", "event"),
                self.runner.clone(),
                self.model.clone(),
                Expr::Symbol(Symbol::new("skill")),
            )
            .with_field("value", expr),
        };
        self.normalize_event(&mut event);
        if event.event == Symbol::new("final") {
            self.saw_final = true;
        }
        self.sink.emit(event)
    }
}

pub(crate) fn as_runner(cx: &mut Cx, args: Args) -> Result<Value> {
    let card = card_arg(cx, args)?;
    sim_lib_agent::install_agent_lib(cx)?;
    let runner = skill_model_runner(cx, &card)?;
    let id = card.id.clone();
    let symbol = card.symbol.clone();
    let model = model_id(&card);
    sim_lib_agent::external_runner_value(
        cx,
        ExternalRunnerSpec {
            symbol: symbol.clone(),
            model,
            capabilities: Vec::new(),
            spec: vec![
                (Symbol::new("skill-id"), Expr::String(id)),
                (Symbol::new("skill-symbol"), Expr::Symbol(symbol)),
                (Symbol::new("supports-stream"), Expr::Bool(true)),
                (Symbol::new("supports-shape"), Expr::Bool(true)),
            ],
            runner: Arc::new(runner),
        },
    )
}

/// Builds a [`SkillModelRunner`] for `card`.
///
/// Errors if the card lacks the [`SkillRole::Model`] role or is not bound to a
/// [`SkillCallable`].
pub fn skill_model_runner(cx: &mut Cx, card: &SkillCard) -> Result<SkillModelRunner> {
    ensure_model_role(card)?;
    let function = cx.resolve_function(&card.symbol)?;
    let callable = function
        .object()
        .downcast_ref::<SkillCallable>()
        .cloned()
        .ok_or_else(|| Error::Eval(format!("skill {} is not bound to a SkillCallable", card.id)))?;
    Ok(SkillModelRunner {
        id: card.id.clone(),
        symbol: card.symbol.clone(),
        model: model_id(card),
        callable,
    })
}

/// Returns the symbol for the `skill/as-runner` operation.
pub fn skill_as_runner_symbol() -> Symbol {
    Symbol::qualified("skill", "as-runner")
}

fn card_arg(cx: &mut Cx, args: Args) -> Result<SkillCard> {
    let mut values = args.into_vec();
    if values.len() != 1 {
        return Err(Error::Eval(
            "skill/as-runner expects one skill id, symbol, or SkillCard value".to_owned(),
        ));
    }
    card_from_target(cx, values.remove(0))
}

fn card_from_target(cx: &mut Cx, value: Value) -> Result<SkillCard> {
    if value.object().downcast_ref::<SkillCard>().is_some() {
        return card_from_value(cx, &value);
    }
    match value.object().as_expr(cx)? {
        Expr::Map(_) => card_from_value(cx, &value),
        Expr::String(id) => crate::skill_registry(cx)?
            .card_by_id(&id)?
            .ok_or_else(|| Error::Eval(format!("unknown skill {id}"))),
        Expr::Symbol(symbol) => crate::skill_registry(cx)?
            .card_by_symbol(&symbol)?
            .ok_or_else(|| Error::Eval(format!("unknown skill {symbol}"))),
        _ => Err(Error::TypeMismatch {
            expected: "skill id, symbol, or SkillCard",
            found: "invalid target",
        }),
    }
}

fn ensure_model_role(card: &SkillCard) -> Result<()> {
    if card.roles.contains(&SkillRole::Model) {
        Ok(())
    } else {
        Err(Error::Eval(format!(
            "skill {} is not marked with model role",
            card.id
        )))
    }
}

fn request_value(cx: &mut Cx, request: ModelRequest) -> Result<Value> {
    cx.factory().expr(Expr::from(request))
}

fn model_id(card: &SkillCard) -> String {
    card.symbol.as_qualified_str()
}

// Canonical shape lives in `sim_codec_chat::text_part`. Kept local here to avoid
// coupling `sim-lib-skill` to the chat codec for one trivial content-part builder.
fn text_part(text: String) -> Expr {
    Expr::Map(vec![
        key_expr("type", Expr::Symbol(Symbol::new("text"))),
        key_expr("text", Expr::String(text)),
    ])
}

fn expr_text(expr: &Expr) -> String {
    match expr {
        Expr::Nil => String::new(),
        Expr::Bool(value) => value.to_string(),
        Expr::Number(number) => number.canonical.clone(),
        Expr::String(value) => value.clone(),
        Expr::Symbol(symbol) | Expr::Local(symbol) => symbol.as_qualified_str(),
        other => format!("{other:?}"),
    }
}

fn key_expr(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}

#[cfg(test)]
mod tests;
