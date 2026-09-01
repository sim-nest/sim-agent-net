use sim_kernel::{Cx, Expr, HandleSeed, Result, Symbol};
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
    seed.fork_from_seed(HandleSeed::new(1))
}

#[cfg(test)]
mod fork_tests {
    use super::clone_stream_cx;
    use sim_kernel::{CapabilityName, testing::bare_cx};

    #[test]
    fn clone_stream_cx_preserves_seed_capabilities() {
        let mut seed = bare_cx();
        let capability = CapabilityName::new("test.stream-fork");
        seed.grant(capability.clone());
        let fork = clone_stream_cx(&seed);
        assert!(fork.capabilities().contains(&capability));
    }
}
