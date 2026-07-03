use super::support::{eval_cx, install_agent_lib, install_test_codec};
use crate::{
    AgentComponent, ComponentBackend, ComponentKind, ModelCard, ModelRequest, ModelResponse,
    ModelRunner, RunnerBackend,
};
use sim_kernel::{Consistency, Cx, EvalFabric, EvalMode, EvalRequest, Expr, Result, Symbol};
use sim_lib_server::ServerAddress;
use std::sync::Arc;

struct StaticRunner;

impl ModelRunner for StaticRunner {
    fn card(&self) -> ModelCard {
        ModelCard::new(
            Symbol::new("runner/external"),
            "external/model",
            Symbol::new("test"),
            Symbol::new("local"),
        )
    }

    fn infer(&self, _cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        Ok(ModelResponse::new(
            Symbol::new("runner/external"),
            "external/model",
            vec![Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("type")),
                    Expr::Symbol(Symbol::new("text")),
                ),
                (Expr::Symbol(Symbol::new("text")), request.task),
            ])],
            Symbol::new("stop"),
        ))
    }
}

#[test]
fn a5_phase3_external_runner_backend_realizes_model_requests() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let component = AgentComponent {
        symbol: Symbol::new("runner/external"),
        kind: ComponentKind::Runner,
        capabilities: Vec::new(),
        address: ServerAddress::Local,
        codecs: crate::util::installed_codecs(&cx),
        spec: vec![(
            Symbol::new("backend"),
            Expr::Symbol(Symbol::new("external")),
        )],
        backend: ComponentBackend::Runner(RunnerBackend::External {
            runner: Arc::new(StaticRunner),
        }),
    };

    let reply = component
        .realize(
            &mut cx,
            EvalRequest {
                expr: Expr::Map(vec![
                    (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
                    (
                        Expr::Symbol(Symbol::new("task")),
                        Expr::String("phase-3".to_owned()),
                    ),
                    (
                        Expr::Symbol(Symbol::new("messages")),
                        Expr::List(Vec::new()),
                    ),
                ]),
                result_shape: None,
                required_capabilities: Vec::new(),
                deadline: None,
                consistency: Consistency::default(),
                mode: EvalMode::default(),
                answer_limit: None,
                stream_buffer: None,
                stream: false,
                trace: false,
            },
        )
        .unwrap();

    let expr = reply.value.object().as_expr(&mut cx).unwrap();
    let response = ModelResponse::try_from(expr).unwrap();
    assert_eq!(response.model, "external/model");
    assert_eq!(response.runner, Symbol::new("runner/external"));
}
