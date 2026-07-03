use super::agent_r18_support::{fixed_reply_connection, register_connection, verifier_connection};
use super::support::{as_component, eval_cx, install_agent_lib, install_roundtrip_codecs};
use sim_kernel::{Args, Consistency, Expr, ReadPolicy, Symbol};
use sim_lib_server::{Connection, EvalSite, FrameKind, ServerFrame};
use sim_lib_stream_core::{
    BufferPolicy, StreamDirection, StreamMedia, StreamMetadata, StreamPacket,
};

#[test]
fn persona_stream_transforms_data_chunks_and_passes_metadata() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let style = cx
        .call_function(
            &Symbol::qualified("persona", "style"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":voice")).unwrap(),
                cx.factory().string("terse".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":prefix")).unwrap(),
                cx.factory().string("SIM:".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":suffix")).unwrap(),
                cx.factory().string("[ok]".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let component = as_component(&style);
    let metadata = stream_metadata("stream/persona");
    let start = stream_frame(&mut cx, FrameKind::StreamStart, metadata.table_expr());
    let start_reply = component.answer(&mut cx, start).unwrap();
    assert_eq!(start_reply.kind, FrameKind::StreamStart);
    assert_eq!(
        start_reply
            .decode_expr(&mut cx, ReadPolicy::default())
            .unwrap(),
        metadata.table_expr()
    );

    let packet = StreamPacket::data(
        Symbol::qualified("stream/data", "model-event"),
        Expr::String("SIM: explain stream routing with extra words [ok]".to_owned()),
    );
    let chunk = stream_frame(&mut cx, FrameKind::StreamChunk, packet.to_expr());
    let chunk_reply = component.answer(&mut cx, chunk).unwrap();
    assert_eq!(chunk_reply.kind, FrameKind::StreamChunk);
    let StreamPacket::Data(shaped) = StreamPacket::try_from(
        chunk_reply
            .decode_expr(&mut cx, ReadPolicy::default())
            .unwrap(),
    )
    .unwrap() else {
        panic!("expected data packet");
    };
    assert_eq!(shaped.kind, Symbol::qualified("stream/data", "model-event"));
    let Expr::String(text) = shaped.payload else {
        panic!("expected shaped string payload");
    };
    assert!(text.starts_with("SIM: "));
    assert!(text.ends_with("[ok]"));
    assert!(text.split_whitespace().count() <= 14);
}

#[test]
fn router_stream_frames_are_not_collapsed_to_response() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let router = cx
        .call_function(
            &Symbol::qualified("router", "round-robin"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":targets")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![Expr::Symbol(Symbol::qualified(
                        "test", "worker",
                    ))]))
                    .unwrap(),
            ]),
        )
        .unwrap();
    let component = as_component(&router);
    let payload = StreamPacket::data(
        Symbol::qualified("stream/data", "model-event"),
        Expr::String("route me".to_owned()),
    )
    .to_expr();

    for kind in [
        FrameKind::StreamStart,
        FrameKind::StreamChunk,
        FrameKind::StreamEnd,
    ] {
        let expr = if kind == FrameKind::StreamStart {
            stream_metadata("stream/router").table_expr()
        } else {
            payload.clone()
        };
        let frame = stream_frame(&mut cx, kind.clone(), expr.clone());
        let reply = component.answer(&mut cx, frame).unwrap();
        assert_eq!(reply.kind, kind);
        if kind == FrameKind::StreamChunk {
            assert_eq!(
                reply.decode_expr(&mut cx, ReadPolicy::default()).unwrap(),
                expr
            );
        }
    }
}

#[test]
fn debate_and_speculate_preserve_stream_boundaries_and_cancellation_data() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let judge = cx
        .call_function(
            &Symbol::qualified("judge", "rubric"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":reference")).unwrap(),
                cx.factory().string("stream winner".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    register_connection(
        &mut cx,
        Symbol::qualified("test", "stream-pro"),
        fixed_reply_connection("stream winner"),
    );
    register_connection(
        &mut cx,
        Symbol::qualified("test", "stream-con"),
        fixed_reply_connection("stream loser"),
    );
    let debate = cx
        .call_function(
            &Symbol::qualified("topology", "debate"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":pro")).unwrap(),
                cx.resolve_value(&Symbol::qualified("test", "stream-pro"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":con")).unwrap(),
                cx.resolve_value(&Symbol::qualified("test", "stream-con"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":judge")).unwrap(),
                judge,
            ]),
        )
        .unwrap();
    assert_topology_stream_passthrough(
        &mut cx,
        debate.object().downcast_ref::<Connection>().unwrap(),
    );

    register_connection(
        &mut cx,
        Symbol::qualified("test", "stream-spec"),
        fixed_reply_connection("speculative"),
    );
    register_connection(
        &mut cx,
        Symbol::qualified("test", "stream-verify"),
        verifier_connection("speculative", "verified"),
    );
    let speculate = cx
        .call_function(
            &Symbol::qualified("topology", "speculate-verify"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":speculator")).unwrap(),
                cx.resolve_value(&Symbol::qualified("test", "stream-spec"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":verifier")).unwrap(),
                cx.resolve_value(&Symbol::qualified("test", "stream-verify"))
                    .unwrap(),
            ]),
        )
        .unwrap();
    assert_topology_stream_passthrough(
        &mut cx,
        speculate.object().downcast_ref::<Connection>().unwrap(),
    );
}

fn assert_topology_stream_passthrough(cx: &mut sim_kernel::Cx, connection: &Connection) {
    for kind in [
        FrameKind::StreamStart,
        FrameKind::StreamChunk,
        FrameKind::StreamEnd,
    ] {
        let payload = if kind == FrameKind::StreamChunk {
            branch_cancel_packet().to_expr()
        } else {
            Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("id")),
                    Expr::String("branch".to_owned()),
                ),
                (
                    Expr::Symbol(Symbol::new("kind")),
                    Expr::Symbol(Symbol::new("stream")),
                ),
            ])
        };
        let frame = stream_frame(cx, kind.clone(), payload);
        let reply = connection.site().answer(cx, frame).unwrap();
        assert_eq!(reply.kind, kind);
        if kind == FrameKind::StreamChunk {
            let StreamPacket::Data(packet) =
                StreamPacket::try_from(reply.decode_expr(cx, ReadPolicy::default()).unwrap())
                    .unwrap()
            else {
                panic!("expected cancellation data packet");
            };
            assert_eq!(
                packet.kind,
                Symbol::qualified("stream/data", "branch-cancelled")
            );
        }
    }
}

fn branch_cancel_packet() -> StreamPacket {
    StreamPacket::data(
        Symbol::qualified("stream/data", "branch-cancelled"),
        Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("branch")),
                Expr::String("speculative".to_owned()),
            ),
            (Expr::Symbol(Symbol::new("cancelled")), Expr::Bool(true)),
        ]),
    )
}

fn stream_metadata(id: &str) -> StreamMetadata {
    StreamMetadata::new(
        Symbol::new(id),
        StreamMedia::Data,
        StreamDirection::Source,
        Symbol::qualified("clock", "data"),
        BufferPolicy::bounded(8).unwrap(),
    )
}

fn stream_frame(cx: &mut sim_kernel::Cx, kind: FrameKind, expr: Expr) -> ServerFrame {
    ServerFrame::from_expr(
        cx,
        Symbol::qualified("codec", "binary"),
        kind,
        &expr,
        Consistency::LocalFirst,
        Vec::new(),
        false,
    )
    .unwrap()
}
