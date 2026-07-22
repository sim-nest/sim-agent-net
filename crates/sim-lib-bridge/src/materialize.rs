use sim_codec_bridge::{BridgePacket, BridgePart};
use sim_kernel::{CapabilityName, Cx, Error, Expr, Result, Symbol};

use crate::report::BridgeObligation;
use crate::rx::effective_caps;

/// Materialized data returned from a `Given` part under an explicit budget.
#[derive(Clone, Debug, PartialEq)]
pub struct GivenMaterialization {
    /// Part id that was materialized.
    pub id: Symbol,
    /// Materialized expression payload.
    pub payload: Expr,
    /// Budget remaining after this materialization.
    pub remaining_budget: usize,
}

/// Capability required to materialize a `Given` payload.
pub fn bridge_given_materialize_capability() -> CapabilityName {
    CapabilityName::new("bridge/given.materialize")
}

/// Capability required to expand context through a typed `Fetch` request.
pub fn bridge_fetch_capability() -> CapabilityName {
    CapabilityName::new("bridge/fetch")
}

/// Materializes one `Given` part. Callers must opt in and supply a budget.
pub fn materialize_given(
    cx: &mut Cx,
    packet: &BridgePacket,
    id: &Symbol,
    budget: usize,
) -> Result<GivenMaterialization> {
    if budget == 0 {
        return Err(Error::Eval(
            "BRIDGE Given materialization budget exhausted".to_owned(),
        ));
    }
    let caps = effective_caps(cx, packet)?;
    if !caps.contains(&bridge_given_materialize_capability()) {
        return Err(Error::CapabilityDenied {
            capability: bridge_given_materialize_capability(),
        });
    }
    let part = part_by_id(packet, id).ok_or_else(|| {
        Error::Eval(format!(
            "BRIDGE Given materialization missing part {}",
            id.as_qualified_str()
        ))
    })?;
    if part.kind != Symbol::qualified("bridge", "Given") {
        return Err(Error::Eval(format!(
            "BRIDGE materialization expects bridge/Given, found {}",
            part.kind
        )));
    }
    Ok(GivenMaterialization {
        id: part.id.clone(),
        payload: part.payload.clone(),
        remaining_budget: budget - 1,
    })
}

/// Builds the typed obligation used when context must be fetched explicitly.
pub fn fetch_obligation(path: impl Into<String>, expected: impl Into<String>) -> BridgeObligation {
    BridgeObligation::new(
        path,
        "typed context expansion requires Fetch",
        expected,
        "missing context",
        vec![
            "send Fetch packet".to_owned(),
            "remove context reference".to_owned(),
        ],
    )
}

fn part_by_id<'a>(packet: &'a BridgePacket, id: &Symbol) -> Option<&'a BridgePart> {
    packet.body.iter().find(|part| &part.id == id)
}
