#[test]
fn auth_vocabulary_and_control_operations_are_stable_and_separate_from_inference() {
    let methods = [
        AuthMethod::ApiKey,
        AuthMethod::OauthBrowser,
        AuthMethod::OauthDevice,
        AuthMethod::Subscription,
        AuthMethod::BrokerOwned,
        AuthMethod::None,
    ];
    assert_eq!(
        methods.map(|method| method.symbol().to_string()),
        [
            "api-key",
            "oauth-browser",
            "oauth-device",
            "subscription",
            "broker-owned",
            "none"
        ]
    );
    assert_eq!(
        crate::provider_operation::all()
            .into_iter()
            .map(|op| op.to_string())
            .collect::<Vec<_>>(),
        [
            "provider/auth-methods",
            "provider/login",
            "provider/status",
            "provider/logout"
        ]
    );
}

#[test]
fn redacted_auth_metadata_round_trips_and_terms_gate_open_and_login() {
    let mut seat = seat_card();
    let metadata = AuthMetadata {
        owner: AuthOwner::Broker,
        session: SessionStatus::LoginRequired,
        required_terms: Some(("provider-terms".into(), "2026-08".into())),
        acknowledgement: None,
    };
    seat.set_auth_metadata(&metadata);
    assert_eq!(seat.auth_metadata().unwrap(), Some(metadata.clone()));
    let encoded = format!("{:?}", seat.extra);
    assert!(!encoded.contains("credential"));
    assert!(!encoded.contains("cookie"));

    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(FictionalAdapter)).unwrap();
    registry
        .discover(&mut test_cx_with_secret_capability(), Expr::Nil)
        .unwrap();
    registry.replace_seat_for_test(seat.clone());
    let mut cx = test_cx_with_secret_capability();
    let open_error = match registry.open(&mut cx, &seat.seat, Expr::Nil) {
        Ok(_) => panic!("unacknowledged terms must refuse open"),
        Err(error) => error,
    };
    assert!(open_error.to_string().contains("must be acknowledged"));
    assert!(
        registry
            .login(&mut cx, &seat.seat, AuthMethod::BrokerOwned)
            .unwrap_err()
            .to_string()
            .contains("must be acknowledged")
    );

    let mut accepted = metadata;
    accepted.acknowledgement = Some(TermsAcknowledgement {
        terms_id: "provider-terms".into(),
        revision: "2026-08".into(),
        acknowledged_by: "operator".into(),
    });
    seat.set_auth_metadata(&accepted);
    registry.replace_seat_for_test(seat.clone());
    let adapter_error = match registry.open(&mut cx, &seat.seat, Expr::Nil) {
        Ok(_) => panic!("fictional adapter unexpectedly opened"),
        Err(error) => error,
    };
    assert!(adapter_error.to_string().contains("no execution backend"));
}
