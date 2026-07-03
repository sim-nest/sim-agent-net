use std::sync::Arc;

use sim_codec_algol::AlgolCodecLib;
use sim_codec_binary::BinaryCodecLib;
use sim_codec_json::JsonCodecLib;
use sim_codec_lisp::LispCodecLib;
use sim_kernel::{DefaultFactory, EagerPolicy, Expr, Symbol, eval_fabric_capability};

use crate::{Connection, EvalSite, FrameKind, LocalEvalSite, ServerAddress, ServerFrame};

use super::runtime::lower_repl_expr;
use super::{DriverSpec, ReplOptions, ReplOutput, run_repl};

struct AgentTestDriver {
    used: bool,
}

impl super::LineDriver for AgentTestDriver {
    fn read_line(
        &mut self,
        _cx: &mut sim_kernel::Cx,
        _prompt: &str,
    ) -> sim_kernel::Result<Option<String>> {
        if self.used {
            Ok(None)
        } else {
            self.used = true;
            Ok(Some("42".to_owned()))
        }
    }

    fn write_output(&mut self, _cx: &mut sim_kernel::Cx, _output: &str) -> sim_kernel::Result<()> {
        Ok(())
    }
}

fn agent_test_driver_factory(
    _cx: &mut sim_kernel::Cx,
    _spec: &Expr,
) -> sim_kernel::Result<Option<Box<dyn super::LineDriver>>> {
    Ok(Some(Box::new(AgentTestDriver { used: false })))
}

fn cx() -> sim_kernel::Cx {
    let mut cx = sim_kernel::Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    let lisp = LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
    let json = JsonCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&json).unwrap();
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
    let algol = AlgolCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&algol).unwrap();
    crate::install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());
    cx
}

#[test]
fn driver_specs_parse_and_reflect() {
    let spec = DriverSpec::from_expr(&Expr::List(vec![
        Expr::Symbol(Symbol::new("buffer")),
        Expr::Symbol(Symbol::new(":path")),
        Expr::String("/tmp/repl.lisp".to_owned()),
        Expr::Symbol(Symbol::new(":on")),
        Expr::Quote {
            mode: sim_kernel::QuoteMode::Quote,
            expr: Box::new(Expr::Symbol(Symbol::new("save"))),
        },
    ]))
    .unwrap();
    assert_eq!(
        spec.as_expr(),
        Expr::List(vec![
            Expr::Symbol(Symbol::new("buffer")),
            Expr::Symbol(Symbol::new(":path")),
            Expr::String("/tmp/repl.lisp".to_owned()),
            Expr::Symbol(Symbol::new(":on")),
            Expr::Quote {
                mode: sim_kernel::QuoteMode::Quote,
                expr: Box::new(Expr::Symbol(Symbol::new("save"))),
            },
        ])
    );
}

#[test]
fn repl_lowers_traditional_lisp_call_syntax_without_changing_quote_data() {
    let lowered = lower_repl_expr(Expr::List(vec![
        Expr::Symbol(Symbol::new("*")),
        Expr::List(vec![
            Expr::Symbol(Symbol::new("+")),
            Expr::String("left".to_owned()),
            Expr::String("right".to_owned()),
        ]),
        Expr::Quote {
            mode: sim_kernel::QuoteMode::Quote,
            expr: Box::new(Expr::List(vec![Expr::Symbol(Symbol::new("+"))])),
        },
    ]));

    assert_eq!(
        lowered,
        Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("math", "mul"))),
            args: vec![
                Expr::Call {
                    operator: Box::new(Expr::Symbol(Symbol::qualified("math", "add"))),
                    args: vec![
                        Expr::String("left".to_owned()),
                        Expr::String("right".to_owned()),
                    ],
                },
                Expr::Quote {
                    mode: sim_kernel::QuoteMode::Quote,
                    expr: Box::new(Expr::List(vec![Expr::Symbol(Symbol::new("+"))])),
                },
            ],
        }
    );
}

#[test]
fn one_shot_repl_returns_rendered_string() {
    let mut cx = cx();
    let codecs = vec![
        Symbol::qualified("codec", "lisp"),
        Symbol::qualified("codec", "json"),
        Symbol::qualified("codec", "binary"),
        Symbol::qualified("codec", "algol"),
    ];
    let connection = Connection::new(
        ServerAddress::Local,
        Symbol::qualified("codec", "json"),
        codecs.clone(),
        Arc::new(LocalEvalSite::new(ServerAddress::Local, codecs)),
    )
    .unwrap();

    let value = run_repl(
        &mut cx,
        ReplOptions {
            connection,
            codec: Symbol::qualified("codec", "lisp"),
            prompt: "sim> ".to_owned(),
            driver: DriverSpec::Line,
            input: Some("42".to_owned()),
            output: ReplOutput::String,
        },
    )
    .unwrap();
    assert_eq!(
        value.object().as_expr(&mut cx).unwrap(),
        Expr::String("42".to_owned())
    );
}

#[derive(Clone)]
struct FrameEchoSite;

impl EvalSite for FrameEchoSite {
    fn site_kind(&self) -> &'static str {
        "frame-echo"
    }

    fn address(&self) -> &ServerAddress {
        static ADDRESS: std::sync::OnceLock<ServerAddress> = std::sync::OnceLock::new();
        ADDRESS.get_or_init(|| ServerAddress::Local)
    }

    fn codecs(&self) -> &[Symbol] {
        static CODECS: std::sync::OnceLock<Vec<Symbol>> = std::sync::OnceLock::new();
        CODECS.get_or_init(|| {
            vec![
                Symbol::qualified("codec", "json"),
                Symbol::qualified("codec", "lisp"),
            ]
        })
    }

    fn answer(
        &self,
        cx: &mut sim_kernel::Cx,
        frame: ServerFrame,
    ) -> sim_kernel::Result<ServerFrame> {
        assert_eq!(frame.kind, FrameKind::Request);
        let expr = crate::frame::eval_request_from_frame(cx, &frame)?.expr;
        crate::server_frame_from_reply(
            cx,
            &frame.codec,
            sim_kernel::EvalReply {
                value: cx.factory().expr(expr)?,
                diagnostics: Vec::new(),
                trace: None,
            },
            sim_kernel::Consistency::LocalFirst,
        )
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn one_shot_repl_routes_through_connection_frames() {
    let mut cx = cx();
    let connection = Connection::new(
        ServerAddress::Local,
        Symbol::qualified("codec", "json"),
        vec![
            Symbol::qualified("codec", "json"),
            Symbol::qualified("codec", "lisp"),
        ],
        Arc::new(FrameEchoSite),
    )
    .unwrap();

    let value = run_repl(
        &mut cx,
        ReplOptions {
            connection,
            codec: Symbol::qualified("codec", "lisp"),
            prompt: "sim> ".to_owned(),
            driver: DriverSpec::Line,
            input: Some("42".to_owned()),
            output: ReplOutput::String,
        },
    )
    .unwrap();
    assert_eq!(
        value.object().as_expr(&mut cx).unwrap(),
        Expr::String("42".to_owned())
    );
}

#[test]
fn registered_agent_driver_factory_is_used_for_unavailable_specs() {
    let mut cx = cx();
    cx.grant(sim_kernel::CapabilityName::new("agent-drive"));
    crate::register_line_driver(Symbol::new("agent"), agent_test_driver_factory).unwrap();

    let connection = Connection::new(
        ServerAddress::Local,
        Symbol::qualified("codec", "json"),
        vec![
            Symbol::qualified("codec", "json"),
            Symbol::qualified("codec", "lisp"),
        ],
        Arc::new(FrameEchoSite),
    )
    .unwrap();

    let value = run_repl(
        &mut cx,
        ReplOptions {
            connection,
            codec: Symbol::qualified("codec", "lisp"),
            prompt: "sim> ".to_owned(),
            driver: DriverSpec::from_expr(&Expr::List(vec![
                Expr::Symbol(Symbol::new("agent")),
                Expr::Symbol(Symbol::new("reviewer")),
            ]))
            .unwrap(),
            input: None,
            output: ReplOutput::Driver,
        },
    )
    .unwrap();

    assert!(matches!(
        value.object().as_expr(&mut cx).unwrap(),
        Expr::Nil
    ));
}
