use super::model::{AgentComponent, ComponentBackend, RunnerBackend};
use super::placement::ModelSiteKey;
use sim_kernel::{CapabilityName, Error, Expr, Result, Symbol};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModelSiteCard {
    key: String,
    locality: Symbol,
    model: String,
    available: bool,
    codecs: Vec<Symbol>,
    caps: Vec<CapabilityName>,
}

impl ModelSiteCard {
    pub(crate) fn from_runner(key: &ModelSiteKey, component: &AgentComponent) -> Result<Self> {
        let ComponentBackend::Runner(backend) = &component.backend else {
            return Err(Error::Eval(
                "model site cards require a runner component".to_owned(),
            ));
        };
        let (model, locality) = match backend {
            RunnerBackend::Echo { model } | RunnerBackend::Cassette { model, .. } => {
                (model.clone(), Symbol::new("local"))
            }
            RunnerBackend::Fake { model, .. } => (model.clone(), Symbol::new("fake")),
            RunnerBackend::External { runner } => {
                let card = runner.card();
                (card.model, card.locality)
            }
        };
        Ok(Self {
            key: key.as_str().to_owned(),
            locality,
            model,
            available: true,
            codecs: component.codecs.clone(),
            caps: component.capabilities.clone(),
        })
    }

    pub(crate) fn from_loaded(key: &ModelSiteKey, model: String, codecs: Vec<Symbol>) -> Self {
        Self {
            key: key.as_str().to_owned(),
            locality: Symbol::new("loaded"),
            model,
            available: true,
            codecs,
            caps: Vec::new(),
        }
    }

    pub(crate) fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    pub(crate) fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            key_bool("model-site-card", true),
            key_expr("key", Expr::String(self.key.clone())),
            key_expr("locality", Expr::Symbol(self.locality.clone())),
            key_expr("model", Expr::String(self.model.clone())),
            key_expr("available", Expr::Bool(self.available)),
            key_expr(
                "codecs",
                Expr::List(self.codecs.iter().cloned().map(Expr::Symbol).collect()),
            ),
            key_expr(
                "caps",
                Expr::List(
                    self.caps
                        .iter()
                        .map(|capability| Expr::Symbol(capability.as_symbol()))
                        .collect(),
                ),
            ),
        ])
    }
}

pub(crate) fn model_sites_expr(cards: &[ModelSiteCard]) -> Expr {
    Expr::Map(vec![
        key_bool("model-sites", true),
        key_expr(
            "sites",
            Expr::List(cards.iter().map(ModelSiteCard::to_expr).collect()),
        ),
    ])
}

fn key_bool(key: &str, value: bool) -> (Expr, Expr) {
    key_expr(key, Expr::Bool(value))
}

fn key_expr(key: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(key)), value)
}
