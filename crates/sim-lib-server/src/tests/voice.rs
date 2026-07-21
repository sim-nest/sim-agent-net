use crate::{
    EvalSite, MIC_CAPTURE_KIND, MIC_CAPTURE_NAMESPACE, ModeledAsrFabric, ServerAddress,
    modeled_asr_site,
};
use sim_kernel::{Consistency, EvalMode, EvalReply, Expr, Symbol};
use sim_value::{access, build};

use super::{EvalFabric, EvalRequest, cx, installed_codecs};

#[test]
fn modeled_asr_fabric_realizes_watch_mic_capture() {
    let mut cx = cx();
    let fabric = ModeledAsrFabric::new("fixture-asr");
    let reply = fabric
        .realize(&mut cx, request(mic_capture_expr()))
        .expect("modeled ASR realizes raw mic capture");
    let expr = reply_expr(&mut cx, reply);

    assert_eq!(
        access::field(&expr, "kind"),
        Some(&build::qsym("asr", "transcript"))
    );
    assert_eq!(
        access::field_str(&expr, "text"),
        Some("fixture-asr: seq 12, 2 frame(s), 5 byte(s)")
    );
}

#[test]
fn modeled_asr_site_exposes_eval_fabric_and_rejects_transcript_inputs() {
    let mut cx = cx();
    let site = modeled_asr_site(ServerAddress::Local, installed_codecs(), "watch-relay");
    assert_eq!(site.site_kind(), "watch-asr");
    assert_eq!(site.address(), &ServerAddress::Local);
    let fabric = site.as_eval_fabric().expect("site exposes eval fabric");

    let mut entries = match mic_capture_expr() {
        Expr::Map(entries) => entries,
        _ => unreachable!(),
    };
    entries.push((build::sym("transcript"), build::text("bypass")));
    let err = match fabric.realize(&mut cx, request(Expr::Map(entries))) {
        Ok(_) => panic!("input transcript side channel should be rejected"),
        Err(err) => err,
    };

    assert!(format!("{err}").contains("unexpected field transcript"));
}

fn request(expr: Expr) -> EvalRequest {
    EvalRequest {
        expr,
        result_shape: None,
        required_capabilities: Vec::new(),
        deadline: None,
        consistency: Consistency::LocalFirst,
        mode: EvalMode::Eval,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: false,
    }
}

fn reply_expr(cx: &mut sim_kernel::Cx, reply: EvalReply) -> Expr {
    reply.value.object().as_expr(cx).unwrap()
}

fn mic_capture_expr() -> Expr {
    build::map(vec![
        (
            "kind",
            Expr::Symbol(Symbol::qualified(MIC_CAPTURE_NAMESPACE, MIC_CAPTURE_KIND)),
        ),
        (
            "frames",
            build::list(vec![
                audio_frame(100, vec![1, 2]),
                audio_frame(120, vec![3, 4, 5]),
            ]),
        ),
        ("seq", build::uint(12)),
        ("sample-rate-hz", build::uint(16_000)),
        ("channels", build::uint(1)),
    ])
}

fn audio_frame(at_ms: u64, pcm: Vec<u8>) -> Expr {
    build::map(vec![
        ("kind", build::qsym("watch", "audio-frame")),
        ("at-ms", build::uint(at_ms)),
        ("pcm", Expr::Bytes(pcm)),
    ])
}
