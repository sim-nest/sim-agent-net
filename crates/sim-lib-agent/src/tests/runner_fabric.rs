use super::support::{eval_cx, flatten_text, install_agent_lib, install_test_codec};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Expr, Symbol};

fn model_request_expr(task: &str) -> Expr {
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String(task.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(Vec::new()),
        ),
    ])
}

#[test]
fn a5_phase2_runner_echo_is_a_realize_target() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "echo"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("echo-fabric")).unwrap(),
                cx.factory().symbol(Symbol::new(":model")).unwrap(),
                cx.factory().string("echo/model".to_owned()).unwrap(),
            ]),
        )
        .unwrap();

    assert!(runner.object().as_eval_fabric().is_some());

    let response = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "realize"))),
            args: vec![
                model_request_expr("hi"),
                Expr::Symbol(Symbol::new(":on")),
                Expr::Call {
                    operator: Box::new(Expr::Symbol(Symbol::qualified("runner", "echo"))),
                    args: vec![
                        Expr::Symbol(Symbol::new(":name")),
                        Expr::Symbol(Symbol::new("echo-fabric")),
                        Expr::Symbol(Symbol::new(":model")),
                        Expr::String("echo/model".to_owned()),
                    ],
                },
            ],
        })
        .unwrap();
    let response_expr = response.object().as_expr(&mut cx).unwrap();
    validate_chat_transcript(&response_expr).unwrap();
    assert!(flatten_text(&response_expr).contains("hi"));
}

#[test]
fn a5_phase2_runner_fake_is_a_realize_target() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let runner = cx
        .call_function(
            &Symbol::qualified("runner", "fake"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":script")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![Expr::String("scripted".to_owned())]))
                    .unwrap(),
            ]),
        )
        .unwrap();

    assert!(runner.object().as_eval_fabric().is_some());

    let response = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "realize"))),
            args: vec![
                model_request_expr("ignored"),
                Expr::Symbol(Symbol::new(":on")),
                Expr::Call {
                    operator: Box::new(Expr::Symbol(Symbol::qualified("runner", "fake"))),
                    args: vec![
                        Expr::Symbol(Symbol::new(":script")),
                        Expr::List(vec![Expr::String("scripted".to_owned())]),
                    ],
                },
            ],
        })
        .unwrap();
    let response_expr = response.object().as_expr(&mut cx).unwrap();
    validate_chat_transcript(&response_expr).unwrap();
    assert!(flatten_text(&response_expr).contains("scripted"));
}
