use sim_codec_bridge::{
    BridgeBook, BridgeFramePayload, BridgeHeader, BridgePacket, BridgePart, BridgeProvenance,
    render_frame_part_with_prose,
};
use sim_kernel::{Datum, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::InjectionFence;
use sim_value::build::entry;

/// Builds a BRIEF request packet from one typed frame.
pub fn bridge_brief(
    to: &str,
    frame: BridgeFramePayload,
    return_shape: Expr,
) -> Result<BridgePacket> {
    BridgeBook::standard()
        .frames
        .validate_payload(&frame.to_expr())?;
    Ok(BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("request"),
            from: "sim".to_owned(),
            to: vec![to.to_owned()],
            role: Symbol::new("implementer"),
            parents: Vec::new(),
            task: Symbol::new("T1"),
            output: Symbol::new("O1"),
            ceiling: vec![Symbol::qualified("ai", "run")],
            context: Vec::new(),
            provenance: BridgeProvenance::default(),
        },
        body: vec![
            BridgePart {
                id: Symbol::new("T1"),
                kind: Symbol::qualified("bridge", "Frame"),
                payload: frame.to_expr(),
            },
            BridgePart {
                id: Symbol::new("O1"),
                kind: Symbol::qualified("bridge", "Return"),
                payload: Expr::Map(vec![
                    entry("codec", Expr::Symbol(Symbol::qualified("codec", "bridge"))),
                    entry("shape", return_shape),
                ]),
            },
        ],
        warrant: None,
    })
}

/// Renders every `bridge/Frame` part as a cited BRIEF sentence.
pub fn render_brief_sentences(
    book: &BridgeBook,
    packet: &BridgePacket,
) -> Result<Vec<(String, String)>> {
    packet
        .body
        .iter()
        .filter(|part| part.kind == Symbol::qualified("bridge", "Frame"))
        .map(|part| {
            let part_id = part.id.as_qualified_str().to_owned();
            let rendered = render_frame_part_with_prose(book, part, |hole, value| {
                render_prose_hole(&part.id, hole, value)
            })?;
            Ok((part_id, rendered))
        })
        .collect()
}

fn render_prose_hole(part: &Symbol, hole: &Symbol, value: &Expr) -> Result<String> {
    let Expr::String(text) = value else {
        return Err(Error::Eval(format!(
            "BRIDGE prose hole {}.{} must be text",
            part, hole
        )));
    };
    let id = Datum::String(text.clone()).content_id()?;
    let fence = InjectionFence::for_content(&id);
    Ok(fence.wrap(
        &format!("{}.{}", part.as_qualified_str(), hole.as_qualified_str()),
        text,
    ))
}
