#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

use sim_kernel::{ContentId, Datum, Symbol};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

/// Stable identifier for a validated deck.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceDeckId(pub ContentId);

/// Stable identifier for caller-supplied bytes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteContentId(pub ContentId);

impl ByteContentId {
    /// Computes identity without interpreting the supplied bytes.
    pub fn of(bytes: &[u8]) -> Result<Self, Failure> {
        Ok(Self(
            Datum::Node {
                tag: tag("source-deck", "bytes-v1"),
                fields: vec![(Symbol::new("bytes"), Datum::Bytes(bytes.to_vec()))],
            }
            .content_id()
            .map_err(|e| Failure::Canonical(e.to_string()))?,
        ))
    }
}

/// An immutable repository revision named independently of a local checkout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositorySnapshot {
    pub owner: String,
    pub repository: String,
    pub revision: String,
}

/// Exact source file bytes within a repository snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFile {
    pub owner: String,
    pub path: String,
    pub bytes: Vec<u8>,
    pub content_id: ByteContentId,
}

/// A byte-range witness into a source file. The end is exclusive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Excerpt {
    pub id: String,
    pub owner: String,
    pub path: String,
    pub start: usize,
    pub end: usize,
    pub bytes: Vec<u8>,
}

/// An exact checked specimen supplied as bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecimenPin {
    pub id: String,
    pub owner: String,
    pub bytes: Vec<u8>,
    pub content_id: ByteContentId,
}

/// The relevant, codec-neutral projection of a SIM Index fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexFragment {
    pub owner: String,
    pub anchors: Vec<IndexAnchor>,
    pub specimens: Vec<IndexSpecimen>,
}

/// One exact Index anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexAnchor {
    pub id: String,
    pub owner: String,
    pub source_path: Option<String>,
}

/// One exact Index specimen record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexSpecimen {
    pub id: String,
    pub owner: String,
}

/// Immutable encoded fragment input and its expected identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentPin {
    pub owner: String,
    pub bytes: Vec<u8>,
    pub content_id: ByteContentId,
}

/// Caller adapter which decodes a fragment. Codec implementations belong outside this crate.
pub trait FragmentDecoder {
    fn decode(&self, bytes: &[u8]) -> Result<IndexFragment, Failure>;
}

/// Certifies that one and only one fragment row claims an anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimCertificate {
    pub anchor: String,
    pub owner: String,
    pub fragment_id: ByteContentId,
    pub digest: ByteContentId,
}

impl ClaimCertificate {
    /// Computes the expected certificate digest.
    pub fn expected_digest(&self) -> Result<ByteContentId, Failure> {
        let datum = Datum::Node {
            tag: tag("source-deck", "claim-certificate-v1"),
            fields: vec![
                field("anchor", &self.anchor),
                field("owner", &self.owner),
                cid_field("fragment", &self.fragment_id.0),
            ],
        };
        Ok(ByteContentId(
            datum
                .content_id()
                .map_err(|e| Failure::Canonical(e.to_string()))?,
        ))
    }
}

/// Explicit incompleteness retained in a deck.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Limitation {
    SyntaxBound {
        language: String,
        detail: String,
    },
    ScannerEvidence {
        scanner: String,
        detail: String,
    },
    Truncated {
        subject: String,
        available: usize,
        required: usize,
    },
}

/// Exact request for evidence. No fuzzy matching is available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceQuery {
    Anchor(String),
    Excerpt(String),
    Specimen(String),
}

/// A successfully grounded exact witness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroundedEvidence {
    Anchor(IndexAnchor),
    Excerpt(Excerpt),
    Specimen(SpecimenPin),
}

/// Caller-configured hard bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeckLimits {
    pub repositories: usize,
    pub fragments: usize,
    pub files: usize,
    pub excerpts: usize,
    pub specimens: usize,
    pub queries: usize,
    pub total_bytes: usize,
}

impl DeckLimits {
    pub const fn strict(
        repositories: usize,
        fragments: usize,
        files: usize,
        excerpts: usize,
        specimens: usize,
        queries: usize,
        total_bytes: usize,
    ) -> Self {
        Self {
            repositories,
            fragments,
            files,
            excerpts,
            specimens,
            queries,
            total_bytes,
        }
    }
}

/// Fully validated, immutable source context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDeck {
    id: SourceDeckId,
    repositories: Vec<RepositorySnapshot>,
    fragments: Vec<FragmentPin>,
    files: Vec<SourceFile>,
    evidence: Vec<GroundedEvidence>,
    limitations: Vec<Limitation>,
}

impl SourceDeck {
    pub fn id(&self) -> &SourceDeckId {
        &self.id
    }
    pub fn evidence(&self) -> &[GroundedEvidence] {
        &self.evidence
    }
    pub fn limitations(&self) -> &[Limitation] {
        &self.limitations
    }
    pub fn repositories(&self) -> &[RepositorySnapshot] {
        &self.repositories
    }
    pub fn fragments(&self) -> &[FragmentPin] {
        &self.fragments
    }
    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }
}

/// All caller-owned inputs needed to build a deck.
pub struct DeckInput<'a> {
    pub repositories: Vec<RepositorySnapshot>,
    pub fragments: Vec<FragmentPin>,
    pub certificates: Vec<ClaimCertificate>,
    pub files: Vec<SourceFile>,
    pub excerpts: Vec<Excerpt>,
    pub specimens: Vec<SpecimenPin>,
    pub queries: Vec<SourceQuery>,
    pub limitations: Vec<Limitation>,
    pub limits: DeckLimits,
    pub decoder: &'a dyn FragmentDecoder,
}

/// Typed fail-closed outcomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Unresolved {
        kind: &'static str,
        id: String,
    },
    Ambiguous {
        kind: &'static str,
        id: String,
    },
    Stale {
        subject: String,
    },
    Truncated {
        subject: String,
    },
    OverLimit {
        limit: &'static str,
        actual: usize,
        maximum: usize,
    },
    InvalidPath(String),
    OwnerMismatch {
        subject: String,
    },
    Duplicate {
        kind: &'static str,
        id: String,
    },
    DanglingAnchor(String),
    Unclaimed(String),
    MultiplyClaimed(String),
    Substituted(String),
    CertificateDigestMismatch(String),
    ContentMismatch(String),
    ExcerptForgery(String),
    MissingLimitation(String),
    Collision(String),
    Decode(String),
    Canonical(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Failure {}

/// Validates every input and mints the deck only after exact grounding succeeds.
pub fn build(input: DeckInput<'_>) -> Result<SourceDeck, Failure> {
    check_count(
        "repositories",
        input.repositories.len(),
        input.limits.repositories,
    )?;
    check_count("fragments", input.fragments.len(), input.limits.fragments)?;
    check_count("files", input.files.len(), input.limits.files)?;
    check_count("excerpts", input.excerpts.len(), input.limits.excerpts)?;
    check_count("specimens", input.specimens.len(), input.limits.specimens)?;
    check_count("queries", input.queries.len(), input.limits.queries)?;
    let total_bytes = input
        .fragments
        .iter()
        .map(|v| v.bytes.len())
        .chain(input.files.iter().map(|v| v.bytes.len()))
        .chain(input.specimens.iter().map(|v| v.bytes.len()))
        .sum();
    check_count("total-bytes", total_bytes, input.limits.total_bytes)?;

    unique(input.repositories.iter().map(|v| (&v.owner, "repository")))?;
    let repo_owners: BTreeSet<_> = input
        .repositories
        .iter()
        .map(|v| v.owner.as_str())
        .collect();
    for repo in &input.repositories {
        required(&repo.owner)?;
        required(&repo.repository)?;
        required(&repo.revision)?;
    }

    let mut decoded = Vec::new();
    for pin in &input.fragments {
        require_owner(&repo_owners, &pin.owner, "fragment")?;
        verify_content("fragment", &pin.bytes, &pin.content_id)?;
        let fragment = input.decoder.decode(&pin.bytes)?;
        if fragment.owner != pin.owner {
            return Err(Failure::OwnerMismatch {
                subject: pin.owner.clone(),
            });
        }
        decoded.push((pin, fragment));
    }
    let mut anchors: BTreeMap<&str, Vec<(&FragmentPin, &IndexAnchor)>> = BTreeMap::new();
    let mut index_specimens: BTreeMap<&str, Vec<&IndexSpecimen>> = BTreeMap::new();
    for (pin, fragment) in &decoded {
        for anchor in &fragment.anchors {
            if anchor.owner != fragment.owner {
                return Err(Failure::OwnerMismatch {
                    subject: anchor.id.clone(),
                });
            }
            if let Some(path) = &anchor.source_path {
                valid_path(path)?;
            }
            anchors.entry(&anchor.id).or_default().push((pin, anchor));
        }
        for specimen in &fragment.specimens {
            if specimen.owner != fragment.owner {
                return Err(Failure::OwnerMismatch {
                    subject: specimen.id.clone(),
                });
            }
            index_specimens
                .entry(&specimen.id)
                .or_default()
                .push(specimen);
        }
    }
    unique(
        input
            .certificates
            .iter()
            .map(|v| (&v.anchor, "certificate")),
    )?;
    for (id, rows) in &anchors {
        if rows.len() > 1 {
            return Err(Failure::MultiplyClaimed((*id).into()));
        }
        let certs: Vec<_> = input
            .certificates
            .iter()
            .filter(|v| v.anchor == *id)
            .collect();
        if certs.is_empty() {
            return Err(Failure::Unclaimed((*id).into()));
        }
        if certs.len() != 1 {
            return Err(Failure::MultiplyClaimed((*id).into()));
        }
        let (pin, anchor) = rows[0];
        let cert = certs[0];
        if cert.owner != anchor.owner || cert.fragment_id != pin.content_id {
            return Err(Failure::Substituted((*id).into()));
        }
        if cert.digest != cert.expected_digest()? {
            return Err(Failure::CertificateDigestMismatch((*id).into()));
        }
    }
    for cert in &input.certificates {
        if !anchors.contains_key(cert.anchor.as_str()) {
            return Err(Failure::DanglingAnchor(cert.anchor.clone()));
        }
    }

    let mut files = BTreeMap::new();
    for file in &input.files {
        require_owner(&repo_owners, &file.owner, &file.path)?;
        valid_path(&file.path)?;
        verify_content(&file.path, &file.bytes, &file.content_id)?;
        if files
            .insert((file.owner.as_str(), file.path.as_str()), file)
            .is_some()
        {
            return Err(Failure::Duplicate {
                kind: "file",
                id: file.path.clone(),
            });
        }
    }
    for rows in anchors.values() {
        for (_, anchor) in rows {
            if let Some(path) = &anchor.source_path
                && !files.contains_key(&(anchor.owner.as_str(), path.as_str()))
            {
                return Err(Failure::Unresolved {
                    kind: "anchor source",
                    id: format!("{}:{path}", anchor.owner),
                });
            }
        }
    }
    let mut excerpts = BTreeMap::new();
    for excerpt in &input.excerpts {
        valid_path(&excerpt.path)?;
        let file = files
            .get(&(excerpt.owner.as_str(), excerpt.path.as_str()))
            .ok_or_else(|| Failure::Unresolved {
                kind: "file",
                id: excerpt.path.clone(),
            })?;
        if excerpt.end < excerpt.start || excerpt.end > file.bytes.len() {
            return Err(Failure::Truncated {
                subject: excerpt.id.clone(),
            });
        }
        if file.bytes[excerpt.start..excerpt.end] != excerpt.bytes {
            return Err(Failure::ExcerptForgery(excerpt.id.clone()));
        }
        if excerpts.insert(excerpt.id.as_str(), excerpt).is_some() {
            return Err(Failure::Duplicate {
                kind: "excerpt",
                id: excerpt.id.clone(),
            });
        }
    }
    let mut specimens = BTreeMap::new();
    for specimen in &input.specimens {
        require_owner(&repo_owners, &specimen.owner, &specimen.id)?;
        verify_content(&specimen.id, &specimen.bytes, &specimen.content_id)?;
        match index_specimens.get(specimen.id.as_str()).map(Vec::as_slice) {
            None => {
                return Err(Failure::Unresolved {
                    kind: "specimen",
                    id: specimen.id.clone(),
                });
            }
            Some([row]) if row.owner == specimen.owner => {}
            Some([_]) => {
                return Err(Failure::OwnerMismatch {
                    subject: specimen.id.clone(),
                });
            }
            Some(_) => {
                return Err(Failure::Ambiguous {
                    kind: "specimen",
                    id: specimen.id.clone(),
                });
            }
        }
        if specimens.insert(specimen.id.as_str(), specimen).is_some() {
            return Err(Failure::Duplicate {
                kind: "specimen",
                id: specimen.id.clone(),
            });
        }
    }
    let mut evidence = Vec::new();
    for query in &input.queries {
        match query {
            SourceQuery::Anchor(id) => match anchors.get(id.as_str()).map(Vec::as_slice) {
                None => {
                    return Err(Failure::Unresolved {
                        kind: "anchor",
                        id: id.clone(),
                    });
                }
                Some([(_, row)]) => evidence.push(GroundedEvidence::Anchor((*row).clone())),
                Some(_) => {
                    return Err(Failure::Ambiguous {
                        kind: "anchor",
                        id: id.clone(),
                    });
                }
            },
            SourceQuery::Excerpt(id) => evidence.push(GroundedEvidence::Excerpt(
                (*excerpts
                    .get(id.as_str())
                    .ok_or_else(|| Failure::Unresolved {
                        kind: "excerpt",
                        id: id.clone(),
                    })?)
                .clone(),
            )),
            SourceQuery::Specimen(id) => evidence.push(GroundedEvidence::Specimen(
                (*specimens
                    .get(id.as_str())
                    .ok_or_else(|| Failure::Unresolved {
                        kind: "specimen",
                        id: id.clone(),
                    })?)
                .clone(),
            )),
        }
    }
    if input.limitations.iter().any(|v| matches!(v, Limitation::SyntaxBound { detail, .. } | Limitation::ScannerEvidence { detail, .. } if detail.is_empty())) { return Err(Failure::MissingLimitation("empty limitation evidence".into())); }

    let datum = canonical(
        &input.repositories,
        &input.fragments,
        &input.files,
        &evidence,
        &input.limitations,
    );
    let id = SourceDeckId(
        datum
            .content_id()
            .map_err(|e| Failure::Canonical(e.to_string()))?,
    );
    Ok(SourceDeck {
        id,
        repositories: input.repositories,
        fragments: input.fragments,
        files: input.files,
        evidence,
        limitations: input.limitations,
    })
}

fn canonical(
    repos: &[RepositorySnapshot],
    fragments: &[FragmentPin],
    files: &[SourceFile],
    evidence: &[GroundedEvidence],
    limitations: &[Limitation],
) -> Datum {
    let mut rows = Vec::new();
    for v in repos {
        rows.push((
            format!("repo:{}", v.owner),
            format!("{}:{}", v.repository, v.revision),
        ));
    }
    for v in fragments {
        rows.push((format!("fragment:{}", v.owner), cid_text(&v.content_id.0)));
    }
    for v in files {
        rows.push((
            format!("file:{}:{}", v.owner, v.path),
            cid_text(&v.content_id.0),
        ));
    }
    for v in evidence {
        rows.push((format!("evidence:{v:?}"), format!("{v:?}")));
    }
    for v in limitations {
        rows.push((format!("limitation:{v:?}"), format!("{v:?}")));
    }
    rows.sort();
    Datum::Node {
        tag: tag("source-deck", "deck-v1"),
        fields: vec![(
            Symbol::new("entries"),
            Datum::Map(
                rows.into_iter()
                    .map(|(k, v)| (Datum::String(k), Datum::String(v)))
                    .collect(),
            ),
        )],
    }
}

fn required(value: &str) -> Result<(), Failure> {
    if value.is_empty() {
        Err(Failure::Stale {
            subject: "empty identity field".into(),
        })
    } else {
        Ok(())
    }
}
fn valid_path(path: &str) -> Result<(), Failure> {
    if path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || path.contains('\\')
    {
        Err(Failure::InvalidPath(path.into()))
    } else {
        Ok(())
    }
}
fn check_count(name: &'static str, actual: usize, maximum: usize) -> Result<(), Failure> {
    if actual > maximum {
        Err(Failure::OverLimit {
            limit: name,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}
fn require_owner(owners: &BTreeSet<&str>, owner: &str, subject: &str) -> Result<(), Failure> {
    if owners.contains(owner) {
        Ok(())
    } else {
        Err(Failure::OwnerMismatch {
            subject: subject.into(),
        })
    }
}
fn verify_content(subject: &str, bytes: &[u8], expected: &ByteContentId) -> Result<(), Failure> {
    if &ByteContentId::of(bytes)? == expected {
        Ok(())
    } else {
        Err(Failure::ContentMismatch(subject.into()))
    }
}
fn unique<'a>(items: impl Iterator<Item = (&'a String, &'static str)>) -> Result<(), Failure> {
    let mut seen = BTreeSet::new();
    for (id, kind) in items {
        if !seen.insert(id) {
            return Err(Failure::Duplicate {
                kind,
                id: id.clone(),
            });
        }
    }
    Ok(())
}
fn tag(namespace: &str, name: &str) -> Symbol {
    Symbol::qualified(namespace, name)
}
fn field(name: &str, value: &str) -> (Symbol, Datum) {
    (Symbol::new(name), Datum::String(value.into()))
}
fn cid_field(name: &str, id: &ContentId) -> (Symbol, Datum) {
    (Symbol::new(name), Datum::String(cid_text(id)))
}
fn cid_text(id: &ContentId) -> String {
    format!(
        "{}:{}",
        id.algorithm,
        id.bytes
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>()
    )
}

#[cfg(test)]
mod tests;
