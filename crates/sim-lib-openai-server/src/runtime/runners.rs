use std::{fmt, sync::Arc};

use sim_kernel::{Cx, Error, Result};
use sim_lib_agent_runner_core::{ModelCard, ModelRequest, ModelResponse, ModelRunner};

/// Route-local registry of model runners exposed through OpenAI model ids.
#[derive(Clone, Default)]
pub struct OpenAiRunnerRegistry {
    entries: Vec<OpenAiRunnerEntry>,
}

#[derive(Clone)]
struct OpenAiRunnerEntry {
    model: String,
    runner: Arc<dyn ModelRunner>,
}

impl fmt::Debug for OpenAiRunnerRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiRunnerRegistry")
            .field(
                "models",
                &self
                    .entries
                    .iter()
                    .map(|entry| entry.model.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl OpenAiRunnerRegistry {
    /// Returns an empty runner registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the registry with `runner` registered under OpenAI model id `model`.
    pub fn with_runner(mut self, model: impl Into<String>, runner: Arc<dyn ModelRunner>) -> Self {
        self.register(model, runner);
        self
    }

    /// Registers `runner` under OpenAI model id `model`.
    pub fn register(&mut self, model: impl Into<String>, runner: Arc<dyn ModelRunner>) {
        self.entries.push(OpenAiRunnerEntry {
            model: model.into(),
            runner,
        });
    }

    fn entry_for(&self, model: &str) -> Option<&OpenAiRunnerEntry> {
        self.entries.iter().find(|entry| entry.model == model)
    }

    fn card_for_entry(entry: &OpenAiRunnerEntry) -> ModelCard {
        let mut card = entry.runner.card();
        if card.model != entry.model {
            card.extra.push((
                sim_kernel::Expr::Symbol(sim_kernel::Symbol::new("runner-model")),
                sim_kernel::Expr::String(card.model.clone()),
            ));
            card.model = entry.model.clone();
        }
        card
    }

    /// Returns the model card for each registered runner.
    ///
    /// When a runner's own card model differs from the registered id, the card
    /// is relabeled with the registered id and the original is preserved under a
    /// `runner-model` extra field.
    pub fn cards(&self) -> Vec<ModelCard> {
        self.entries.iter().map(Self::card_for_entry).collect()
    }

    /// Returns the model card for `model`, preserving registry relabeling.
    pub fn card_for(&self, model: &str) -> Option<ModelCard> {
        self.entry_for(model).map(Self::card_for_entry)
    }

    /// Dispatches `request` to the runner registered for `model`.
    ///
    /// Returns a `model_not_found` error when no runner matches `model`.
    pub fn infer(&self, cx: &mut Cx, model: &str, request: ModelRequest) -> Result<ModelResponse> {
        let runner = self
            .entry_for(model)
            .map(|entry| entry.runner.clone())
            .ok_or_else(|| Error::Eval(format!("model_not_found: {model}")))?;
        runner.infer(cx, request)
    }

    /// Returns `true` when no runners are registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
