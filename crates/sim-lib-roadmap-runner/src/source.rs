use sim_lib_exec::{
    ArgAtom, MountAccess, ProcessCancellation, ProgramRef, SandboxAttempt, SandboxControl,
    SandboxLauncher, SandboxLimits, SandboxMount, SandboxPolicy, SandboxRequest,
    SandboxRequirement, SealedBindings,
};
use sim_source_deck::{
    ByteContentId, ClaimCertificate, DeckInput, DeckLimits, Excerpt, FragmentDecoder, FragmentPin,
    Limitation, RepositorySnapshot, SourceDeck, SourceDeckId, SourceFile, SourceQuery, SpecimenPin,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

const SOURCE_GUEST_ROOT: &str = "/source";

/// Identity of the exact checkout for which evidence is requested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryIdentity {
    pub owner: String,
    pub repository: String,
    pub local_head: String,
    /// Opaque mount source resolved by the platform capsule.
    pub root: String,
}

/// One explicit, shell-free repo-contract artifact command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactCommand {
    pub identity: String,
    pub program: String,
    pub argv: Vec<String>,
    pub working_directory: String,
    pub environment: BTreeMap<String, String>,
}

/// Hard limits shared by command capture and deck construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDeckBounds {
    pub command_wall_time_ms: u64,
    pub command_output_bytes: usize,
    pub deck: DeckLimits,
}

/// Exact evidence request. Paths are repository-relative and roots are allowlisted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDeckRequest {
    pub repository: RepositoryIdentity,
    pub allowed_roots: Vec<String>,
    pub queries: Vec<SourceQuery>,
    pub artifact_command: ArtifactCommand,
    pub bounds: SourceDeckBounds,
}

/// Decoder-owned projection of repo-contract output; this adapter never scans source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactProjection {
    pub repository_head: String,
    pub fragment: FragmentPin,
    pub certificates: Vec<ClaimCertificate>,
    pub source_paths: Vec<String>,
    pub excerpts: Vec<ArtifactExcerpt>,
    pub specimens: Vec<ArtifactSpecimen>,
    pub limitations: Vec<Limitation>,
    pub artifact_id: ByteContentId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactExcerpt {
    pub id: String,
    pub path: String,
    pub start: usize,
    pub end: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactSpecimen {
    pub id: String,
    pub path: String,
}

/// Published tooling adapter for the repo-contract wire format.
pub trait RepoContractDecoder: Send + Sync {
    fn decode(&self, bytes: &[u8]) -> Result<ArtifactProjection, SourceDeckProviderError>;
}

/// Read-only checkout authority. Implementations must reject non-regular files and escapes.
pub trait SourceRepository: Send + Sync {
    fn read_regular(
        &self,
        repository: &RepositoryIdentity,
        path: &str,
    ) -> Result<Vec<u8>, SourceDeckProviderError>;
}

/// Auditable result of repo-contract artifact emission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactReceipt {
    pub command_identity: String,
    pub artifact_id: ByteContentId,
    pub repository_head: String,
    pub sandbox_launcher: String,
    pub output_bytes: usize,
}

/// Complete receipt returned to production execution paths.
#[derive(Clone, Debug)]
pub struct SourceDeckReceipt {
    pub deck: SourceDeck,
    pub deck_id: SourceDeckId,
    pub artifact: ArtifactReceipt,
    pub dependencies: BTreeSet<String>,
}

/// Mutation facts used to decide whether an old receipt may be reused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TouchedPaths(pub BTreeSet<String>);

impl SourceDeckReceipt {
    /// A receipt is reusable only if its exact dependency set is disjoint from mutations.
    pub fn reusable_after(&self, touched: &TouchedPaths) -> bool {
        self.dependencies.is_disjoint(&touched.0)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SourceDeckProviderError {
    #[error("invalid source-deck request: {0}")]
    Invalid(String),
    #[error("repo-contract command was refused: {0}")]
    Command(String),
    #[error("repo-contract artifact exceeded its output bound")]
    Oversize,
    #[error("repo-contract artifact identity mismatch")]
    ArtifactIdentity,
    #[error("repository head differs from the requested exact head")]
    HeadMismatch,
    #[error("source path is outside the declared roots: {0}")]
    Path(String),
    #[error("requested evidence was not covered: {0}")]
    Coverage(String),
    #[error("source-deck grounding failed: {0}")]
    Deck(String),
    #[error("repository read failed: {0}")]
    Read(String),
}

/// Real execution adapter: command effects are sandboxed and grounding remains in sim-source-deck.
pub struct SourceDeckProvider<'a> {
    launcher: &'a dyn SandboxLauncher,
    repository: &'a dyn SourceRepository,
    artifact_decoder: &'a dyn RepoContractDecoder,
    fragment_decoder: &'a dyn FragmentDecoder,
}

impl<'a> SourceDeckProvider<'a> {
    pub fn new(
        launcher: &'a dyn SandboxLauncher,
        repository: &'a dyn SourceRepository,
        artifact_decoder: &'a dyn RepoContractDecoder,
        fragment_decoder: &'a dyn FragmentDecoder,
    ) -> Self {
        Self {
            launcher,
            repository,
            artifact_decoder,
            fragment_decoder,
        }
    }

    pub fn provide(
        &self,
        request: &SourceDeckRequest,
        cancellation: &ProcessCancellation,
    ) -> Result<SourceDeckReceipt, SourceDeckProviderError> {
        validate_request(request)?;
        let sandbox_request = sandbox_request(request)?;
        let result = match self.launcher.launch(&sandbox_request, cancellation) {
            SandboxAttempt::Completed(v)
                if v.exit_code == 0 && v.report.proves_required(&sandbox_request.policy) =>
            {
                v
            }
            SandboxAttempt::Completed(v) => {
                return Err(SourceDeckProviderError::Command(format!(
                    "exit {}",
                    v.exit_code
                )));
            }
            other => return Err(SourceDeckProviderError::Command(format!("{other:?}"))),
        };
        if result.stdout.len() > request.bounds.command_output_bytes
            || !result.report.limit_hits.is_empty()
        {
            return Err(SourceDeckProviderError::Oversize);
        }
        let projection = self.artifact_decoder.decode(&result.stdout)?;
        if projection.repository_head != request.repository.local_head {
            return Err(SourceDeckProviderError::HeadMismatch);
        }
        if ByteContentId::of(&result.stdout)
            .map_err(|e| SourceDeckProviderError::Deck(e.to_string()))?
            != projection.artifact_id
        {
            return Err(SourceDeckProviderError::ArtifactIdentity);
        }

        let mut dependencies = BTreeSet::new();
        let mut files = Vec::new();
        for path in &projection.source_paths {
            validate_path(path, &request.allowed_roots)?;
            let bytes = self.repository.read_regular(&request.repository, path)?;
            dependencies.insert(path.clone());
            files.push(SourceFile {
                owner: request.repository.owner.clone(),
                path: path.clone(),
                content_id: ByteContentId::of(&bytes)
                    .map_err(|e| SourceDeckProviderError::Deck(e.to_string()))?,
                bytes,
            });
        }
        let by_path: BTreeMap<_, _> = files.iter().map(|f| (f.path.as_str(), f)).collect();
        let mut excerpts = Vec::new();
        for item in &projection.excerpts {
            let file = by_path
                .get(item.path.as_str())
                .ok_or_else(|| SourceDeckProviderError::Coverage(item.id.clone()))?;
            let bytes = file
                .bytes
                .get(item.start..item.end)
                .ok_or_else(|| SourceDeckProviderError::Coverage(item.id.clone()))?
                .to_vec();
            excerpts.push(Excerpt {
                id: item.id.clone(),
                owner: request.repository.owner.clone(),
                path: item.path.clone(),
                start: item.start,
                end: item.end,
                bytes,
            });
        }
        let mut specimens = Vec::new();
        for item in &projection.specimens {
            validate_path(&item.path, &request.allowed_roots)?;
            let bytes = self
                .repository
                .read_regular(&request.repository, &item.path)?;
            dependencies.insert(item.path.clone());
            specimens.push(SpecimenPin {
                id: item.id.clone(),
                owner: request.repository.owner.clone(),
                content_id: ByteContentId::of(&bytes)
                    .map_err(|e| SourceDeckProviderError::Deck(e.to_string()))?,
                bytes,
            });
        }
        dependencies.insert("docs/generated/repo-contract.json".into());
        dependencies.insert("docs/generated/sim-index-fragment.sx".into());
        ensure_coverage(&request.queries, &projection)?;
        let deck = sim_source_deck::build(DeckInput {
            repositories: vec![RepositorySnapshot {
                owner: request.repository.owner.clone(),
                repository: request.repository.repository.clone(),
                revision: request.repository.local_head.clone(),
            }],
            fragments: vec![projection.fragment],
            certificates: projection.certificates,
            files,
            excerpts,
            specimens,
            queries: request.queries.clone(),
            limitations: projection.limitations,
            limits: request.bounds.deck,
            decoder: self.fragment_decoder,
        })
        .map_err(|e| SourceDeckProviderError::Deck(e.to_string()))?;
        let deck_id = deck.id().clone();
        Ok(SourceDeckReceipt {
            deck,
            deck_id,
            artifact: ArtifactReceipt {
                command_identity: request.artifact_command.identity.clone(),
                artifact_id: projection.artifact_id,
                repository_head: projection.repository_head,
                sandbox_launcher: result.report.launcher,
                output_bytes: result.stdout.len(),
            },
            dependencies,
        })
    }
}

fn sandbox_request(request: &SourceDeckRequest) -> Result<SandboxRequest, SourceDeckProviderError> {
    let required = [
        SandboxControl::Network,
        SandboxControl::Mounts,
        SandboxControl::Root,
        SandboxControl::Environment,
        SandboxControl::Identity,
        SandboxControl::Cpu,
        SandboxControl::Memory,
        SandboxControl::WallTime,
        SandboxControl::ProcessCount,
        SandboxControl::FileCount,
        SandboxControl::FileBytes,
        SandboxControl::Output,
        SandboxControl::Stdin,
        SandboxControl::ProcessTree,
    ]
    .into_iter()
    .map(|c| (c, SandboxRequirement::Required));
    let policy = SandboxPolicy::new(
        required,
        vec![SandboxMount {
            source: request.repository.root.clone(),
            guest_path: SOURCE_GUEST_ROOT.into(),
            access: MountAccess::ReadOnly,
        }],
        SandboxLimits {
            cpu_seconds: 30,
            memory_bytes: 512 * 1024 * 1024,
            wall_time_ms: request.bounds.command_wall_time_ms,
            process_count: 32,
            file_count: 1,
            file_bytes: 1,
            output_bytes: request.bounds.command_output_bytes,
            stdin_bytes: 1,
        },
    )
    .map_err(|e| SourceDeckProviderError::Invalid(e.to_string()))?;
    let argv = request
        .artifact_command
        .argv
        .iter()
        .cloned()
        .map(ArgAtom::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| SourceDeckProviderError::Invalid(e.to_string()))?;
    let mut environment_values = request.artifact_command.environment.clone();
    environment_values.insert(
        "PWD".into(),
        request.artifact_command.working_directory.clone(),
    );
    let environment = SealedBindings::literals(environment_values)
        .map_err(|e| SourceDeckProviderError::Invalid(e.to_string()))?;
    SandboxRequest::new(
        ProgramRef::new(request.artifact_command.program.clone())
            .map_err(|e| SourceDeckProviderError::Invalid(e.to_string()))?,
        argv,
        environment,
        vec![],
        policy,
    )
    .map_err(|e| SourceDeckProviderError::Invalid(e.to_string()))
}

fn validate_request(r: &SourceDeckRequest) -> Result<(), SourceDeckProviderError> {
    if r.repository.owner.is_empty()
        || r.repository.repository.is_empty()
        || r.repository.local_head.is_empty()
        || r.repository.root.is_empty()
        || r.artifact_command.identity.is_empty()
        || r.artifact_command.working_directory != SOURCE_GUEST_ROOT
        || r.allowed_roots.is_empty()
        || r.bounds.command_wall_time_ms == 0
        || r.bounds.command_output_bytes == 0
    {
        return Err(SourceDeckProviderError::Invalid(
            "empty identity, roots, or bounds".into(),
        ));
    }
    for root in &r.allowed_roots {
        validate_relative(root)?;
    }
    Ok(())
}
fn validate_relative(path: &str) -> Result<(), SourceDeckProviderError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        Err(SourceDeckProviderError::Path(path.into()))
    } else {
        Ok(())
    }
}
fn validate_path(path: &str, roots: &[String]) -> Result<(), SourceDeckProviderError> {
    validate_relative(path)?;
    if roots.iter().any(|root| {
        path == root
            || path
                .strip_prefix(root)
                .is_some_and(|tail| tail.starts_with('/'))
    }) {
        Ok(())
    } else {
        Err(SourceDeckProviderError::Path(path.into()))
    }
}
fn ensure_coverage(
    queries: &[SourceQuery],
    p: &ArtifactProjection,
) -> Result<(), SourceDeckProviderError> {
    for q in queries {
        let covered = match q {
            SourceQuery::Anchor(id) => p.certificates.iter().any(|v| &v.anchor == id),
            SourceQuery::Excerpt(id) => p.excerpts.iter().any(|v| &v.id == id),
            SourceQuery::Specimen(id) => p.specimens.iter().any(|v| &v.id == id),
        };
        if !covered {
            return Err(SourceDeckProviderError::Coverage(format!("{q:?}")));
        }
    }
    Ok(())
}

/// Deterministic provider for higher-level state-machine tests. It cannot execute commands.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub struct FakeSourceDeckProvider {
    receipt: SourceDeckReceipt,
}

#[cfg(any(test, feature = "test-support"))]
impl FakeSourceDeckProvider {
    pub fn new(receipt: SourceDeckReceipt) -> Self {
        Self { receipt }
    }
    pub fn provide(&self) -> SourceDeckReceipt {
        self.receipt.clone()
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
// conformance: content-pinned execution source-deck acquisition.
