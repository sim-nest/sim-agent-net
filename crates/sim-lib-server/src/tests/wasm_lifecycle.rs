use super::*;

#[test]
fn server_wasm_region_requires_sandbox_wasm_capability() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let err = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "wasm-region"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::new(":name")),
                Expr::String("shared-cap".to_owned()),
                Expr::Symbol(Symbol::new(":module")),
                Expr::Bytes(minimal_wasm_guest_bytes()),
            ],
        )
        .unwrap_err();
    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == CapabilityName::new("sandbox-wasm")
    ));
}

#[test]
fn server_wasm_region_registers_from_inline_bytes() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant_named("sandbox-wasm");

    let region = format!(
        "shared-inline-{}",
        NEXT_TEST_VALUE_ID.fetch_add(1, Ordering::Relaxed)
    );
    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "wasm-region"))
            .unwrap(),
        vec![
            Expr::Symbol(Symbol::new(":name")),
            Expr::String(region.clone()),
            Expr::Symbol(Symbol::new(":module")),
            Expr::Bytes(minimal_wasm_guest_bytes()),
        ],
    )
    .unwrap();

    assert!(lookup_wasm_region(&region).is_ok());
}

#[test]
fn server_wasm_region_registers_from_file_path() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant_named("sandbox-wasm");

    let region = format!(
        "shared-file-{}",
        NEXT_TEST_VALUE_ID.fetch_add(1, Ordering::Relaxed)
    );
    let path = std::env::temp_dir().join(format!(
        "sim-lib-server-wasm-{}.wasm",
        NEXT_TEST_VALUE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, minimal_wasm_guest_bytes()).unwrap();

    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "wasm-region"))
            .unwrap(),
        vec![
            Expr::Symbol(Symbol::new(":name")),
            Expr::String(region.clone()),
            Expr::Symbol(Symbol::new(":module")),
            Expr::String(path.display().to_string()),
        ],
    )
    .unwrap();

    assert!(lookup_wasm_region(&region).is_ok());
    let _ = fs::remove_file(path);
}

#[test]
fn server_start_with_wasm_address_requires_registered_region() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let err = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![
                Expr::Symbol(Symbol::new(":address")),
                Expr::List(vec![
                    Expr::Symbol(Symbol::new("wasm")),
                    Expr::Symbol(Symbol::new(":region")),
                    Expr::String("missing-start-region".to_owned()),
                ]),
            ],
        })
        .unwrap_err();
    assert!(
        matches!(err, sim_kernel::Error::Eval(message) if message == "unknown wasm region missing-start-region")
    );
}

#[test]
fn server_connect_with_wasm_address_requires_registered_region() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let err = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![quoted(Expr::List(vec![
                Expr::Symbol(Symbol::new("wasm")),
                Expr::Symbol(Symbol::new(":region")),
                Expr::String("missing-connect-region".to_owned()),
            ]))],
        )
        .unwrap_err();
    assert!(
        matches!(err, sim_kernel::Error::Eval(message) if message == "unknown wasm region missing-connect-region")
    );
}
