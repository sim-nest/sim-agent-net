use super::*;

#[test]
fn trigger_uses_injected_server_wall_clock_for_frame_timestamp() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(read_eval_capability());

    let codecs = installed_codecs();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let site = Arc::new(RecordingSite {
        address: ServerAddress::Local,
        codecs: codecs.clone(),
        seen: seen.clone(),
    });
    let server = Arc::new(
        Server::new(
            ServerAddress::Local,
            codecs[0].clone(),
            codecs.clone(),
            crate::ThreadMode::Coop,
            IsolationPolicy::default(),
            None,
            site,
            Vec::new(),
        )
        .unwrap()
        .with_wall_clock(Arc::new(crate::DeterministicWallClock::new(42_000, 0))),
    );
    let trigger = crate::trigger::register_trigger(
        &mut cx,
        server,
        Expr::Symbol(Symbol::new("stdin")),
        Expr::Symbol(codecs[0].clone()),
        None,
        codecs[0].clone(),
    )
    .unwrap();

    trigger.inject_text(&mut cx, "\"tick\"").unwrap();
    assert_eq!(
        seen.lock()
            .expect("recording site mutex poisoned")
            .as_slice(),
        &[FrameKind::Trigger {
            source: Symbol::new("stdin"),
            when_ms: 42_000,
        }]
    );
    trigger.stop().unwrap();
}
