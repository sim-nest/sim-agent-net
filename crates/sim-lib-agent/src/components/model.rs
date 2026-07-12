use crate::{Component, ComponentKind, util::value_from_expr};
use sim_kernel::{
    CapabilityName, ClassRef, Cx, Error, EvalFabric, EvalReply, EvalRequest, Expr, Object,
    ObjectCompat, Result, Symbol, Value,
};
use sim_lib_agent_runner_core::ModelRunner;
use sim_lib_server::{
    EvalSite, ServerAddress, ServerFrame, StreamSink, eval_reply_from_frame,
    server_frame_from_request,
};
use std::{
    any::Any,
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

#[sim_citizen_derive::non_citizen(
    reason = "agent component runtime shell; reconstruct from topology/Package and agent-runner/ModelCard descriptors",
    kind = "marker"
)]
#[derive(Clone)]
pub(crate) struct AgentComponent {
    pub(crate) symbol: Symbol,
    pub(crate) kind: ComponentKind,
    pub(crate) capabilities: Vec<CapabilityName>,
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
    pub(crate) spec: Vec<(Symbol, Expr)>,
    pub(crate) backend: ComponentBackend,
}

#[derive(Clone)]
pub(crate) enum ComponentBackend {
    Runner(RunnerBackend),
    Planner(PlannerBackend),
    Judge(JudgeBackend),
    Router(RouterBackend),
    Persona(PersonaBackend),
    Retriever(RetrieverBackend),
    Sandbox(SandboxBackend),
    Recorder(RecorderBackend),
    Voice(VoiceBackend),
}

/// The strategy a runner component uses to produce model responses.
#[derive(Clone)]
pub enum RunnerBackend {
    /// Echoes the request back, attributed to a named model.
    Echo {
        /// Name of the model this backend reports as.
        model: String,
    },
    /// Replays pre-recorded responses from a fixed cassette.
    Cassette {
        /// Name of the model this backend reports as.
        model: String,
        /// Whether replay fails when the request does not match the recording.
        strict: bool,
        /// The recorded response entries, replayed in order.
        entries: Arc<Vec<Expr>>,
    },
    /// Emits scripted responses with a simulated delay, for testing.
    Fake {
        /// Name of the model this backend reports as.
        model: String,
        /// Queue of scripted responses, consumed per request.
        script: Arc<Mutex<VecDeque<Expr>>>,
        /// Artificial delay applied before each response.
        delay: Duration,
    },
    /// Delegates to an external [`ModelRunner`] implementation.
    External {
        /// The backing model runner.
        runner: Arc<dyn ModelRunner>,
    },
}

#[derive(Clone)]
pub(crate) enum PlannerBackend {
    Budget {
        max_turns: Option<u32>,
        max_cost: Option<f64>,
    },
    Refine,
    Parallel {
        branches: u32,
    },
    Chain,
}

#[derive(Clone)]
pub(crate) enum JudgeBackend {
    Rubric { config: JudgeConfig },
    RankedVote { config: JudgeConfig },
    Threshold { threshold: f64, config: JudgeConfig },
}

#[derive(Clone)]
pub(crate) enum RouterBackend {
    RoundRobin {
        targets: Vec<Symbol>,
        cursor: Arc<Mutex<usize>>,
    },
    Bid {
        targets: Vec<Symbol>,
        metric: Symbol,
        auction_window: Duration,
    },
    Sticky {
        targets: Vec<Symbol>,
        sticky_key: Symbol,
    },
}

#[derive(Clone)]
pub(crate) enum PersonaBackend {
    Style {
        voice: String,
        prefix: String,
        suffix: String,
        max_words: usize,
    },
    Language {
        language: String,
        glossary: Vec<(String, String)>,
    },
    Translator {
        from: String,
        to: String,
        table: Vec<(String, String)>,
        command: Option<String>,
    },
}

#[derive(Clone)]
pub(crate) struct JudgeConfig {
    pub(crate) rubric: Expr,
    pub(crate) criteria: Vec<JudgeCriterion>,
    pub(crate) reference: Option<String>,
    pub(crate) style_markers: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct JudgeCriterion {
    pub(crate) name: String,
    pub(crate) weight: f64,
}

#[derive(Clone)]
pub(crate) enum RetrieverBackend {
    Vector { store: String, corpus: Vec<String> },
    Web { endpoint: String },
    File { root: Option<PathBuf> },
    Db { path: PathBuf },
}

#[derive(Clone)]
pub(crate) enum SandboxBackend {
    Wasm {
        region: String,
    },
    Subprocess {
        command: Option<String>,
        max_time: Duration,
        max_output_bytes: usize,
    },
    CapabilityRestricted {
        allowed: Vec<CapabilityName>,
    },
}

#[derive(Clone)]
pub(crate) enum RecorderBackend {
    Journal {
        path: Option<PathBuf>,
        entries: crate::memory::SharedEntries,
    },
    Audit {
        path: Option<PathBuf>,
        entries: crate::memory::SharedEntries,
    },
    Prometheus {
        namespace: String,
        entries: crate::memory::SharedEntries,
    },
}

#[derive(Clone)]
pub(crate) enum VoiceBackend {
    Tts {
        voice: String,
        command: Option<String>,
    },
    Stt {
        locale: String,
        command: Option<String>,
    },
}

impl Object for AgentComponent {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<component {}>", self.symbol))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for AgentComponent {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Table"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            sim_kernel::CORE_TABLE_CLASS_ID,
            Symbol::qualified("core", "Table"),
        )
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table(cx)?.object().as_expr(cx)
    }
    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        let mut entries = vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(component_kind_symbol(&self.kind))?,
            ),
            (
                Symbol::new("name"),
                cx.factory().symbol(self.symbol.clone())?,
            ),
            (
                Symbol::new("capabilities"),
                cx.factory().list(
                    self.capabilities
                        .iter()
                        .map(|capability| cx.factory().string(capability.as_str().to_owned()))
                        .collect::<Result<Vec<_>>>()?,
                )?,
            ),
        ];
        for (key, value) in &self.spec {
            entries.push((key.clone(), value_from_expr(cx, value)?));
        }
        cx.factory().table(entries)
    }
    fn as_eval_fabric(&self) -> Option<&dyn EvalFabric> {
        matches!(&self.backend, ComponentBackend::Runner(_)).then_some(self)
    }
}

impl EvalSite for AgentComponent {
    fn site_kind(&self) -> &'static str {
        match self.kind {
            ComponentKind::Runner => "runner",
            ComponentKind::Planner => "planner",
            ComponentKind::Judge => "judge",
            ComponentKind::Router => "router",
            ComponentKind::Persona => "persona",
            ComponentKind::Retriever => "retriever",
            ComponentKind::Sandbox => "sandbox",
            ComponentKind::Recorder => "recorder",
            ComponentKind::Voice => "voice",
            ComponentKind::Tool => "tool",
            ComponentKind::Memory => "memory",
            ComponentKind::Topology => "topology",
            ComponentKind::Custom(_) => "component",
        }
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        self.backend.answer(cx, self, frame)
    }

    fn stream(&self, cx: &mut Cx, frame: ServerFrame, sink: &mut dyn StreamSink) -> Result<()> {
        match &self.backend {
            ComponentBackend::Runner(backend) => {
                super::runtime::stream_runner(cx, self, backend, frame, sink)
            }
            _ => {
                let reply = self.answer(cx, frame)?;
                sink.chunk(cx, reply)?;
                sink.end(cx)
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Component for AgentComponent {
    fn kind(&self) -> ComponentKind {
        self.kind.clone()
    }

    fn name(&self) -> &Symbol {
        &self.symbol
    }

    fn capabilities(&self) -> &[CapabilityName] {
        &self.capabilities
    }

    fn reflect(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table(cx)?.object().as_expr(cx)
    }
}

impl EvalFabric for AgentComponent {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        match &self.backend {
            ComponentBackend::Runner(_) => {
                let codec = self.codecs.first().ok_or_else(|| {
                    Error::Eval(format!("{} has no installed codec", self.symbol))
                })?;
                let frame = server_frame_from_request(cx, codec, request)?;
                let reply = self.answer(cx, frame)?;
                eval_reply_from_frame(cx, &reply)
            }
            _ => Err(Error::Eval(format!(
                "{} is not a realize target",
                self.symbol
            ))),
        }
    }
}

impl ComponentBackend {
    fn answer(
        &self,
        cx: &mut Cx,
        component: &AgentComponent,
        frame: ServerFrame,
    ) -> Result<ServerFrame> {
        match self {
            Self::Runner(backend) => super::runtime::answer_runner(cx, component, backend, frame),
            Self::Planner(backend) => super::runtime::answer_planner(cx, component, backend, frame),
            Self::Judge(backend) => super::runtime::answer_judge(cx, component, backend, frame),
            Self::Router(backend) => super::runtime::answer_router(cx, component, backend, frame),
            Self::Persona(backend) => super::runtime::answer_persona(cx, component, backend, frame),
            Self::Retriever(backend) => {
                super::runtime::answer_retriever(cx, component, backend, frame)
            }
            Self::Sandbox(backend) => super::runtime::answer_sandbox(cx, component, backend, frame),
            Self::Recorder(backend) => {
                super::runtime::answer_recorder(cx, component, backend, frame)
            }
            Self::Voice(backend) => super::runtime::answer_voice(cx, component, backend, frame),
        }
    }
}

pub(crate) fn component_kind_symbol(kind: &ComponentKind) -> Symbol {
    match kind {
        ComponentKind::Runner => Symbol::new("runner"),
        ComponentKind::Tool => Symbol::new("tool"),
        ComponentKind::Memory => Symbol::new("memory"),
        ComponentKind::Planner => Symbol::new("planner"),
        ComponentKind::Judge => Symbol::new("judge"),
        ComponentKind::Router => Symbol::new("router"),
        ComponentKind::Persona => Symbol::new("persona"),
        ComponentKind::Retriever => Symbol::new("retriever"),
        ComponentKind::Sandbox => Symbol::new("sandbox"),
        ComponentKind::Recorder => Symbol::new("recorder"),
        ComponentKind::Voice => Symbol::new("voice"),
        ComponentKind::Topology => Symbol::new("topology"),
        ComponentKind::Custom(symbol) => symbol.clone(),
    }
}

pub(crate) fn component_value(cx: &mut Cx, component: AgentComponent) -> Result<Value> {
    cx.factory().opaque(Arc::new(component))
}

pub(crate) fn number_expr(value: u32) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

pub(crate) fn number_expr_from_f64(value: f64) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}
