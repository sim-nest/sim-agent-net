use super::radar::{RadarChunk, RadarIndex, RadarQuery, SourceSpan, retrieve_radar_hints};

#[test]
fn radar_ranks_matching_operation_first() {
    let index = RadarIndex {
        chunks: vec![
            chunk(
                "validation",
                "Validation command",
                "sim-tooling",
                "src/lib.rs",
                "signature: fn validate_workspace docs: run cargo validation command",
                &["validation"],
            ),
            chunk(
                "fruit",
                "Smoothie recipe",
                "sim-tooling",
                "README.md",
                "banana mango smoothie",
                &[],
            ),
        ],
    };

    let report = retrieve_radar_hints(&index, &RadarQuery::new("validation command")).unwrap();

    assert_eq!(report.hints[0].chunk_id, "validation");
    assert!((0.0..=1.0).contains(&report.hints[0].confidence));
    assert!(!report.stale_index);
}

#[test]
fn radar_capability_filter_excludes_non_matching_chunks() {
    let index = RadarIndex {
        chunks: vec![
            chunk(
                "validation",
                "Validation",
                "sim-tooling",
                "src/lib.rs",
                "cargo test command",
                &["validation"],
            ),
            chunk(
                "codec",
                "Codec",
                "sim-codecs",
                "src/lib.rs",
                "json codec round trip",
                &["codec"],
            ),
        ],
    };
    let mut query = RadarQuery::new("command codec");
    query.capability = Some("validation".to_owned());

    let report = retrieve_radar_hints(&index, &query).unwrap();

    assert_eq!(report.hints.len(), 1);
    assert_eq!(report.hints[0].chunk_id, "validation");
}

#[test]
fn radar_drops_stale_chunks_and_flags_index() {
    let mut stale = chunk(
        "stale",
        "Stale",
        "sim-tooling",
        "missing.rs",
        "validation command",
        &["validation"],
    );
    stale.live = false;
    let index = RadarIndex {
        chunks: vec![stale],
    };

    let report = retrieve_radar_hints(&index, &RadarQuery::new("validation")).unwrap();

    assert!(report.hints.is_empty());
    assert!(report.stale_index);
    assert_eq!(report.stale_chunk_ids, vec!["stale"]);
}

fn chunk(
    id: &str,
    title: &str,
    repo: &str,
    path: &str,
    text: &str,
    capabilities: &[&str],
) -> RadarChunk {
    let mut chunk = RadarChunk::new(
        id,
        title,
        SourceSpan {
            repo: repo.to_owned(),
            path: path.to_owned(),
            line: 1,
        },
        "rust-fn",
        text,
    );
    chunk.capabilities = capabilities
        .iter()
        .map(|value| (*value).to_owned())
        .collect();
    chunk
}
