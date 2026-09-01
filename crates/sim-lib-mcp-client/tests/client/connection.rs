#[test]
fn endpoint_identity_normalizes_and_excludes_credentials() {
    assert_eq!(
        EndpointIdentity::http("HTTPS://Example.COM:443//mcp/?display=x#y")
            .unwrap()
            .to_string(),
        "https://example.com/mcp"
    );
    assert!(EndpointIdentity::http("https://secret@example.com/mcp").is_err());
    assert_ne!(
        EndpointIdentity::process("child-1").unwrap(),
        EndpointIdentity::process("child-2").unwrap()
    );
}

#[test]
fn modern_probe_import_and_effecting_call_happen_once() {
    let peer = Arc::new(Peer::new(
        "http",
        vec![
            Ok(discovery()),
            Ok(cards(false, true)),
            Ok(PeerReply::Complete(
                json!({"resultType":"complete","value":"ok"}),
            )),
        ],
    ));
    let client = make_client(peer.clone(), 4);
    let cancellation = Cancellation::new();
    let mut cx = bare_cx();
    let callable = client
        .import_cards(&mut cx, &context("alice", &cancellation, &[], 1))
        .unwrap()
        .remove(0);
    assert_eq!(callable.icons()[0].src, "https://icons.invalid/echo.svg");
    let outcome = client
        .invoke(
            &callable,
            json!({"text":"hello"}),
            &context("alice", &cancellation, &[], 2),
        )
        .unwrap();
    assert_eq!(outcome.value, json!("ok"));
    assert_eq!(
        peer.methods(),
        ["server/discover", "server/cards", "tools/call"]
    );
    assert_eq!(peer.seen.lock().unwrap()[2].era, Era::Modern);
}

#[test]
fn exact_http_400_and_stdio_method_not_found_fall_back_before_call() {
    let body =
        serde_json::to_vec(&json!({"error":{"code":-32601,"message":"Method not found"}})).unwrap();
    for (kind, error) in [
        ("http", BindingError::Http { status: 400, body }),
        (
            "stdio",
            BindingError::Rpc {
                code: -32601,
                message: "anything".into(),
                data: None,
            },
        ),
    ] {
        let peer = Arc::new(Peer::new(
            kind,
            vec![
                Err(error),
                Ok(legacy_discovery()),
                Ok(cards(false, true)),
                Ok(PeerReply::Complete(json!({"value":"once"}))),
            ],
        ));
        let client = make_client(peer.clone(), 2);
        let cancellation = Cancellation::new();
        let mut cx = bare_cx();
        let callable = client
            .import_cards(&mut cx, &context("p", &cancellation, &[], 0))
            .unwrap()
            .remove(0);
        client
            .invoke(
                &callable,
                json!({"text":"x"}),
                &context("p", &cancellation, &[], 1),
            )
            .unwrap();
        assert_eq!(
            peer.methods(),
            [
                "server/discover",
                "initialize",
                "server/cards",
                "tools/call"
            ]
        );
        let seen = peer.seen.lock().unwrap();
        assert_ne!(seen[0].id, seen[1].id);
        assert_eq!(seen[3].era, Era::Legacy);
    }
}

#[test]
fn unrecognized_discovery_timeout_and_near_miss_http_body_fail_closed() {
    let near =
        serde_json::to_vec(&json!({"error":{"code":-32601,"message":"method not found"}})).unwrap();
    for error in [
        BindingError::Rpc {
            code: -32000,
            message: "no".into(),
            data: None,
        },
        BindingError::Http {
            status: 400,
            body: near,
        },
    ] {
        let peer = Arc::new(Peer::new("http", vec![Err(error)]));
        let client = make_client(peer.clone(), 1);
        let cancellation = Cancellation::new();
        assert!(matches!(
            client.discover(&context("p", &cancellation, &[], 0)),
            Err(ClientError::UnrecognizedDiscovery)
        ));
        assert_eq!(peer.methods(), ["server/discover"]);
    }
    let peer = Arc::new(Peer::new("stdio", vec![Err(BindingError::Timeout)]));
    let client = make_client(peer, 1);
    let cancellation = Cancellation::new();
    assert!(matches!(
        client.discover(&context("p", &cancellation, &[], 0)),
        Err(ClientError::Binding(BindingError::Timeout))
    ));
}

#[test]
fn schema_rejection_occurs_before_application_call() {
    let peer = Arc::new(Peer::new(
        "http",
        vec![Ok(discovery()), Ok(cards(false, true))],
    ));
    let client = make_client(peer.clone(), 1);
    let cancellation = Cancellation::new();
    let mut cx = bare_cx();
    let callable = client
        .import_cards(&mut cx, &context("p", &cancellation, &[], 0))
        .unwrap()
        .remove(0);
    assert!(matches!(
        client.invoke(
            &callable,
            json!({"text":3}),
            &context("p", &cancellation, &[], 1)
        ),
        Err(ClientError::Schema(_))
    ));
    assert_eq!(peer.methods(), ["server/discover", "server/cards"]);
}

#[test]
fn mrtr_uses_fresh_id_exact_state_and_bounds_capabilities() {
    let requested = BTreeMap::from([("confirm".into(), "input.confirm".into())]);
    let peer = Arc::new(Peer::new(
        "stdio",
        vec![
            Ok(discovery()),
            Ok(cards(false, true)),
            Ok(PeerReply::InputRequired {
                request_state: json!({"opaque":"exact"}),
                requested,
            }),
            Ok(PeerReply::Complete(json!({"value":"done"}))),
        ],
    ));
    let client = make_client(peer.clone(), 1);
    let cancellation = Cancellation::new();
    let caps = ["input.confirm".into()];
    let mut cx = bare_cx();
    let callable = client
        .import_cards(&mut cx, &context("p", &cancellation, &caps, 0))
        .unwrap()
        .remove(0);
    client
        .invoke(
            &callable,
            json!({"text":"x"}),
            &context("p", &cancellation, &caps, 1),
        )
        .unwrap();
    let seen = peer.seen.lock().unwrap();
    assert_ne!(seen[2].id, seen[3].id);
    assert_eq!(seen[3].params["requestState"], json!({"opaque":"exact"}));
}
