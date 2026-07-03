use sim_kernel::{Callable, Error, eval_fabric_capability};

use crate::OpenAiGatewayFunction;

#[test]
fn fabric_function_requires_eval_fabric_capability() {
    let mut cx = super::cx();

    let err = OpenAiGatewayFunction::fabric()
        .call(&mut cx, sim_kernel::Args::new(Vec::new()))
        .unwrap_err();
    assert!(matches!(
        err,
        Error::CapabilityDenied { capability } if capability == eval_fabric_capability()
    ));

    cx.grant(eval_fabric_capability());
    let value = OpenAiGatewayFunction::fabric()
        .call(&mut cx, sim_kernel::Args::new(Vec::new()))
        .unwrap();
    assert!(value.object().as_eval_fabric().is_some());
}
