use super::*;

struct Decoder;
impl FragmentDecoder for Decoder {
    fn decode(&self, bytes: &[u8]) -> Result<IndexFragment, Failure> {
        match bytes {
            b"fragment-a" => Ok(fragment("owner", "src/lib.rs")),
            b"fragment-b" => Ok(fragment("owner", "src/other.rs")),
            _ => Err(Failure::Decode("fixture".into())),
        }
    }
}
fn fragment(owner: &str, path: &str) -> IndexFragment {
    IndexFragment {
        owner: owner.into(),
        anchors: vec![IndexAnchor {
            id: "anchor/rustdoc/public-declaration".into(),
            owner: owner.into(),
            source_path: Some(path.into()),
        }],
        specimens: vec![IndexSpecimen {
            id: "spec-test/owner/public-declaration".into(),
            owner: owner.into(),
        }],
    }
}
fn fixture(fragment_bytes: &[u8], source: &[u8]) -> DeckInput<'static> {
    let fid = ByteContentId::of(fragment_bytes).unwrap();
    let mut cert = ClaimCertificate {
        anchor: "anchor/rustdoc/public-declaration".into(),
        owner: "owner".into(),
        fragment_id: fid.clone(),
        digest: ByteContentId::of(b"placeholder").unwrap(),
    };
    cert.digest = cert.expected_digest().unwrap();
    DeckInput {
        repositories: vec![RepositorySnapshot {
            owner: "owner".into(),
            repository: "https://example.invalid/owner".into(),
            revision: "abc123".into(),
        }],
        fragments: vec![FragmentPin {
            owner: "owner".into(),
            bytes: fragment_bytes.into(),
            content_id: fid,
        }],
        certificates: vec![cert],
        files: vec![SourceFile {
            owner: "owner".into(),
            path: "src/lib.rs".into(),
            bytes: source.into(),
            content_id: ByteContentId::of(source).unwrap(),
        }],
        excerpts: vec![Excerpt {
            id: "private-body".into(),
            owner: "owner".into(),
            path: "src/lib.rs".into(),
            start: 0,
            end: source.len(),
            bytes: source.into(),
        }],
        specimens: vec![SpecimenPin {
            id: "spec-test/owner/public-declaration".into(),
            owner: "owner".into(),
            bytes: b"assert-public".into(),
            content_id: ByteContentId::of(b"assert-public").unwrap(),
        }],
        queries: vec![
            SourceQuery::Anchor("anchor/rustdoc/public-declaration".into()),
            SourceQuery::Excerpt("private-body".into()),
            SourceQuery::Specimen("spec-test/owner/public-declaration".into()),
        ],
        limitations: vec![Limitation::SyntaxBound {
            language: "rust".into(),
            detail: "fixture supplies bytes, not an AST".into(),
        }],
        limits: DeckLimits::strict(1, 1, 1, 1, 1, 3, 4096),
        decoder: &Decoder,
    }
}

#[test]
fn insertion_order_does_not_change_identity() {
    let mut a = fixture(b"fragment-a", b"pub fn declaration() {}");
    let mut b = fixture(b"fragment-a", b"pub fn declaration() {}");
    a.queries.reverse();
    b.repositories.reverse();
    assert_eq!(build(a).unwrap().id(), build(b).unwrap().id());
}
#[test]
fn every_source_byte_is_identity_relevant() {
    let a = build(fixture(b"fragment-a", b"abcdef")).unwrap();
    let b = build(fixture(b"fragment-a", b"abcdeg")).unwrap();
    assert_ne!(a.id(), b.id());
}
#[test]
fn stale_fragment_bytes_fail_before_grounding() {
    let mut v = fixture(b"fragment-a", b"source");
    v.fragments[0].bytes[0] ^= 1;
    assert!(matches!(build(v), Err(Failure::ContentMismatch(_))));
}
#[test]
fn swapped_fragment_is_rejected() {
    let mut v = fixture(b"fragment-a", b"source");
    v.fragments[0].bytes = b"fragment-b".to_vec();
    v.fragments[0].content_id = ByteContentId::of(b"fragment-b").unwrap();
    assert!(matches!(build(v), Err(Failure::Substituted(_))));
}
#[test]
fn excerpt_forgery_is_rejected() {
    let mut v = fixture(b"fragment-a", b"source");
    v.excerpts[0].bytes[0] ^= 1;
    assert!(matches!(build(v), Err(Failure::ExcerptForgery(_))));
}
#[test]
fn path_escape_is_rejected() {
    let mut v = fixture(b"fragment-a", b"source");
    v.files[0].path = "../secret".into();
    assert!(matches!(build(v), Err(Failure::InvalidPath(_))));
}
#[test]
fn duplicate_collision_is_rejected() {
    let mut v = fixture(b"fragment-a", b"source");
    v.files.push(v.files[0].clone());
    v.limits.files = 2;
    assert!(matches!(
        build(v),
        Err(Failure::Duplicate { kind: "file", .. })
    ));
}
#[test]
fn truncation_is_typed() {
    let mut v = fixture(b"fragment-a", b"source");
    v.excerpts[0].end = 99;
    assert!(matches!(build(v), Err(Failure::Truncated { .. })));
}
#[test]
fn hostile_input_hits_bound_before_decode() {
    let mut v = fixture(b"fragment-a", b"source");
    v.fragments[0].bytes = vec![0; 4097];
    assert!(matches!(
        build(v),
        Err(Failure::OverLimit {
            limit: "total-bytes",
            ..
        })
    ));
}
#[test]
fn certificate_digest_mismatch_is_rejected() {
    let mut v = fixture(b"fragment-a", b"source");
    v.certificates[0].digest = ByteContentId::of(b"forged").unwrap();
    assert!(matches!(
        build(v),
        Err(Failure::CertificateDigestMismatch(_))
    ));
}

#[test]
fn public_qualification_corpus_tracks_changes_moves_truncation_and_private_witnesses() {
    let a = include_bytes!("../../sim-lib-roadmap/qualification/source/revision-a.rs");
    let b = include_bytes!("../../sim-lib-roadmap/qualification/source/revision-b.rs");
    let truncated = include_bytes!("../../sim-lib-roadmap/qualification/source/truncated.rs");
    let collision = include_bytes!("../../sim-lib-roadmap/qualification/source/collision.rs");
    let specimen = include_bytes!("../../sim-lib-roadmap/qualification/source/specimen.txt");
    assert_ne!(ByteContentId::of(a).unwrap(), ByteContentId::of(b).unwrap());
    assert!(
        a.windows(b"fn private_helper".len())
            .any(|w| w == b"fn private_helper")
    );
    assert!(
        b.windows(b"pub mod moved".len())
            .any(|w| w == b"pub mod moved")
    );
    assert!(truncated.ends_with(b"->\n"));
    assert_eq!(
        collision
            .windows(b"pub fn public_api".len())
            .filter(|w| *w == b"pub fn public_api")
            .count(),
        2
    );
    assert!(
        specimen
            .windows(b"exact source excerpt".len())
            .any(|w| w == b"exact source excerpt")
    );
}

#[test]
fn formatting_changes_preserve_declared_evidence_but_semantic_bytes_invalidate_exactly() {
    let base = b"pub fn declaration() {}";
    let formatted = b"pub fn declaration() { }";
    let a = build(fixture(b"fragment-a", base)).unwrap();
    let b = build(fixture(b"fragment-a", formatted)).unwrap();
    assert_ne!(
        a.id(),
        b.id(),
        "exact source decks must invalidate on byte changes"
    );
    assert_eq!(a.evidence().len(), b.evidence().len());
    assert_eq!(a.limitations(), b.limitations());
}
