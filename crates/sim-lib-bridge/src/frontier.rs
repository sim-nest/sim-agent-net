use std::sync::Arc;

use sim_codec_bridge::{BridgeBook, BridgeFramePayload, BridgePacket};
use sim_kernel::{Cx, Expr, Result, Symbol};
use sim_lib_agent_runner_core::shape_to_grammar;
use sim_shape::{ExactExprShape, OneOfShape, Shape};
use sim_value::build::entry;

/// Legal next heads plus in-scope slot shapes for a BRIDGE packet.
pub struct FrontierMenu {
    /// Slot id to shape descriptor.
    pub slots: Vec<(String, Expr)>,
    /// Flat `OneOf` descriptor for legal next move heads.
    pub heads: Expr,
    /// Shape object backing the legal next move heads.
    pub head_shape: Arc<dyn Shape>,
    /// Grammar lowered from the same flat `OneOf`.
    pub grammar: String,
}

/// Computes the shared BRIDGE frontier for BRIEF, ASK, LOOM, and COLLAB views.
pub fn frontier(_cx: &mut Cx, packet: &BridgePacket) -> Result<FrontierMenu> {
    let book = BridgeBook::standard();
    let slots = frame_slots(&book, packet)?;
    let heads = book
        .moves
        .legal_reply_intents(std::slice::from_ref(&packet.header.move_kind));
    let head_shape = OneOfShape::new(
        heads
            .iter()
            .map(|head| Arc::new(ExactExprShape::new(Expr::Symbol(head.clone()))) as Arc<dyn Shape>)
            .collect(),
    );
    let grammar = shape_to_grammar(&head_shape)?;
    Ok(FrontierMenu {
        slots,
        heads: Expr::Map(vec![
            entry("shape", Expr::Symbol(Symbol::qualified("shape", "OneOf"))),
            entry(
                "choices",
                Expr::Vector(heads.into_iter().map(Expr::Symbol).collect()),
            ),
        ]),
        head_shape: Arc::new(head_shape),
        grammar,
    })
}

fn frame_slots(book: &BridgeBook, packet: &BridgePacket) -> Result<Vec<(String, Expr)>> {
    let mut slots = Vec::new();
    for part in &packet.body {
        if part.kind != Symbol::qualified("bridge", "Frame") {
            continue;
        }
        let payload = BridgeFramePayload::from_expr(&part.payload)?;
        let spec = book.frames.require_spec(&payload.frame)?;
        for hole in &spec.holes {
            slots.push((
                format!(
                    "{}.{}",
                    part.id.as_qualified_str(),
                    hole.name.as_qualified_str()
                ),
                hole.kind.shape_expr(),
            ));
        }
    }
    Ok(slots)
}
