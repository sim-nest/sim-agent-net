use super::super::model::AgentComponent;
use sim_codec_chat::{model_error_expr, model_response_expr, validate_chat_transcript};
use sim_kernel::{Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelEvent, ModelEventSink, ModelResponse};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

pub(super) fn fake_response_expr(
    component: &AgentComponent,
    model: &str,
    script: &Arc<Mutex<VecDeque<Expr>>>,
    delay: Duration,
) -> Result<Expr> {
    apply_delay(delay);
    let next = script
        .lock()
        .map_err(|_| Error::PoisonedLock("runner fake script"))?
        .pop_front();
    Ok(match next {
        Some(expr) => normalize_scripted_response(component, model, expr)?,
        None => model_error_expr(component.symbol.clone(), model, "no scripted response"),
    })
}

pub(super) fn fake_stream_response(
    component: &AgentComponent,
    model: &str,
    script: &Arc<Mutex<VecDeque<Expr>>>,
    delay: Duration,
    sink: &mut dyn ModelEventSink,
) -> Result<ModelResponse> {
    apply_delay(delay);
    let next = script
        .lock()
        .map_err(|_| Error::PoisonedLock("runner fake script"))?
        .pop_front();
    let expr = match next {
        Some(expr) => expr,
        None => model_error_expr(component.symbol.clone(), model, "no scripted response"),
    };
    stream_scripted_response(component, model, expr, sink)
}

fn stream_scripted_response(
    component: &AgentComponent,
    model: &str,
    expr: Expr,
    sink: &mut dyn ModelEventSink,
) -> Result<ModelResponse> {
    if let Expr::List(items) = &expr {
        let mut final_response = None;
        for item in items {
            let event = ModelEvent::try_from(item.clone())?;
            if event.event == Symbol::new("final") {
                final_response = event.response.clone();
            }
            sink.emit(event)?;
        }
        return final_response.ok_or_else(|| {
            Error::Eval("runner/fake streaming script must include a final event".to_owned())
        });
    }
    if let Ok(event) = ModelEvent::try_from(expr.clone()) {
        let final_response = if event.event == Symbol::new("final") {
            event.response.clone()
        } else {
            None
        };
        sink.emit(event)?;
        return final_response.ok_or_else(|| {
            Error::Eval("runner/fake streaming event must be a final event".to_owned())
        });
    }
    if validate_chat_transcript(&expr).is_ok() {
        let response = ModelResponse::try_from(expr)?;
        sink.emit(ModelEvent::final_of(&response))?;
        return Ok(response);
    }
    let normalized = normalize_scripted_response(component, model, expr)?;
    let response = ModelResponse::try_from(normalized)?;
    sink.emit(ModelEvent::final_of(&response))?;
    Ok(response)
}

fn normalize_scripted_response(
    component: &AgentComponent,
    model: &str,
    expr: Expr,
) -> Result<Expr> {
    if validate_chat_transcript(&expr).is_ok() {
        return Ok(expr);
    }
    text_response_expr(component, model, expr_text(&expr))
}

fn text_response_expr(component: &AgentComponent, model: &str, text: String) -> Result<Expr> {
    let part = Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("type")),
            Expr::Symbol(Symbol::new("text")),
        ),
        (Expr::Symbol(Symbol::new("text")), Expr::String(text)),
    ]);
    Ok(model_response_expr(
        component.symbol.clone(),
        model,
        vec![part],
        Symbol::new("stop"),
    ))
}

fn apply_delay(delay: Duration) {
    if !delay.is_zero() {
        thread::sleep(delay);
    }
}

fn expr_text(expr: &Expr) -> String {
    match expr {
        Expr::Nil => "nil".to_owned(),
        Expr::Bool(value) => value.to_string(),
        Expr::Symbol(symbol) => symbol.to_string(),
        Expr::String(text) => text.clone(),
        _ => format!("{expr:?}"),
    }
}
