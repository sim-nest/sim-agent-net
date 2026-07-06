use sim_kernel::{Cx, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelEvent, ModelEventSink};
use sim_lib_server::{
    FrameEnvelope, StreamSink, stream_chunk_frame_from_expr, stream_end_frame,
    stream_frame_from_expr,
};

pub(super) struct ModelEventStreamSink<'a> {
    cx: Cx,
    codec: Symbol,
    envelope: FrameEnvelope,
    sink: &'a mut dyn StreamSink,
}

impl<'a> ModelEventStreamSink<'a> {
    pub(super) fn new(
        seed: &Cx,
        codec: Symbol,
        envelope: FrameEnvelope,
        sink: &'a mut dyn StreamSink,
    ) -> Self {
        Self {
            cx: clone_stream_cx(seed),
            codec,
            envelope,
            sink,
        }
    }

    pub(super) fn emit_start(&mut self, metadata: Expr) -> Result<()> {
        let frame = stream_frame_from_expr(
            &mut self.cx,
            self.codec.clone(),
            sim_lib_server::FrameKind::StreamStart,
            &metadata,
            self.envelope.clone(),
        )?;
        self.sink.chunk(&mut self.cx, frame)
    }

    pub(super) fn emit_end(&mut self) -> Result<()> {
        let frame = stream_end_frame(self.codec.clone(), self.envelope.clone());
        self.sink.chunk(&mut self.cx, frame)
    }
}

impl ModelEventSink for ModelEventStreamSink<'_> {
    fn emit(&mut self, event: ModelEvent) -> Result<()> {
        let expr: Expr = event.into();
        let frame = stream_chunk_frame_from_expr(
            &mut self.cx,
            self.codec.clone(),
            &expr,
            self.envelope.clone(),
        )?;
        self.sink.chunk(&mut self.cx, frame)
    }
}

pub(super) struct TeeModelEventSink<'a> {
    primary: &'a mut dyn ModelEventSink,
    observer: &'a mut dyn ModelEventSink,
}

impl<'a> TeeModelEventSink<'a> {
    pub(super) fn new(
        primary: &'a mut dyn ModelEventSink,
        observer: &'a mut dyn ModelEventSink,
    ) -> Self {
        Self { primary, observer }
    }
}

impl ModelEventSink for TeeModelEventSink<'_> {
    fn emit(&mut self, event: ModelEvent) -> Result<()> {
        self.primary.emit(event.clone())?;
        self.observer.emit(event)
    }
}

#[derive(Default)]
pub(super) struct FinalEventTracker {
    seen_final: bool,
}

impl FinalEventTracker {
    pub(super) fn seen_final(&self) -> bool {
        self.seen_final
    }
}

impl ModelEventSink for FinalEventTracker {
    fn emit(&mut self, event: ModelEvent) -> Result<()> {
        if event.event == Symbol::new("final") {
            self.seen_final = true;
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct DiscardModelEventSink;

impl ModelEventSink for DiscardModelEventSink {
    fn emit(&mut self, _event: ModelEvent) -> Result<()> {
        Ok(())
    }
}

pub(super) fn model_stream_metadata(runner: Symbol, model: String) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("stream")),
            Expr::Symbol(Symbol::new("model-events")),
        ),
        (Expr::Symbol(Symbol::new("runner")), Expr::Symbol(runner)),
        (Expr::Symbol(Symbol::new("model")), Expr::String(model)),
    ])
}

fn clone_stream_cx(seed: &Cx) -> Cx {
    let (mut cloned, seat) = Cx::new_seated(seed.eval_policy_ref(), seed.factory_ref());
    *cloned.env_mut() = seed.env().clone();
    *cloned.registry_mut() = seed.registry().clone();
    *cloned.sources_mut() = seed.sources().clone();
    cloned.set_promotion_search_limits(seed.promotion_search_limits());
    cloned.set_control_policy(seed.control_policy_ref());
    for capability in seed.capabilities().iter() {
        seat.grant(&mut cloned, capability.clone());
    }
    if let Some(expander) = seed.macro_expander_ref() {
        cloned.set_macro_expander(expander);
    }
    cloned
}
