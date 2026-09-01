// conformance: immutable source-deck grounding and exact declaration lookup.

use sim_source_deck::*;

struct FixtureDecoder;
impl FragmentDecoder for FixtureDecoder {
    fn decode(&self, _: &[u8]) -> Result<IndexFragment, Failure> {
        Ok(IndexFragment {
            owner: "sim-source-deck".into(),
            anchors: vec![IndexAnchor {
                id: "anchor/rustdoc/sim-source-deck/source-deck".into(),
                owner: "sim-source-deck".into(),
                source_path: Some("src/lib.rs".into()),
            }],
            specimens: vec![IndexSpecimen {
                id: "spec-test/sim-source-deck/grounded-declaration".into(),
                owner: "sim-source-deck".into(),
            }],
        })
    }
}

fn main() -> Result<(), Failure> {
    let fragment = b"fixture-index-fragment".to_vec();
    let source = b"pub struct SourceDeck { /* private fields */ }".to_vec();
    let test = b"assert exact grounding".to_vec();
    let fragment_id = ByteContentId::of(&fragment)?;
    let mut certificate = ClaimCertificate {
        anchor: "anchor/rustdoc/sim-source-deck/source-deck".into(),
        owner: "sim-source-deck".into(),
        fragment_id: fragment_id.clone(),
        digest: ByteContentId::of(b"pending")?,
    };
    certificate.digest = certificate.expected_digest()?;
    let deck = build(DeckInput {
        repositories: vec![RepositorySnapshot {
            owner: "sim-source-deck".into(),
            repository: "https://github.com/sim-nest/sim-agent-net".into(),
            revision: "fixture-v1".into(),
        }],
        fragments: vec![FragmentPin {
            owner: "sim-source-deck".into(),
            bytes: fragment,
            content_id: fragment_id,
        }],
        certificates: vec![certificate],
        files: vec![SourceFile {
            owner: "sim-source-deck".into(),
            path: "src/lib.rs".into(),
            content_id: ByteContentId::of(&source)?,
            bytes: source.clone(),
        }],
        excerpts: vec![Excerpt {
            id: "private-fields".into(),
            owner: "sim-source-deck".into(),
            path: "src/lib.rs".into(),
            start: 0,
            end: source.len(),
            bytes: source,
        }],
        specimens: vec![SpecimenPin {
            id: "spec-test/sim-source-deck/grounded-declaration".into(),
            owner: "sim-source-deck".into(),
            content_id: ByteContentId::of(&test)?,
            bytes: test,
        }],
        queries: vec![
            SourceQuery::Anchor("anchor/rustdoc/sim-source-deck/source-deck".into()),
            SourceQuery::Excerpt("private-fields".into()),
            SourceQuery::Specimen("spec-test/sim-source-deck/grounded-declaration".into()),
        ],
        limitations: vec![Limitation::SyntaxBound {
            language: "rust".into(),
            detail: "fixture is byte-exact; no scanner evidence supplied".into(),
        }],
        limits: DeckLimits::strict(1, 1, 1, 1, 1, 3, 1024),
        decoder: &FixtureDecoder,
    })?;
    assert_eq!(deck.evidence().len(), 3);
    println!("{:?}", deck.id());
    Ok(())
}
