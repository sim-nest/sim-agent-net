use crate::{ModelRequest, ModelResponse, ProcessRunner};
use sim_kernel::{
    CapabilityName, Cx, Datum, DatumStore, Effect, Expr, Ref, Result, Symbol, core_any_ref, effect,
    value_from_ref,
};

/// Returns the capability gating subprocess execution by a [`ProcessRunner`].
///
/// Spawning `/bin/sh -c <command>` is an arbitrary host-exec operation, so the
/// runner demands this capability before reaching the shell, mirroring the way
/// sibling crates gate their host effects with an explicit `cx.require(...)`.
pub fn host_process_capability() -> CapabilityName {
    CapabilityName::new("exec")
}

pub(super) fn resolve_process_effect<F>(
    runner: &ProcessRunner,
    cx: &mut Cx,
    request: ModelRequest,
    perform: F,
) -> Result<ModelResponse>
where
    F: FnOnce(&ProcessRunner, ModelRequest) -> Result<ModelResponse>,
{
    let required_capability = process_effect_capability(cx);
    let effect = process_effect(runner, cx, &request, required_capability)?;
    let result = effect::resolve_effect(cx, effect, |cx, _effect| {
        let response = perform(runner, request)?;
        response_ref(cx, response)
    })?;
    response_from_ref(cx, &result)
}

fn process_effect(
    runner: &ProcessRunner,
    cx: &mut Cx,
    request: &ModelRequest,
    required_capability: CapabilityName,
) -> Result<Effect> {
    let input = Datum::Node {
        tag: Symbol::qualified("agent", "ProcessRunnerInput"),
        fields: vec![
            (Symbol::new("runner"), Datum::Symbol(runner.runner.clone())),
            (Symbol::new("model"), Datum::String(runner.model.clone())),
            (
                Symbol::new("request"),
                Datum::try_from(Expr::from(request.clone()))?,
            ),
        ],
    };
    let input = Ref::Content(cx.datum_store_mut().intern(input)?);
    Effect::new(
        host_process_effect_kind(),
        Ref::Symbol(runner.runner.clone()),
        input,
        core_any_ref(),
        effect::effect_resume_op_key(),
        effect::effect_abort_op_key(),
    )
    .with_replay_key(Some(Ref::Symbol(Symbol::qualified(
        "agent",
        "process-runner-v1",
    ))))
    .map(|effect| effect.requiring(required_capability))
}

fn host_process_effect_kind() -> Symbol {
    Symbol::qualified("effect", "host-process")
}

fn process_effect_capability(cx: &Cx) -> CapabilityName {
    granted_capability_or_alias(cx, host_process_capability(), exec_aliases())
        .unwrap_or_else(host_process_capability)
}

fn exec_aliases() -> &'static [&'static str] {
    &["host.process"]
}

fn granted_capability_or_alias(
    cx: &Cx,
    canonical: CapabilityName,
    aliases: &'static [&'static str],
) -> Option<CapabilityName> {
    if cx.capabilities().contains(&canonical) {
        return Some(canonical);
    }
    aliases
        .iter()
        .copied()
        .map(CapabilityName::new)
        .find(|alias| cx.capabilities().contains(alias))
}

fn response_ref(cx: &mut Cx, response: ModelResponse) -> Result<Ref> {
    Ok(Ref::Content(
        cx.datum_store_mut()
            .intern(Datum::try_from(Expr::from(response))?)?,
    ))
}

fn response_from_ref(cx: &mut Cx, reference: &Ref) -> Result<ModelResponse> {
    ModelResponse::try_from(value_from_ref(cx, reference)?.object().as_expr(cx)?)
}
