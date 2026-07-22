use sim_codec_bridge::{
    FrameHoleKind, FrameKind, frame_book_content_id, render_frame, standard_frame_book,
};
use sim_kernel::{Expr, Symbol, testing::bare_cx as cx};
use sim_value::access::field;

use crate::{approve_frame_proposal, propose_frame, proposed_frame_part};

#[test]
fn unmapped_intent_yields_frame_proposal() {
    let mut cx = cx();
    let proposal = propose_frame(&mut cx, "summarize the customer transcript").unwrap();
    let expr = proposal.to_expr();

    assert_eq!(proposal.kind, FrameKind::Task);
    assert_eq!(proposal.template, "{action} {target}.");
    assert_eq!(
        proposal
            .holes
            .iter()
            .map(|hole| hole.kind)
            .collect::<Vec<_>>(),
        vec![FrameHoleKind::Term, FrameHoleKind::Term]
    );
    assert_eq!(
        field(&expr, "status"),
        Some(&Expr::Symbol(Symbol::qualified("forge", "candidate")))
    );
    assert_eq!(
        proposal.content_id().unwrap(),
        propose_frame(&mut cx, "summarize the customer transcript")
            .unwrap()
            .content_id()
            .unwrap()
    );
}

#[test]
fn unapproved_proposal_not_registered() {
    let mut cx = cx();
    let proposal = propose_frame(&mut cx, "summarize the customer transcript").unwrap();
    let book = standard_frame_book();

    assert!(book.spec(&proposal.id).is_none());
    let err = proposed_frame_part(
        &book,
        &proposal,
        "summarize the customer transcript",
        Symbol::new("T1"),
    )
    .unwrap_err();
    assert!(err.to_string().contains("unknown BRIDGE frame"));
}

#[test]
fn approved_frame_compiles_next_time() {
    let mut cx = cx();
    let proposal = propose_frame(&mut cx, "summarize the customer transcript").unwrap();
    let mut book = standard_frame_book();
    let before = frame_book_content_id(&book).unwrap();
    let after = approve_frame_proposal(&mut book, &proposal).unwrap();
    let part = proposed_frame_part(
        &book,
        &proposal,
        "summarize the customer transcript",
        Symbol::new("T1"),
    )
    .unwrap();
    let spec = book.require_spec(&proposal.id).unwrap();
    let rendered = render_frame(spec, &part.payload).unwrap();

    assert_ne!(before, after);
    assert_eq!(part.kind, Symbol::qualified("bridge", "Frame"));
    assert!(rendered.contains("You MUST summarize transcript."));
}
