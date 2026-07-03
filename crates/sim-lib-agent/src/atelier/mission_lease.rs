//! Workspace leases and conflict checks for Atelier missions.

use super::mission::{AgentMission, key};
use sim_kernel::{Expr, Symbol};

/// Workspace lease target kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceLeaseKind {
    /// A single file path within a repo.
    File,
    /// A directory subtree within a repo.
    Directory,
    /// A named crate within a repo.
    Crate,
    /// An IDE object identified by symbol, independent of any repo.
    IdeObject,
}

/// Lease mode. Write leases conflict with overlapping read or write leases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceLeaseMode {
    /// Read access that tolerates other concurrent readers.
    SharedRead,
    /// Write access that excludes any overlapping read or write lease.
    ExclusiveWrite,
}

/// Declared claim on a file, directory, crate, or IDE object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceLease {
    /// What kind of target the lease claims.
    pub kind: WorkspaceLeaseKind,
    /// Owning repo, or `None` for repo-independent targets such as IDE objects.
    pub repo: Option<String>,
    /// Normalized target path, crate name, or object id.
    pub target: String,
    /// Whether the claim is a shared read or an exclusive write.
    pub mode: WorkspaceLeaseMode,
}

impl WorkspaceLease {
    /// Builds an exclusive-write lease on a file in a repo.
    pub fn file(repo: impl Into<String>, path: impl Into<String>) -> Self {
        Self::repo_target(WorkspaceLeaseKind::File, repo, path)
    }

    /// Builds an exclusive-write lease on a directory subtree in a repo.
    pub fn directory(repo: impl Into<String>, path: impl Into<String>) -> Self {
        Self::repo_target(WorkspaceLeaseKind::Directory, repo, path)
    }

    /// Builds an exclusive-write lease on a named crate in a repo.
    pub fn crate_name(repo: impl Into<String>, crate_name: impl Into<String>) -> Self {
        Self::repo_target(WorkspaceLeaseKind::Crate, repo, crate_name)
    }

    /// Builds an exclusive-write lease on an IDE object by id.
    pub fn ide_object(object_id: Symbol) -> Self {
        Self {
            kind: WorkspaceLeaseKind::IdeObject,
            repo: None,
            target: object_id.to_string(),
            mode: WorkspaceLeaseMode::ExclusiveWrite,
        }
    }

    /// Returns the lease downgraded to shared-read mode.
    pub fn for_read(mut self) -> Self {
        self.mode = WorkspaceLeaseMode::SharedRead;
        self
    }

    /// Returns the lease upgraded to exclusive-write mode.
    pub fn for_write(mut self) -> Self {
        self.mode = WorkspaceLeaseMode::ExclusiveWrite;
        self
    }

    pub(crate) fn as_expr(&self) -> Expr {
        let mut entries = vec![
            key(
                "kind",
                Expr::Symbol(Symbol::new(lease_kind_label(&self.kind))),
            ),
            key("target", Expr::String(self.target.clone())),
            key(
                "mode",
                Expr::Symbol(Symbol::new(lease_mode_label(&self.mode))),
            ),
        ];
        if let Some(repo) = &self.repo {
            entries.push(key("repo", Expr::String(repo.clone())));
        }
        Expr::Map(entries)
    }

    fn repo_target(
        kind: WorkspaceLeaseKind,
        repo: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            repo: Some(repo.into()),
            target: normalize_path(&target.into()),
            mode: WorkspaceLeaseMode::ExclusiveWrite,
        }
    }

    fn conflicts_with(&self, other: &Self) -> bool {
        if self.mode == WorkspaceLeaseMode::SharedRead
            && other.mode == WorkspaceLeaseMode::SharedRead
        {
            return false;
        }
        leases_overlap(self, other)
    }
}

/// Conflict between two mission leases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceLeaseConflict {
    /// Id of the mission holding the left lease.
    pub left_mission: Symbol,
    /// The conflicting lease from the left mission.
    pub left: WorkspaceLease,
    /// Id of the mission holding the right lease.
    pub right_mission: Symbol,
    /// The conflicting lease from the right mission.
    pub right: WorkspaceLease,
}

/// Finds incompatible leases across mission pairs.
pub fn detect_workspace_lease_conflicts(missions: &[AgentMission]) -> Vec<WorkspaceLeaseConflict> {
    let mut conflicts = Vec::new();
    for (left_index, left_mission) in missions.iter().enumerate() {
        for right_mission in missions.iter().skip(left_index + 1) {
            for left in left_mission.leases() {
                for right in right_mission.leases() {
                    if left.conflicts_with(right) {
                        conflicts.push(WorkspaceLeaseConflict {
                            left_mission: left_mission.id().clone(),
                            left: left.clone(),
                            right_mission: right_mission.id().clone(),
                            right: right.clone(),
                        });
                    }
                }
            }
        }
    }
    conflicts
}

pub(crate) fn leases_expr(leases: &[WorkspaceLease]) -> Expr {
    Expr::List(leases.iter().map(WorkspaceLease::as_expr).collect())
}

fn leases_overlap(left: &WorkspaceLease, right: &WorkspaceLease) -> bool {
    if left.kind == WorkspaceLeaseKind::IdeObject || right.kind == WorkspaceLeaseKind::IdeObject {
        return left.kind == right.kind && left.target == right.target;
    }
    if left.repo != right.repo {
        return false;
    }
    match (&left.kind, &right.kind) {
        (WorkspaceLeaseKind::Crate, WorkspaceLeaseKind::Crate) => left.target == right.target,
        (WorkspaceLeaseKind::File, WorkspaceLeaseKind::File) => left.target == right.target,
        (WorkspaceLeaseKind::Directory, WorkspaceLeaseKind::Directory) => {
            path_contains(&left.target, &right.target) || path_contains(&right.target, &left.target)
        }
        (WorkspaceLeaseKind::Directory, WorkspaceLeaseKind::File) => {
            path_contains(&left.target, &right.target)
        }
        (WorkspaceLeaseKind::File, WorkspaceLeaseKind::Directory) => {
            path_contains(&right.target, &left.target)
        }
        _ => false,
    }
}

fn path_contains(directory: &str, path: &str) -> bool {
    directory == "."
        || directory == path
        || path
            .strip_prefix(directory)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn normalize_path(path: &str) -> String {
    let normalized = path.trim_matches('/').replace('\\', "/");
    if normalized.is_empty() {
        ".".to_owned()
    } else {
        normalized
    }
}

fn lease_kind_label(kind: &WorkspaceLeaseKind) -> &'static str {
    match kind {
        WorkspaceLeaseKind::File => "file",
        WorkspaceLeaseKind::Directory => "directory",
        WorkspaceLeaseKind::Crate => "crate",
        WorkspaceLeaseKind::IdeObject => "ide-object",
    }
}

fn lease_mode_label(mode: &WorkspaceLeaseMode) -> &'static str {
    match mode {
        WorkspaceLeaseMode::SharedRead => "shared-read",
        WorkspaceLeaseMode::ExclusiveWrite => "exclusive-write",
    }
}
