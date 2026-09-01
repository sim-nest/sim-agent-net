#[test]
fn complete_read_cache_preserves_scope_ttl_order_cursor_and_eviction() {
    let peer = Arc::new(Peer::new(
        "http",
        vec![
            Ok(discovery()),
            Ok(cards(true, false)),
            Ok(PeerReply::Complete(
                json!({"resultType":"complete","value":"a","ttlMs":10}),
            )),
            Ok(PeerReply::Complete(
                json!({"resultType":"complete","value":"bob","ttlMs":10}),
            )),
            Ok(PeerReply::Complete(
                json!({"resultType":"complete","value":"expired","ttlMs":10}),
            )),
        ],
    ));
    let client = make_client(peer.clone(), 2);
    let cancellation = Cancellation::new();
    let mut cx = bare_cx();
    let callable = client
        .import_cards(&mut cx, &context("alice", &cancellation, &[], 0))
        .unwrap()
        .remove(0);
    let params = json!({"text":"x","order":[3,1,2]});
    assert_eq!(
        client
            .invoke(
                &callable,
                params.clone(),
                &context("alice", &cancellation, &[], 1)
            )
            .unwrap()
            .value,
        json!("a")
    );
    assert_eq!(
        client
            .invoke(
                &callable,
                params.clone(),
                &context("alice", &cancellation, &[], 2)
            )
            .unwrap()
            .value,
        json!("a")
    );
    assert_eq!(
        client
            .invoke(
                &callable,
                params.clone(),
                &context("bob", &cancellation, &[], 3)
            )
            .unwrap()
            .value,
        json!("bob")
    );
    assert_eq!(
        client
            .invoke(&callable, params, &context("alice", &cancellation, &[], 12))
            .unwrap()
            .value,
        json!("expired")
    );
    assert_eq!(
        peer.methods()
            .iter()
            .filter(|m| m.as_str() == "tools/call")
            .count(),
        3
    );
}

#[test]
fn subscription_checks_ids_terminal_and_cancellation() {
    let frames = vec![
        json!({"type":"acknowledged","subscriptionId":"s1"}),
        json!({"type":"event","subscriptionId":"s1","event":{"n":1}}),
        json!({"type":"complete","subscriptionId":"s1","completedAtMs":44,"cancelled":false}),
    ];
    let peer = Arc::new(Peer::new(
        "http",
        vec![Ok(discovery()), Ok(PeerReply::Stream(frames))],
    ));
    let client = make_client(peer, 1);
    let cancellation = Cancellation::new();
    let stream = client
        .subscribe(
            "events/subscribe",
            json!({}),
            &context("p", &cancellation, &[], 0),
        )
        .unwrap();
    assert_eq!(stream.id, "s1");
    assert!(matches!(
        stream.events.last(),
        Some(ClientEvent::Complete {
            completed_at_ms: 44,
            ..
        })
    ));

    let bad = vec![
        json!({"type":"acknowledged","subscriptionId":"a"}),
        json!({"type":"complete","subscriptionId":"b","completedAtMs":1}),
    ];
    let peer = Arc::new(Peer::new(
        "http",
        vec![Ok(discovery()), Ok(PeerReply::Stream(bad))],
    ));
    let client = make_client(peer, 1);
    assert!(matches!(
        client.subscribe(
            "events/subscribe",
            json!({}),
            &context("p", &cancellation, &[], 0)
        ),
        Err(ClientError::Subscription(_))
    ));
}

#[test]
fn concurrent_subscriptions_keep_independent_ids() {
    let stream = |id: &str| {
        PeerReply::Stream(vec![
            json!({"type":"acknowledged","subscriptionId":id}),
            json!({"type":"complete","subscriptionId":id,"completedAtMs":1,"cancelled":false}),
        ])
    };
    let peer = Arc::new(Peer::new(
        "http",
        vec![Ok(discovery()), Ok(stream("a")), Ok(stream("b"))],
    ));
    let client = Arc::new(make_client(peer, 1));
    let cancellation = Cancellation::new();
    client
        .discover(&context("p", &cancellation, &[], 0))
        .unwrap();
    let ids = std::thread::scope(|scope| {
        let first = {
            let client = Arc::clone(&client);
            scope.spawn(move || {
                let cancellation = Cancellation::new();
                client
                    .subscribe(
                        "events/subscribe",
                        json!({"listen":1}),
                        &context("p", &cancellation, &[], 1),
                    )
                    .unwrap()
                    .id
            })
        };
        let second = {
            let client = Arc::clone(&client);
            scope.spawn(move || {
                let cancellation = Cancellation::new();
                client
                    .subscribe(
                        "events/subscribe",
                        json!({"listen":2}),
                        &context("p", &cancellation, &[], 1),
                    )
                    .unwrap()
                    .id
            })
        };
        [first.join().unwrap(), second.join().unwrap()]
    });
    assert_eq!(
        ids.into_iter().collect::<std::collections::BTreeSet<_>>(),
        ["a".into(), "b".into()].into()
    );
}

#[test]
fn policy_expiry_and_process_exit_invalidate_era() {
    let peer = Arc::new(Peer::new(
        "stdio",
        vec![
            Ok(discovery()),
            Err(BindingError::ProcessExited(9)),
            Ok(discovery()),
        ],
    ));
    let client = make_client(peer.clone(), 1);
    let cancellation = Cancellation::new();
    client
        .discover(&context("p", &cancellation, &[], 0))
        .unwrap();
    assert!(matches!(
        client.subscribe(
            "events/subscribe",
            json!({}),
            &context("p", &cancellation, &[], 1)
        ),
        Err(ClientError::Binding(BindingError::ProcessExited(9)))
    ));
    client
        .discover(&context("p", &cancellation, &[], 2))
        .unwrap();
    assert_eq!(
        peer.methods(),
        ["server/discover", "events/subscribe", "server/discover"]
    );
}
