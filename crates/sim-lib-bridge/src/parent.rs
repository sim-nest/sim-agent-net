use sim_codec_bridge::BridgePacket;

pub(crate) fn parent_token(parent: &BridgePacket) -> Option<String> {
    parent
        .header
        .cid
        .as_ref()
        .map(|cid| format!("{cid}#move={}", parent.header.move_kind.as_qualified_str()))
}

pub(crate) fn parent_cid(token: &str) -> &str {
    token
        .split_once("#move=")
        .map(|(cid, _move_kind)| cid)
        .unwrap_or(token)
}

pub(crate) fn parents_contain_cid(parents: &[String], cid: &str) -> bool {
    parents.iter().any(|parent| parent_cid(parent) == cid)
}
