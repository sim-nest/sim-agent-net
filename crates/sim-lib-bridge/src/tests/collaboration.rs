use super::*;

#[test]
fn collaboration_is_packet_based() {
    let base = stamp_packet_cid(&collaboration_base_packet()).unwrap();
    let patch = stamp_packet_cid(&patch_reply(
        &base,
        "human:reviewer",
        Expr::String("accepted answer".to_owned()),
    ))
    .unwrap();
    let merged = merge_bridge_replies(&base, &[patch], &MergePolicy::Single).unwrap();
    let patch = BridgePatchPayload::from_expr(&merged.body[0].payload).unwrap();

    assert_eq!(merged.header.move_kind, Symbol::new("patch"));
    assert_eq!(patch.parent_cid, base.header.cid.unwrap());
    assert_eq!(patch.target, "body/O2/payload");
    assert_eq!(
        patch.replacement,
        Expr::String("accepted answer".to_owned())
    );
}

#[test]
fn merge_still_satisfies_root_return() {
    let mut cx = cx();
    let book = BridgeBook::standard();
    let base = stamp_packet_cid(&collaboration_base_packet()).unwrap();
    let patch = stamp_packet_cid(&patch_reply(
        &base,
        "model:synthesizer",
        Expr::String("accepted answer".to_owned()),
    ))
    .unwrap();
    let vote = stamp_packet_cid(&vote_reply(&base, "model:judge")).unwrap();
    let merged = merge_bridge_replies(
        &base,
        &[patch, vote],
        &MergePolicy::SynthesisThenVote {
            synthesizer: "model:synthesizer".to_owned(),
            min_votes: 1,
        },
    )
    .unwrap();
    let report = rx_check(&mut cx, &book, &merged, Some(&base)).unwrap();

    assert!(report.accepted());
}

#[test]
fn effective_capabilities_never_exceed_ceiling() {
    let cx = cx();
    let packet = request_packet(
        Expr::Symbol(Symbol::qualified("core", "Any")),
        vec![Symbol::qualified("ai", "run")],
    );
    let caps = effective_caps(&cx, &packet).unwrap();

    assert!(caps.contains(&CapabilityName::new("ai/run")));
    assert!(!caps.contains(&CapabilityName::new("bridge/given.materialize")));
}

#[test]
fn content_key_changes_when_request_changes() {
    let left = ContentKey::from_request(&eval_request("left"));
    let right = ContentKey::from_request(&eval_request("right"));

    assert_ne!(left, right);
}

#[test]
fn one_frame_record_yields_both_faces() {
    let book = BridgeBook::standard();
    let packet = stamp_packet_cid(
        &bridge_brief(
            "model:drafter",
            BridgeFramePayload::new(Symbol::qualified("bridge", "produce-artifact"))
                .with_slot(
                    Symbol::new("what"),
                    Expr::Symbol(Symbol::qualified("bridge", "proposal")),
                )
                .with_slot(
                    Symbol::new("target"),
                    Expr::String("sim-human-model".to_owned()),
                ),
            Expr::Symbol(Symbol::qualified("core", "String")),
        )
        .unwrap(),
    )
    .unwrap();
    let canonical = encode_bridge_text(&packet, &book).unwrap();
    let (face, spans) = render_model_face(&book, &packet).unwrap();

    assert!(canonical.contains("FRAME T1 payload="));
    assert!(face.starts_with(&canonical));
    assert!(face.contains("FLUENT"));
    assert!(face.contains("[T1] You MUST produce bridge/proposal for sim-human-model."));
    assert_total_ownership(&face, &spans).unwrap();
}

#[test]
fn matching_books_accept() {
    let mut cx = cx();
    let book = BridgeBook::standard().with_warrant_policy(BridgeWarrantPolicy::Verify);
    let packet = prepare_packet(
        &mut cx,
        &book,
        &request_packet(
            Expr::Symbol(Symbol::qualified("core", "Any")),
            vec![Symbol::qualified("ai", "run")],
        ),
    )
    .unwrap();
    let report = rx_check(&mut cx, &book, &packet, None).unwrap();

    assert!(packet.warrant.is_some());
    assert!(report.accepted());
}

#[test]
fn stale_book_emits_fetch_obligation() {
    let mut cx = cx();
    let book = BridgeBook::standard().with_warrant_policy(BridgeWarrantPolicy::Verify);
    let packet = prepare_packet(
        &mut cx,
        &book,
        &request_packet(
            Expr::Symbol(Symbol::qualified("core", "Any")),
            vec![Symbol::qualified("ai", "run")],
        ),
    )
    .unwrap();
    let stale_book = BridgeBook::standard()
        .with_part(BridgePartSpec::new(
            Symbol::qualified("bridge", "Frame"),
            Expr::Symbol(Symbol::qualified("bridge", "StaleFrame")),
            RenderClass::Frame,
            AuthorityClass::Normative,
            UnknownPolicy::Reject,
        ))
        .with_warrant_policy(BridgeWarrantPolicy::Verify);
    let report = rx_check(&mut cx, &stale_book, &packet, None).unwrap();

    assert!(!report.accepted());
    assert!(report.obligations.iter().any(|obligation| {
        obligation.path == "warrant/parts/bridge/Frame"
            && obligation.reason == "typed context expansion requires Fetch"
            && obligation.expected.starts_with("bridge/Fetch core/")
            && obligation
                .repair_menu
                .contains(&"send Fetch packet".to_owned())
    }));
}

#[test]
fn forged_warrant_cid_is_obligation_not_panic() {
    let mut cx = cx();
    let book = BridgeBook::standard().with_warrant_policy(BridgeWarrantPolicy::Verify);
    let mut packet = prepare_packet(
        &mut cx,
        &book,
        &request_packet(
            Expr::Symbol(Symbol::qualified("core", "Any")),
            vec![Symbol::qualified("ai", "run")],
        ),
    )
    .unwrap();
    let forged = Datum::String("forged bridge warrant cid".to_owned())
        .content_id()
        .unwrap();
    packet.warrant.as_mut().unwrap().parts[0].1 = forged.clone();
    let packet = stamp_packet_cid(&packet).unwrap();
    let report = rx_check(&mut cx, &book, &packet, None).unwrap();

    assert!(!report.accepted());
    assert!(report.obligations.iter().any(|obligation| {
        obligation.path == "warrant/parts/bridge/Frame"
            && obligation.expected == format!("bridge/Fetch {}", content_id_string(&forged))
    }));
}

#[test]
fn frontier_emits_flat_oneof_that_lowers() {
    let mut cx = cx();
    let packet = bridge_brief(
        "model:drafter",
        BridgeFramePayload::new(Symbol::qualified("bridge", "produce-artifact"))
            .with_slot(
                Symbol::new("what"),
                Expr::Symbol(Symbol::qualified("bridge", "proposal")),
            )
            .with_slot(
                Symbol::new("target"),
                Expr::String("sim-human-model".to_owned()),
            ),
        Expr::Symbol(Symbol::qualified("core", "String")),
    )
    .unwrap();
    let menu = frontier(&mut cx, &packet).unwrap();

    assert_eq!(menu.slots.len(), 2);
    assert!(format!("{:?}", menu.heads).contains("reply"));
    assert!(menu.grammar.contains(r#""anyOf""#));
    assert!(menu.grammar.contains("reply"));
}

#[test]
fn bridge_brief_runtime_export_constructs_packet() {
    let mut cx = cx();
    let exported = BridgeLib
        .manifest()
        .exports
        .iter()
        .any(|export| matches!(export, Export::Function { symbol, .. } if *symbol == bridge_brief_symbol()));
    assert!(exported);

    let target = cx
        .factory()
        .expr(Expr::String("model:drafter".to_owned()))
        .unwrap();
    let frame = cx
        .factory()
        .expr(
            BridgeFramePayload::new(Symbol::qualified("bridge", "produce-artifact"))
                .with_slot(
                    Symbol::new("what"),
                    Expr::Symbol(Symbol::qualified("bridge", "proposal")),
                )
                .with_slot(
                    Symbol::new("target"),
                    Expr::String("sim-human-model".to_owned()),
                )
                .to_expr(),
        )
        .unwrap();
    let return_shape = cx
        .factory()
        .expr(Expr::Symbol(Symbol::qualified("core", "String")))
        .unwrap();

    let value = BridgeFunction::new(BridgeFunctionKind::Brief)
        .call(&mut cx, Args::new(vec![target, frame, return_shape]))
        .unwrap();
    let packet =
        sim_codec_bridge::expr_to_packet(&value.object().as_expr(&mut cx).unwrap()).unwrap();

    assert_eq!(packet.header.to, vec!["model:drafter".to_owned()]);
    assert_eq!(packet.body[0].kind, Symbol::qualified("bridge", "Frame"));
    assert_eq!(packet.body[1].kind, Symbol::qualified("bridge", "Return"));
}
