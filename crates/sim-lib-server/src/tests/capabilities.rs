use super::*;

#[test]
fn eval_local_surfaces_require_eval_fabric() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let connection = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "connect"))
                .unwrap(),
            vec![Expr::Symbol(Symbol::new("local"))],
        )
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "local-conn"), connection)
        .unwrap();

    let err = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "request"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "local-conn")),
                Expr::Nil,
            ],
        )
        .unwrap_err();
    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == eval_fabric_capability()
    ));
}

#[test]
fn file_read_trigger_requires_capability() {
    assert_trigger_source_requires_capability(
        quoted(Expr::List(vec![
            Expr::Symbol(Symbol::new("file-tail")),
            Expr::Symbol(Symbol::new(":path")),
            Expr::String("/tmp/sim-s10-file".to_owned()),
        ])),
        CapabilityName::new("file-read"),
    );
}

#[test]
fn mail_read_trigger_requires_capability() {
    assert_trigger_source_requires_capability(
        quoted(Expr::List(vec![
            Expr::Symbol(Symbol::new("imap")),
            Expr::Symbol(Symbol::new(":address")),
            Expr::String("mail.example".to_owned()),
            Expr::Symbol(Symbol::new(":mailbox")),
            Expr::String("INBOX".to_owned()),
        ])),
        CapabilityName::new("mail-read"),
    );
}

#[test]
fn mail_write_trigger_requires_capability() {
    assert_trigger_source_requires_capability(
        quoted(Expr::List(vec![
            Expr::Symbol(Symbol::new("smtp")),
            Expr::Symbol(Symbol::new(":address")),
            Expr::String("smtp.example".to_owned()),
        ])),
        CapabilityName::new("mail-write"),
    );
}

#[test]
fn telegram_trigger_requires_capability() {
    assert_trigger_source_requires_capability(
        quoted(Expr::List(vec![
            Expr::Symbol(Symbol::new("telegram")),
            Expr::Symbol(Symbol::new(":chat-id")),
            Expr::String("123".to_owned()),
            Expr::Symbol(Symbol::new(":bot")),
            Expr::String("bot".to_owned()),
        ])),
        CapabilityName::new("telegram-bot"),
    );
}

#[test]
fn matrix_trigger_requires_capability() {
    assert_trigger_source_requires_capability(
        quoted(Expr::List(vec![
            Expr::Symbol(Symbol::new("matrix")),
            Expr::Symbol(Symbol::new(":room-id")),
            Expr::String("!room:example".to_owned()),
        ])),
        CapabilityName::new("matrix-bot"),
    );
}

#[test]
fn webhook_trigger_requires_capability() {
    assert_trigger_source_requires_capability(
        quoted(Expr::List(vec![
            Expr::Symbol(Symbol::new("webhook")),
            Expr::Symbol(Symbol::new(":route")),
            Expr::String("/hook".to_owned()),
        ])),
        CapabilityName::new("webhook-serve"),
    );
}

#[test]
fn cron_trigger_requires_capability() {
    assert_trigger_source_requires_capability(
        quoted(Expr::List(vec![
            Expr::Symbol(Symbol::new("cron")),
            Expr::Symbol(Symbol::new(":spec")),
            Expr::String("*/5 * * * *".to_owned()),
        ])),
        CapabilityName::new("cron-schedule"),
    );
}

#[test]
fn agent_driver_requires_capability() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant(eval_fabric_capability());

    let err = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "repl"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::new(":driver")),
                Expr::List(vec![
                    Expr::Symbol(Symbol::new("agent")),
                    Expr::String("reviewer".to_owned()),
                ]),
            ],
        )
        .unwrap_err();
    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == CapabilityName::new("agent-drive")
    ));
}
