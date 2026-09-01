use std::sync::Arc;

use sim_kernel::{
    Args, Callable, ClassRef, EvalFabric, EvalReply, EvalRequest, Expr, Object, Result, Symbol,
    Value,
};
use sim_lib_stream_core::{
    ClockDomain, LatencyClass, PlacedFragment, RateContract, StreamDirection, StreamEndpointKind,
    StreamMedia, StreamPacket, stream_edge,
};

use crate::{
    Coroutine, CoroutineEvalSite, EvalSite, FabricEvalSite, FrameEnvelope, FrameKind,
    LocalEvalSite, LoopEvalSite, PipelineEvalSite, ServerAddress, ServerFrame, Site, SiteKind,
    realize_stream_events,
};

use super::{cx, installed_codecs};

#[test]
fn eval_site_family_implements_site_trait() -> Result<()> {
    let cx = cx();
    let codecs = installed_codecs();
    let local = LocalEvalSite::new(ServerAddress::Local, codecs.clone());
    let coroutine_handler = cx.factory().opaque(Arc::new(CoroutineEchoFn))?;
    let coroutine = Arc::new(Coroutine::new(
        ServerAddress::Coroutine { id: 1 },
        coroutine_handler,
    ));
    let coroutine_site = CoroutineEvalSite::new(
        ServerAddress::Coroutine { id: coroutine.id() },
        codecs.clone(),
        coroutine,
    );
    let pipeline = PipelineEvalSite::new(
        ServerAddress::Pipeline { steps: Vec::new() },
        codecs.clone(),
        Vec::new(),
    );
    let loop_site = LoopEvalSite::new(
        ServerAddress::Pipeline { steps: Vec::new() },
        codecs.clone(),
        Vec::new(),
        1,
        cx.factory().bool(true)?,
    );
    let fabric = FabricEvalSite::new("fabric", ServerAddress::Local, codecs, Arc::new(EchoFabric));

    assert_site(&local, SiteKind::Local);
    assert_site(&coroutine_site, SiteKind::Coroutine);
    assert_site(&pipeline, SiteKind::Pipeline);
    assert_site(&loop_site, SiteKind::Loop);
    assert_site(&fabric, SiteKind::Fabric);
    Ok(())
}

#[test]
fn fabric_site_preserves_stream_frames_without_realizing_them() -> Result<()> {
    let mut cx = cx();
    let site = FabricEvalSite::new(
        "fabric",
        ServerAddress::Local,
        installed_codecs(),
        Arc::new(EchoFabric),
    );

    for kind in [
        FrameKind::StreamStart,
        FrameKind::StreamChunk,
        FrameKind::StreamEnd,
    ] {
        let mut frame = ServerFrame::new(
            Symbol::qualified("codec", "lisp"),
            kind,
            FrameEnvelope::default(),
            vec![0xde, 0xad, 0xbe, 0xef],
        );
        frame.msg_id = Some(17);
        frame.correlate = Some(9);

        assert_eq!(site.answer(&mut cx, frame.clone())?, frame);
    }
    Ok(())
}

#[test]
fn placed_fragment_runs_at_thread_and_coroutine_sites_with_same_envelopes() -> Result<()> {
    let mut cx = cx();
    let codecs = installed_codecs();
    let thread_site = LocalEvalSite::new(ServerAddress::InProcess { thread: 7 }, codecs.clone());
    let coroutine_handler = cx.factory().opaque(Arc::new(CoroutineEchoFn))?;
    let coroutine = Arc::new(Coroutine::new(
        ServerAddress::Coroutine { id: 2 },
        coroutine_handler,
    ));
    let coroutine_site = CoroutineEvalSite::new(
        ServerAddress::Coroutine { id: coroutine.id() },
        codecs,
        coroutine,
    );
    let fragment = fragment("same-result");

    let thread_events = realize_stream_events(&mut cx, fragment.clone(), &thread_site)?;
    let coroutine_events = realize_stream_events(&mut cx, fragment, &coroutine_site)?;

    assert_eq!(payloads(&thread_events), payloads(&coroutine_events));
    assert_eq!(thread_events[0].clock_domain(), ClockDomain::Control);
    assert_eq!(thread_site.kind(), SiteKind::Local);
    assert_eq!(coroutine_site.kind(), SiteKind::Coroutine);
    Ok(())
}

fn assert_site<T: Site>(site: &T, expected: SiteKind) {
    assert_eq!(site.kind(), expected);
    assert_eq!(site.endpoint_kind(), StreamEndpointKind::EvalSite);
    assert_eq!(site.clock_domain(), ClockDomain::Control);
    assert_eq!(site.latency_class(), LatencyClass::Interactive);
}

fn fragment(text: &str) -> PlacedFragment {
    PlacedFragment::new(
        Symbol::qualified("test", "placed-fragment"),
        Expr::String(text.to_owned()),
    )
    .with_output_edge(stream_edge(
        "out",
        StreamMedia::Data,
        StreamDirection::Source,
        RateContract::control(),
    ))
}

fn payloads(envelopes: &[sim_lib_stream_core::StreamEnvelope]) -> Vec<Expr> {
    envelopes
        .iter()
        .map(|envelope| match envelope.packet() {
            StreamPacket::Data(packet) => packet.payload.clone(),
            other => panic!("expected data packet, got {other:?}"),
        })
        .collect()
}

struct EchoFabric;

impl EvalFabric for EchoFabric {
    fn realize(&self, cx: &mut sim_kernel::Cx, request: EvalRequest) -> Result<EvalReply> {
        Ok(EvalReply {
            value: cx.factory().expr(request.expr)?,
            diagnostics: Vec::new(),
            trace: None,
        })
    }
}

#[derive(Clone)]
struct CoroutineEchoFn;

impl Object for CoroutineEchoFn {
    fn display(&self, _cx: &mut sim_kernel::Cx) -> Result<String> {
        Ok("#<function test/coroutine-echo>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for CoroutineEchoFn {
    fn class(&self, cx: &mut sim_kernel::Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            sim_kernel::CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }

    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for CoroutineEchoFn {
    fn call(&self, _cx: &mut sim_kernel::Cx, args: Args) -> Result<Value> {
        args.values()
            .get(1)
            .cloned()
            .ok_or_else(|| sim_kernel::Error::Eval("coroutine echo expects input".to_owned()))
    }
}
