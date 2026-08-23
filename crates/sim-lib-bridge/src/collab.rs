use std::sync::Arc;

use sim_codec_bridge::{
    BridgeBook, BridgePacket, BridgePatchPayload, BridgeVotePayload, stamp_packet_cid,
};
use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Error, Result, Symbol};

use crate::parent::parents_contain_cid;
use crate::rx_check;

/// Declared policy for combining collaboration contributions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergePolicy {
    /// Accept one patch contribution.
    Single,
    /// Accept one patch contribution once enough vote records target it.
    Quorum {
        /// Minimum number of matching vote records.
        min_votes: u32,
    },
    /// Accept a synthesizer patch contribution once enough vote records target it.
    SynthesisThenVote {
        /// Seat name of the synthesizer whose patch is eligible.
        synthesizer: String,
        /// Minimum number of matching vote records.
        min_votes: u32,
    },
}

/// Merge patch replies by exact parent content id and target path.
///
/// The selected packet is checked as a reply to `base`, so the root packet's
/// `Return` contract still decides whether the merged payload is admissible.
pub fn merge_bridge_replies(
    base: &BridgePacket,
    replies: &[BridgePacket],
    policy: &MergePolicy,
) -> Result<BridgePacket> {
    let base_cid = base.header.cid.as_deref().ok_or_else(|| {
        Error::Eval("BRIDGE collaboration merge requires a stamped base packet".to_owned())
    })?;
    let patches = patch_candidates(base_cid, replies)?;
    if patches.is_empty() {
        return Err(Error::Eval(
            "BRIDGE collaboration merge found no patch for the base packet".to_owned(),
        ));
    }
    require_one_target(&patches)?;
    let selected = select_patch(base_cid, replies, &patches, policy)?;
    let merged = stamp_packet_cid(&selected.packet.canonicalized())?;
    check_merged_reply(base, &merged)?;
    Ok(merged)
}

#[derive(Clone)]
struct PatchCandidate<'a> {
    packet: &'a BridgePacket,
    patch: BridgePatchPayload,
}

fn patch_candidates<'a>(
    base_cid: &str,
    replies: &'a [BridgePacket],
) -> Result<Vec<PatchCandidate<'a>>> {
    let mut patches = Vec::new();
    for packet in replies {
        if !parents_contain_cid(&packet.header.parents, base_cid) {
            continue;
        }
        for part in &packet.body {
            if part.kind != Symbol::qualified("bridge", "Patch") {
                continue;
            }
            let patch = BridgePatchPayload::from_expr(&part.payload)?;
            if patch.parent_cid == base_cid {
                patches.push(PatchCandidate { packet, patch });
            }
        }
    }
    Ok(patches)
}

fn require_one_target(patches: &[PatchCandidate<'_>]) -> Result<()> {
    let target = patches[0].patch.target.as_str();
    if patches
        .iter()
        .all(|candidate| candidate.patch.target == target)
    {
        return Ok(());
    }
    Err(Error::Eval(
        "BRIDGE collaboration merge patches must share one exact target path".to_owned(),
    ))
}

fn select_patch<'a>(
    base_cid: &str,
    replies: &'a [BridgePacket],
    patches: &[PatchCandidate<'a>],
    policy: &MergePolicy,
) -> Result<PatchCandidate<'a>> {
    match policy {
        MergePolicy::Single => {
            if patches.len() == 1 {
                Ok(patches[0].clone())
            } else {
                Err(Error::Eval(
                    "BRIDGE single merge requires exactly one patch".to_owned(),
                ))
            }
        }
        MergePolicy::Quorum { min_votes } => {
            select_quorum_patch(base_cid, replies, patches, *min_votes)
        }
        MergePolicy::SynthesisThenVote {
            synthesizer,
            min_votes,
        } => {
            if synthesizer.is_empty() {
                return Err(Error::Eval(
                    "BRIDGE synthesis merge requires a synthesizer seat".to_owned(),
                ));
            }
            let synthesized = patches
                .iter()
                .filter(|candidate| candidate.packet.header.from == *synthesizer)
                .cloned()
                .collect::<Vec<_>>();
            select_quorum_patch(base_cid, replies, &synthesized, *min_votes)
        }
    }
}

fn select_quorum_patch<'a>(
    base_cid: &str,
    replies: &[BridgePacket],
    patches: &[PatchCandidate<'a>],
    min_votes: u32,
) -> Result<PatchCandidate<'a>> {
    if min_votes == 0 {
        return Err(Error::Eval(
            "BRIDGE quorum merge requires at least one vote".to_owned(),
        ));
    }
    let eligible = patches
        .iter()
        .filter(|candidate| {
            votes_for_target(base_cid, replies, &candidate.patch.target)
                .map(|votes| votes >= min_votes)
                .unwrap_or(false)
        })
        .cloned()
        .collect::<Vec<_>>();
    if eligible.len() == 1 {
        Ok(eligible[0].clone())
    } else if eligible.is_empty() {
        Err(Error::Eval(
            "BRIDGE quorum merge found no patch with enough votes".to_owned(),
        ))
    } else {
        Err(Error::Eval(
            "BRIDGE quorum merge selected more than one patch".to_owned(),
        ))
    }
}

fn votes_for_target(base_cid: &str, replies: &[BridgePacket], target: &str) -> Result<u32> {
    let mut votes = 0u32;
    for packet in replies {
        if !parents_contain_cid(&packet.header.parents, base_cid) {
            continue;
        }
        for part in &packet.body {
            if part.kind != Symbol::qualified("bridge", "Vote") {
                continue;
            }
            let vote = BridgeVotePayload::from_expr(&part.payload)?;
            if vote.target == target {
                votes += 1;
            }
        }
    }
    Ok(votes)
}

fn check_merged_reply(base: &BridgePacket, merged: &BridgePacket) -> Result<()> {
    let book = BridgeBook::standard();
    let mut cx = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(1),
    );
    let report = rx_check(&mut cx, &book, merged, Some(base))?;
    if report.accepted() {
        return Ok(());
    }
    Err(Error::Eval(format!(
        "BRIDGE merged reply failed receive check: {:?}",
        report.obligations
    )))
}
