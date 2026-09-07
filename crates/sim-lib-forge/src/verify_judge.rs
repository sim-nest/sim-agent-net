use sim_codec_bridge::{BridgeBook, BridgePacket, BridgeVotePayload};
use sim_kernel::{Cx, Result, Symbol};
use sim_lib_bridge::rx_check;

pub(super) fn check_judge(
    cx: &mut Cx,
    seat: &str,
    packet: &BridgePacket,
    reply_to: Option<&BridgePacket>,
    target: &str,
    min_votes: u32,
) -> Result<std::result::Result<(), String>> {
    if min_votes == 0 {
        return Ok(Err("judge quorum must require at least one vote".to_owned()));
    }
    if packet.header.from != seat {
        return Ok(Err(format!(
            "judge packet came from {}, expected {seat}",
            packet.header.from
        )));
    }
    let report = rx_check(cx, &BridgeBook::standard(), packet, reply_to)?;
    if !report.accepted() {
        return Ok(Err(format!(
            "judge packet failed BRIDGE rx_check: {:?}",
            report.obligations
        )));
    }

    let mut votes = 0u32;
    for part in &packet.body {
        if part.kind != Symbol::qualified("bridge", "Vote") {
            continue;
        }
        let vote = BridgeVotePayload::from_expr(&part.payload)?;
        if vote.target == target && vote.scores.iter().any(|score| score.value > 0) {
            votes = votes.saturating_add(1);
        }
    }

    if votes >= min_votes {
        Ok(Ok(()))
    } else {
        Ok(Err(format!(
            "judge quorum for {target} has {votes} vote(s), needs {min_votes}"
        )))
    }
}
