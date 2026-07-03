use std::sync::Arc;

use sim_kernel::{CapabilityName, Cx, Expr, Result, Symbol, Value};
use sim_lib_server::ServerAddress;

use crate::{
    ComponentKind, ModelRunner, components::AgentComponent, components::ComponentBackend,
    components::RunnerBackend, components::component_value, installed_codecs,
};

/// Metadata needed to project an existing [`ModelRunner`] as an agent runner.
pub struct ExternalRunnerSpec {
    /// Component symbol used by `runner/card`, `runner/market`, and `realize`.
    pub symbol: Symbol,
    /// Model id advertised by the component wrapper.
    pub model: String,
    /// Component-level capabilities checked before runner execution.
    pub capabilities: Vec<CapabilityName>,
    /// Extra component metadata exposed through the runner card.
    pub spec: Vec<(Symbol, Expr)>,
    /// The underlying provider-neutral runner implementation.
    pub runner: Arc<dyn ModelRunner>,
}

/// Wraps a provider-neutral [`ModelRunner`] in the existing agent runner value.
pub fn external_runner_value(cx: &mut Cx, spec: ExternalRunnerSpec) -> Result<Value> {
    let mut component_spec = vec![
        (
            Symbol::new("backend"),
            Expr::Symbol(Symbol::new("external")),
        ),
        (Symbol::new("model"), Expr::String(spec.model)),
    ];
    component_spec.extend(spec.spec);
    component_value(
        cx,
        AgentComponent {
            symbol: spec.symbol,
            kind: ComponentKind::Runner,
            capabilities: spec.capabilities,
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: component_spec,
            backend: ComponentBackend::Runner(RunnerBackend::External {
                runner: spec.runner,
            }),
        },
    )
}
