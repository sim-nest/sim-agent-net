use std::collections::{BTreeMap, BTreeSet};

use sim_codec_bridge::{
    BridgeBook, BridgePacket, BridgeWarrant, BridgeWarrantPolicy, content_id_string,
    frame_book_content_id, move_book_content_id, part_spec_content_id,
};
use sim_kernel::{ContentId, Cx, Result, Symbol};

use crate::materialize::fetch_obligation;
use crate::report::BridgeObligation;

/// Verifies a packet warrant against the local bridge book.
///
/// A warrant miss is a Fetch obligation, never a hard fail or silent accept.
pub fn verify_warrant(
    _cx: &mut Cx,
    book: &BridgeBook,
    packet: &BridgePacket,
) -> Result<Vec<BridgeObligation>> {
    if book.warrant_policy == BridgeWarrantPolicy::SharedTrust {
        return Ok(Vec::new());
    }
    let Some(warrant) = &packet.warrant else {
        return Ok(vec![fetch_obligation("warrant", "bridge/Warrant")]);
    };

    let mut obligations = Vec::new();
    check_book_id(
        &mut obligations,
        "warrant/moves",
        &warrant.moves,
        &move_book_content_id(&book.moves)?,
    );
    check_book_id(
        &mut obligations,
        "warrant/frames",
        &warrant.frames,
        &frame_book_content_id(&book.frames)?,
    );
    check_part_ids(&mut obligations, book, packet, warrant)?;
    Ok(obligations)
}

fn check_book_id(
    obligations: &mut Vec<BridgeObligation>,
    path: &str,
    declared: &ContentId,
    local: &ContentId,
) {
    if declared != local {
        obligations.push(fetch_content_id(path, declared));
    }
}

fn check_part_ids(
    obligations: &mut Vec<BridgeObligation>,
    book: &BridgeBook,
    packet: &BridgePacket,
    warrant: &BridgeWarrant,
) -> Result<()> {
    let declared = warrant
        .parts
        .iter()
        .map(|(kind, cid)| (kind.clone(), cid))
        .collect::<BTreeMap<Symbol, &ContentId>>();
    let mut seen = BTreeSet::new();
    for (kind, declared_cid) in &warrant.parts {
        let path = part_path(kind);
        if !seen.insert(kind.clone()) {
            obligations.push(fetch_content_id(path, declared_cid));
        }
    }

    let mut used = BTreeSet::new();
    for part in &packet.body {
        if !used.insert(part.kind.clone()) {
            continue;
        }
        let path = part_path(&part.kind);
        let Some(declared_cid) = declared.get(&part.kind).copied() else {
            obligations.push(fetch_missing_part(&path, &part.kind));
            continue;
        };
        let Some(spec) = book.parts.spec(&part.kind) else {
            obligations.push(fetch_content_id(path, declared_cid));
            continue;
        };
        let local = part_spec_content_id(spec)?;
        if declared_cid != &local {
            obligations.push(fetch_content_id(path, declared_cid));
        }
    }

    for (kind, declared_cid) in &warrant.parts {
        if !used.contains(kind) {
            obligations.push(fetch_content_id(part_path(kind), declared_cid));
        }
    }
    Ok(())
}

fn fetch_missing_part(path: &str, kind: &Symbol) -> BridgeObligation {
    fetch_obligation(
        path.to_owned(),
        format!("bridge/Fetch part spec {}", kind.as_qualified_str()),
    )
}

fn fetch_content_id(path: impl Into<String>, id: &ContentId) -> BridgeObligation {
    fetch_obligation(path, format!("bridge/Fetch {}", content_id_string(id)))
}

fn part_path(kind: &Symbol) -> String {
    format!("warrant/parts/{}", kind.as_qualified_str())
}
