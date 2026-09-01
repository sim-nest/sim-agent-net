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
