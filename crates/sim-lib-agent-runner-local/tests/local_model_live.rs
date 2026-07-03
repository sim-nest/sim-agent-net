#[cfg(feature = "native-inference")]
use std::sync::Arc;

#[cfg(feature = "native-inference")]
use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Expr};
#[cfg(feature = "native-inference")]
use sim_lib_agent_runner_core::{ModelRequest, ModelRunner};
#[cfg(feature = "native-inference")]
use sim_lib_agent_runner_local::LocalModelBackend;

#[cfg(feature = "native-inference")]
fn smoke_cx() -> Cx {
    Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory))
}

#[cfg(feature = "native-inference")]
fn model_request_typed(task: &str) -> ModelRequest {
    ModelRequest::new(Expr::String(task.to_owned()), Vec::new())
}

#[test]
#[ignore = "requires SIM_LOCAL_MODEL_SMOKE=1 and native-inference"]
fn local_model_smoke_returns_non_empty_response() {
    if std::env::var("SIM_LOCAL_MODEL_SMOKE").as_deref() != Ok("1") {
        eprintln!("set SIM_LOCAL_MODEL_SMOKE=1 to run the local model smoke");
        return;
    }

    #[cfg(feature = "native-inference")]
    {
        let backend = LocalModelBackend::new();
        let response = backend
            .infer(
                &mut smoke_cx(),
                model_request_typed("Reply with: sim-local-ok"),
            )
            .unwrap();
        assert!(!response.content.is_empty());
    }

    #[cfg(not(feature = "native-inference"))]
    panic!("enable the native-inference feature to run the local model smoke");
}
