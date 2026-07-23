use sim_kernel::{Cx, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelCard, ModelRequest, ModelResponse, ModelRunner};

use crate::{LOCAL_MODEL_ID, LOCAL_MODEL_RUNNER, LOCAL_MODEL_SITE_KEY};

/// Local model backend exposed as a provider-neutral [`ModelRunner`].
#[derive(Clone, Debug)]
pub struct LocalModelBackend {
    runner: Symbol,
    model: String,
    placement_key: String,
}

impl LocalModelBackend {
    /// Builds the deterministic local backend used by the loadable site.
    pub fn new() -> Self {
        Self {
            runner: Symbol::new(LOCAL_MODEL_RUNNER),
            model: LOCAL_MODEL_ID.to_owned(),
            placement_key: LOCAL_MODEL_SITE_KEY.to_owned(),
        }
    }

    /// Returns the placement key this backend registers.
    pub fn placement_key(&self) -> &str {
        &self.placement_key
    }

    fn modeled_response(&self, request: ModelRequest) -> ModelResponse {
        let content = vec![Expr::Map(vec![
            key_expr("type", Expr::Symbol(Symbol::new("text"))),
            key_expr("text", modeled_text(&request)),
        ])];
        let mut response = ModelResponse::new(
            self.runner.clone(),
            self.model.clone(),
            content,
            Symbol::new("stop"),
        );
        response.extra.push(key_expr(
            "backend",
            Expr::Symbol(Symbol::new("modeled-local")),
        ));
        response
    }
}

impl Default for LocalModelBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelRunner for LocalModelBackend {
    fn card(&self) -> ModelCard {
        let mut card = ModelCard::new(
            self.runner.clone(),
            self.model.clone(),
            Symbol::new("local-model"),
            Symbol::new("local"),
        );
        card.extra.push(key_expr(
            "placement-key",
            Expr::String(self.placement_key.clone()),
        ));
        card.extra
            .push(key_expr("supports-stream", Expr::Bool(true)));
        card.extra
            .push(key_expr("supports-cache", Expr::Bool(true)));
        card
    }

    fn infer(&self, _cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        Ok(self.modeled_response(request))
    }
}

fn modeled_text(request: &ModelRequest) -> Expr {
    let task_text = match &request.task {
        Expr::String(text) if !text.is_empty() => text.as_str(),
        _ => "request",
    };
    Expr::String(format!("sim-local-modeled-ok: {task_text}"))
}

fn key_expr(key: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(key)), value)
}
