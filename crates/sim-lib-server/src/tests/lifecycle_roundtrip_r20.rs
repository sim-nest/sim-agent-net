use super::*;

#[derive(Clone)]
struct IsolationReportSite {
    expected_factory: Arc<dyn sim_kernel::Factory>,
}

impl sim_kernel::Object for IsolationReportSite {
    fn display(&self, _cx: &mut sim_kernel::Cx) -> sim_kernel::Result<String> {
        Ok("#<site test/isolation-report>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for IsolationReportSite {
    fn class(&self, cx: &mut sim_kernel::Cx) -> sim_kernel::Result<sim_kernel::ClassRef> {
        cx.factory()
            .class_stub(sim_kernel::ClassId(0), Symbol::qualified("core", "Site"))
    }
}

impl crate::EvalSite for IsolationReportSite {
    fn site_kind(&self) -> &'static str {
        "isolation-report"
    }

    fn address(&self) -> &crate::ServerAddress {
        static LOCAL: crate::ServerAddress = crate::ServerAddress::Local;
        &LOCAL
    }

    fn codecs(&self) -> &[Symbol] {
        static CODECS: std::sync::OnceLock<Vec<Symbol>> = std::sync::OnceLock::new();
        CODECS.get_or_init(installed_codecs)
    }

    fn answer(
        &self,
        cx: &mut sim_kernel::Cx,
        frame: crate::ServerFrame,
    ) -> sim_kernel::Result<crate::ServerFrame> {
        let _ = crate::eval_request_from_frame(cx, &frame)?;
        let child_only = cx.factory().string("child-only".to_owned())?;
        cx.env_mut().define(Symbol::new("child-only"), child_only);
        let report = cx.factory().expr(Expr::Map(vec![
            bool_entry(
                "shared-env",
                cx.env().get(&Symbol::new("shared-env")).is_some(),
            ),
            bool_entry(
                "imported-env",
                cx.env()
                    .get(&Symbol::qualified("snap", "imported-env"))
                    .is_some(),
            ),
            bool_entry(
                "shared-cap",
                cx.capabilities()
                    .contains(&CapabilityName::new("shared.cap")),
            ),
            bool_entry(
                "imported-cap",
                cx.capabilities()
                    .contains(&CapabilityName::new("imported.cap")),
            ),
            bool_entry(
                "factory-shared",
                Arc::ptr_eq(&cx.factory_ref(), &self.expected_factory),
            ),
            bool_entry(
                "server-request-visible",
                cx.registry()
                    .function_by_symbol(&Symbol::qualified("server", "request"))
                    .is_some(),
            ),
            bool_entry(
                "json-codec-visible",
                cx.registry()
                    .codec_by_symbol(&Symbol::qualified("codec", "json"))
                    .is_some(),
            ),
            bool_entry(
                "child-write-visible",
                cx.env().get(&Symbol::new("child-only")).is_some(),
            ),
        ]))?;
        let diagnostics = cx.take_diagnostics();
        crate::server_frame_from_reply(
            cx,
            &frame.codec,
            sim_kernel::EvalReply {
                value: report,
                diagnostics,
                trace: None,
            },
            frame.envelope.consistency,
        )
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn child_and_import_isolation_axes_apply_over_real_tcp_connections() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());
    cx.grant(eval_remote_capability());
    cx.grant_named("network");
    cx.grant_named("shared.cap");
    let shared_env = cx.factory().bool(true).unwrap();
    cx.env_mut().define(Symbol::new("shared-env"), shared_env);
    let imported_env_snapshot = cx
        .factory()
        .expr(Expr::Map(vec![(
            Expr::Symbol(Symbol::qualified("snap", "imported-env")),
            Expr::Bool(true),
        )]))
        .unwrap();
    let imported_caps_snapshot = cx
        .factory()
        .expr(Expr::List(vec![Expr::String("imported.cap".to_owned())]))
        .unwrap();
    let imported_registry_snapshot = cx
        .factory()
        .expr(Expr::List(vec![
            Expr::Symbol(Symbol::new("server")),
            Expr::Symbol(Symbol::qualified("codec", "binary")),
        ]))
        .unwrap();
    let expected_factory = cx.factory_ref();
    let isolation_site = cx
        .factory()
        .opaque(Arc::new(
            Connection::new(
                ServerAddress::Local,
                Symbol::qualified("codec", "binary"),
                installed_codecs(),
                Arc::new(IsolationReportSite { expected_factory }),
            )
            .unwrap(),
        ))
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("snap", "env"), imported_env_snapshot)
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("snap", "caps"), imported_caps_snapshot)
        .unwrap();
    cx.registry_mut()
        .register_value(
            Symbol::qualified("snap", "registry"),
            imported_registry_snapshot,
        )
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "isolation-site"), isolation_site)
        .unwrap();

    let child_server = match cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "start"))
            .unwrap(),
        vec![
            Expr::Symbol(Symbol::new(":address")),
            tcp_loopback_expr(),
            Expr::Symbol(Symbol::new(":site")),
            Expr::Symbol(Symbol::qualified("test", "isolation-site")),
            Expr::Symbol(Symbol::new(":isolate")),
            Expr::List(vec![
                Expr::Symbol(Symbol::new(":env")),
                Expr::Symbol(Symbol::new("child")),
                Expr::Symbol(Symbol::new(":libs")),
                Expr::Symbol(Symbol::new("share")),
                Expr::Symbol(Symbol::new(":factory")),
                Expr::Symbol(Symbol::new("share")),
                Expr::Symbol(Symbol::new(":registry")),
                Expr::Symbol(Symbol::new("share")),
                Expr::Symbol(Symbol::new(":capabilities")),
                Expr::Symbol(Symbol::new("share")),
            ]),
        ],
    ) {
        Ok(server) => server,
        Err(sim_kernel::Error::HostError(message)) if message.contains("PermissionDenied") => {
            return;
        }
        Err(error) => panic!("tcp child server start failed: {error}"),
    };
    cx.registry_mut()
        .register_value(
            Symbol::qualified("test", "child-tcp-server"),
            child_server.clone(),
        )
        .unwrap();
    let child_report = request_report(&mut cx, Symbol::qualified("test", "child-tcp-server"));
    assert!(map_bool_field(&child_report, "shared-env"));
    assert!(!map_bool_field(&child_report, "imported-env"));
    assert!(map_bool_field(&child_report, "shared-cap"));
    assert!(!map_bool_field(&child_report, "imported-cap"));
    assert!(map_bool_field(&child_report, "factory-shared"));
    assert!(map_bool_field(&child_report, "server-request-visible"));
    assert!(map_bool_field(&child_report, "json-codec-visible"));
    assert!(map_bool_field(&child_report, "child-write-visible"));
    assert!(cx.env().get(&Symbol::new("child-only")).is_none());
    stop_server(&mut cx, child_server);

    let libs_server = match cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "start"))
            .unwrap(),
        vec![
            Expr::Symbol(Symbol::new(":address")),
            tcp_loopback_expr(),
            Expr::Symbol(Symbol::new(":site")),
            Expr::Symbol(Symbol::qualified("test", "isolation-site")),
            Expr::Symbol(Symbol::new(":isolate")),
            Expr::List(vec![
                Expr::Symbol(Symbol::new(":env")),
                Expr::Symbol(Symbol::new("share")),
                Expr::Symbol(Symbol::new(":libs")),
                Expr::Symbol(Symbol::new("isolate")),
                Expr::Symbol(Symbol::new(":factory")),
                Expr::Symbol(Symbol::new("share")),
                Expr::Symbol(Symbol::new(":registry")),
                Expr::Symbol(Symbol::new("share")),
                Expr::Symbol(Symbol::new(":capabilities")),
                Expr::Symbol(Symbol::new("share")),
            ]),
        ],
    ) {
        Ok(server) => server,
        Err(sim_kernel::Error::HostError(message)) if message.contains("PermissionDenied") => {
            return;
        }
        Err(error) => panic!("tcp libs server start failed: {error}"),
    };
    cx.registry_mut()
        .register_value(
            Symbol::qualified("test", "libs-tcp-server"),
            libs_server.clone(),
        )
        .unwrap();
    let libs_error = request_report_error(&mut cx, Symbol::qualified("test", "libs-tcp-server"));
    assert!(!libs_error.is_empty());
    stop_server(&mut cx, libs_server);

    let import_server = match cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "start"))
            .unwrap(),
        vec![
            Expr::Symbol(Symbol::new(":address")),
            tcp_loopback_expr(),
            Expr::Symbol(Symbol::new(":site")),
            Expr::Symbol(Symbol::qualified("test", "isolation-site")),
            Expr::Symbol(Symbol::new(":isolate")),
            Expr::List(vec![
                Expr::Symbol(Symbol::new(":env")),
                Expr::List(vec![
                    Expr::Symbol(Symbol::new("import")),
                    Expr::Symbol(Symbol::qualified("snap", "env")),
                ]),
                Expr::Symbol(Symbol::new(":libs")),
                Expr::Symbol(Symbol::new("share")),
                Expr::Symbol(Symbol::new(":factory")),
                Expr::Symbol(Symbol::new("isolate")),
                Expr::Symbol(Symbol::new(":registry")),
                Expr::List(vec![
                    Expr::Symbol(Symbol::new("import")),
                    Expr::Symbol(Symbol::qualified("snap", "registry")),
                ]),
                Expr::Symbol(Symbol::new(":capabilities")),
                Expr::List(vec![
                    Expr::Symbol(Symbol::new("import")),
                    Expr::Symbol(Symbol::qualified("snap", "caps")),
                ]),
            ]),
        ],
    ) {
        Ok(server) => server,
        Err(sim_kernel::Error::HostError(message)) if message.contains("PermissionDenied") => {
            return;
        }
        Err(error) => panic!("tcp import server start failed: {error}"),
    };
    cx.registry_mut()
        .register_value(
            Symbol::qualified("test", "import-tcp-server"),
            import_server.clone(),
        )
        .unwrap();
    let import_report = request_report(&mut cx, Symbol::qualified("test", "import-tcp-server"));
    assert!(!map_bool_field(&import_report, "shared-env"));
    assert!(map_bool_field(&import_report, "imported-env"));
    assert!(!map_bool_field(&import_report, "shared-cap"));
    assert!(map_bool_field(&import_report, "imported-cap"));
    assert!(!map_bool_field(&import_report, "factory-shared"));
    assert!(map_bool_field(&import_report, "server-request-visible"));
    assert!(!map_bool_field(&import_report, "json-codec-visible"));
    stop_server(&mut cx, import_server);
}

fn bool_entry(key: &str, value: bool) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(key)), Expr::Bool(value))
}

fn map_bool_field(expr: &Expr, key: &str) -> bool {
    let Expr::Map(entries) = expr else {
        panic!("expected map expr, found {expr:?}");
    };
    entries
        .iter()
        .find_map(|(entry_key, entry_value)| match (entry_key, entry_value) {
            (Expr::Symbol(symbol), Expr::Bool(value)) if symbol.name.as_ref() == key => {
                Some(*value)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing bool field {key} in {expr:?}"))
}

fn request_report(cx: &mut sim_kernel::Cx, server: Symbol) -> Expr {
    let connection = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![Expr::Symbol(server)],
        )
        .unwrap();
    let connection_symbol = Symbol::qualified(
        "test",
        format!(
            "report-conn-{}",
            NEXT_TEST_VALUE_ID.fetch_add(1, Ordering::Relaxed)
        ),
    );
    cx.registry_mut()
        .register_value(connection_symbol.clone(), connection)
        .unwrap();
    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "request"))
            .unwrap(),
        vec![
            Expr::Symbol(connection_symbol),
            Expr::String("inspect".to_owned()),
        ],
    )
    .unwrap()
    .object()
    .as_expr(cx)
    .unwrap()
}

fn request_report_error(cx: &mut sim_kernel::Cx, server: Symbol) -> String {
    let connection = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![Expr::Symbol(server)],
        )
        .unwrap();
    let connection_symbol = Symbol::qualified(
        "test",
        format!(
            "report-error-conn-{}",
            NEXT_TEST_VALUE_ID.fetch_add(1, Ordering::Relaxed)
        ),
    );
    cx.registry_mut()
        .register_value(connection_symbol.clone(), connection)
        .unwrap();
    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "request"))
            .unwrap(),
        vec![
            Expr::Symbol(connection_symbol),
            Expr::String("inspect".to_owned()),
        ],
    )
    .unwrap_err()
    .to_string()
}

fn stop_server(cx: &mut sim_kernel::Cx, server: sim_kernel::Value) {
    cx.call_function(
        &Symbol::qualified("server", "stop"),
        sim_kernel::Args::new(vec![server]),
    )
    .unwrap();
}

fn tcp_loopback_expr() -> Expr {
    Expr::List(vec![
        Expr::Symbol(Symbol::new("tcp")),
        Expr::Symbol(Symbol::new(":host")),
        Expr::String("127.0.0.1".to_owned()),
        Expr::Symbol(Symbol::new(":port")),
        Expr::String("0".to_owned()),
    ])
}
