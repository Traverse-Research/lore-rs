use lore_sys::{
    lore_file_action_t, lore_file_stage_count_data_t, lore_file_unstage_count_data_t, lore_hash_t,
    lore_revision_commit_count_data_t, LORE_FILE_ACTION_ADD, LORE_FILE_ACTION_COPY,
    LORE_FILE_ACTION_DELETE, LORE_FILE_ACTION_KEEP, LORE_FILE_ACTION_MOVE,
};

use crate::{
    BranchId, BranchInfoArgs, Event, FileStageArgs, FileUnstageArgs, GlobalArgs, InstanceId, Lore,
    LoreError, NodeKind, RepositoryId, RepositoryInfoArgs, RepositoryStatusArgs, Revision,
    RevisionCommitArgs, RevisionInfoArgs,
};

/// A local repository instance — the directory holding `.lore` — together
/// with the [`GlobalArgs`] every command against it runs with.
///
/// This is what Lore's repository verbs operate on: `repository_*`,
/// `branch_*`, `file_*`, `revision_*` and the rest of the working-tree
/// commands. Lore checks for `.lore` under `repository_path` before running
/// any of them and reports "repository not found" otherwise. The instance need
/// not hold a working tree: a `.lore` with only `id` and `config.toml` in it
/// is enough for the reads here, and `config.toml` is where Lore finds the
/// remote these commands talk to.
///
/// Lore's storage API is not tied to an instance, so it is not here; see
/// [`Store`](crate::Store). An instance's own stores are opened with
/// [`StoreLocation::OnDisk`](crate::StoreLocation::OnDisk).
///
/// Cheap to clone, `Send + Sync`, and nothing is opened until a call.
#[derive(Debug, Clone)]
pub struct Repository {
    lore: &'static Lore,
    globals: GlobalArgs,
}

impl Repository {
    /// `globals.repository_path` names the instance. Nothing is checked until
    /// the first call.
    pub fn new(lore: &'static Lore, globals: GlobalArgs) -> Self {
        Self { lore, globals }
    }

    pub fn lore(&self) -> &'static Lore {
        self.lore
    }

    pub fn globals(&self) -> &GlobalArgs {
        &self.globals
    }

    /// Changes only this value; clone first when other holders should keep
    /// their flags.
    pub fn globals_mut(&mut self) -> &mut GlobalArgs {
        &mut self.globals
    }

    /// The instance directory, the one holding `.lore`.
    pub fn path(&self) -> &str {
        &self.globals.repository_path
    }

    /// Asks about this repository.
    ///
    /// Lore assembles the question from the instance: the server from
    /// `.lore/config.toml`, the repository from `.lore/id`. With
    /// [`GlobalArgs::local`] set it answers from the instance's own stores
    /// instead of the server.
    ///
    /// This corresponds to `lore_sys::Lore::lore_repository_info`.
    pub fn info(&self) -> Result<RepositoryInfo, LoreError> {
        repository_info(self.lore, &self.globals, "")
    }

    /// Lists the checkouts registered for this repository. Lore registers a
    /// checkout on its first writable open, so one only ever read through
    /// these bindings is not among them.
    ///
    /// Opens the instance to do so, which creates its `.lore/instance` when
    /// missing, as it is on a fresh checkout of a `.lore` that only commits
    /// `id` and `config.toml`. [`Self::info`] reads that file but never
    /// creates it, so call this first on such a checkout. Needs no branch
    /// state, so it succeeds on a bare `.lore`.
    ///
    /// This corresponds to `lore_sys::Lore::lore_repository_instance_list`.
    pub fn instances(&self) -> Result<Vec<InstanceInfo>, LoreError> {
        const COMMAND: &str = "repository::instance_list";
        let mut instances = Vec::new();

        crate::call::repository_instance_list(self.lore, COMMAND, &self.globals, |event| {
            crate::log_event(&event);

            if let Ok(Event::RepositoryInstance {
                instance_id,
                path,
                branch_name,
                branch,
                revision,
                stale,
            }) = event
            {
                instances.push(InstanceInfo {
                    id: InstanceId::from_raw(instance_id),
                    path: path.to_owned(),
                    branch_name: branch_name.to_owned(),
                    branch: BranchId::from_raw(branch),
                    revision: Revision::from_raw(revision),
                    staleness: InstanceStaleness::from_raw(stale),
                });
            }
        })?;

        Ok(instances)
    }

    /// Asks about one branch. Empty `name` is the branch the instance is on,
    /// which needs a synced instance rather than a bare `.lore`.
    ///
    /// This corresponds to `lore_sys::Lore::lore_branch_info`.
    pub fn branch(&self, name: &str) -> Result<BranchInfo, LoreError> {
        const COMMAND: &str = "branch::info";
        let mut info = None;

        crate::call::branch_info(
            self.lore,
            COMMAND,
            &self.globals,
            BranchInfoArgs {
                branch: name,
                link: "",
            },
            |event| {
                crate::log_event(&event);

                if let Ok(Event::BranchInfo {
                    id,
                    name,
                    category,
                    latest,
                    latest_remote,
                    parent,
                    branch_point,
                    creator,
                    created,
                    archived,
                }) = event
                {
                    info = Some(BranchInfo {
                        id: BranchId::from_raw(id),
                        name: name.to_owned(),
                        category: category.to_owned(),
                        creator: creator.to_owned(),
                        created,
                        archived,
                        latest: Revision::from_raw(latest),
                        latest_remote: Revision::from_raw(latest_remote),
                        parent: BranchId::from_raw(parent),
                        branch_point: Revision::from_raw(branch_point),
                    });
                }
            },
        )?;

        let info = info.ok_or(LoreError::MissingEvent {
            command: COMMAND,
            expected: "branch_info",
        })?;

        if info.id.is_zero() {
            return Err(LoreError::InvalidResponse {
                command: COMMAND,
                detail: "the branch id is all zeroes".to_owned(),
            });
        }

        Ok(info)
    }

    /// Resolves a revision signature the way Lore itself does, and reports
    /// the revision it names.
    ///
    /// `signature` takes every form Lore's CLI accepts: a full hash,
    /// `branch@LATEST`, `branch@<number>`, or `@LATEST` for the branch the
    /// instance is on. For `@LATEST` Lore compares the local tip with the
    /// remote's by walking the history between them: the remote wins only
    /// when it is strictly ahead, so unpushed local commits are never hidden
    /// and a stale instance never wins over the server. [`GlobalArgs::local`]
    /// or `offline` restricts that to the local tip, `remote` to the server's.
    ///
    /// [`None`] when the signature resolves to no revision at all, which is a
    /// branch with nothing on it. A signature Lore cannot resolve is an error.
    ///
    /// This corresponds to `lore_sys::Lore::lore_revision_info`.
    pub fn revision(&self, signature: &str) -> Result<Option<RevisionInfo>, LoreError> {
        const COMMAND: &str = "revision::info";
        let mut info = None;

        crate::call::revision_info(
            self.lore,
            COMMAND,
            &self.globals,
            RevisionInfoArgs {
                revision: signature,
                delta: false,
                metadata: false,
            },
            |event| {
                crate::log_event(&event);

                if let Ok(Event::RevisionInfo {
                    repository,
                    revision,
                    revision_number,
                    parents,
                }) = event
                {
                    info = Some(RevisionInfo {
                        repository: RepositoryId::from_raw(repository),
                        revision: Revision::from_raw(revision),
                        number: revision_number,
                        parents: [
                            Revision::from_raw(parents[0]),
                            Revision::from_raw(parents[1]),
                        ],
                    });
                }
            },
        )?;

        let info = info.ok_or(LoreError::MissingEvent {
            command: COMMAND,
            expected: "revision_info",
        })?;

        Ok(Some(info).filter(|info| !info.revision.is_zero()))
    }
}

/// One registered checkout of a repository, from [`Repository::instances`].
#[derive(Debug, Clone)]
pub struct InstanceInfo {
    pub id: InstanceId,
    /// Where the checkout was registered from.
    pub path: String,
    /// The branch the checkout is on; empty, with [`Self::branch`] zero, when
    /// it has none, as on a bare `.lore`.
    pub branch_name: String,
    pub branch: BranchId,
    pub revision: Revision,
    pub staleness: InstanceStaleness,
}

/// Whether an [`InstanceInfo`] still describes a live checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceStaleness {
    Active,
    /// The registered path no longer exists.
    PathMissing,
    /// The path holds a checkout whose `.lore/instance` names another
    /// instance: it was re-created or re-cloned since.
    Superseded,
    /// The path holds no readable `.lore/instance`.
    NoInstance,
    /// A reason this version of the bindings does not know.
    Other(u8),
}

impl InstanceStaleness {
    fn from_raw(stale: u8) -> Self {
        match stale {
            0 => Self::Active,
            1 => Self::PathMissing,
            2 => Self::Superseded,
            3 => Self::NoInstance,
            other => Self::Other(other),
        }
    }
}

/// A resolved revision, from [`Repository::revision`].
#[derive(Debug, Clone)]
pub struct RevisionInfo {
    pub repository: RepositoryId,
    pub revision: Revision,
    /// Lore's sequential number of the revision on its branch.
    pub number: u64,
    parents: [Revision; 2],
}

impl RevisionInfo {
    /// The parent revisions: none for an initial revision, one normally, two
    /// for a merge.
    pub fn parents(&self) -> impl Iterator<Item = Revision> + '_ {
        self.parents
            .iter()
            .copied()
            .filter(|parent| !parent.is_zero())
    }
}

impl Lore {
    /// Asks a server about a repository.
    ///
    /// `repository_url` is the server and the repository name together, as
    /// `lore://host:port/name`. This needs no local instance; Lore runs it
    /// against in-memory stores. It reads [`GlobalArgs::identity`] — without
    /// one a server that requires authentication refuses — and it must not
    /// see [`GlobalArgs::local`], which makes Lore ignore the URL and read
    /// `repository_path` instead; that is [`Repository::info`].
    ///
    /// This corresponds to `lore_sys::Lore::lore_repository_info`.
    pub fn repository_info(
        &self,
        globals: &GlobalArgs,
        repository_url: &str,
    ) -> Result<RepositoryInfo, LoreError> {
        repository_info(self, globals, repository_url)
    }
}

/// What a server says about a repository.
///
/// [`Self::id`] and [`Self::default_branch`] are always set — a response
/// carrying zero for either is rejected as [`LoreError::InvalidResponse`]
/// rather than handed on.
#[derive(Debug, Clone)]
pub struct RepositoryInfo {
    /// Also the storage partition every read against this repository names.
    pub id: RepositoryId,
    pub name: String,
    pub description: String,
    pub default_branch: BranchId,
    pub default_branch_name: String,
    pub creator: String,
    /// **Milliseconds** since the Unix epoch — unlike a branch's `created`,
    /// which is seconds.
    pub created: u64,
    /// The server as Lore resolved it out of the URL, which is not
    /// necessarily the string that went in.
    pub remote_url: String,
}

fn repository_info(
    lore: &Lore,
    globals: &GlobalArgs,
    repository_url: &str,
) -> Result<RepositoryInfo, LoreError> {
    const COMMAND: &str = "repository::info";
    let mut info = None;

    crate::call::repository_info(
        lore,
        COMMAND,
        globals,
        RepositoryInfoArgs { repository_url },
        |event| {
            crate::log_event(&event);

            if let Ok(Event::RepositoryData {
                remote_url,
                id,
                name,
                description,
                default_branch,
                default_branch_name,
                creator,
                created,
            }) = event
            {
                info = Some(RepositoryInfo {
                    id: RepositoryId::from_raw(id),
                    name: name.to_owned(),
                    description: description.to_owned(),
                    default_branch: BranchId::from_raw(default_branch),
                    default_branch_name: default_branch_name.to_owned(),
                    creator: creator.to_owned(),
                    created,
                    remote_url: remote_url.to_owned(),
                });
            }
        },
    )?;

    let info = info.ok_or(LoreError::MissingEvent {
        command: COMMAND,
        expected: "repository_data",
    })?;

    for (identifier_is_zero, detail) in [
        (info.id.is_zero(), "the repository id is all zeroes"),
        (
            info.default_branch.is_zero(),
            "the default branch id is all zeroes",
        ),
    ] {
        if identifier_is_zero {
            return Err(LoreError::InvalidResponse {
                command: COMMAND,
                detail: detail.to_owned(),
            });
        }
    }

    Ok(info)
}

/// What a repository says about one of its branches.
///
/// The revisions a branch may not have are behind accessors that return
/// [`None`] for Lore's all-zero "none", so no zero identifier is ever handed
/// out: [`Self::latest`], [`Self::latest_remote`], [`Self::branch_point`].
///
/// The two tips are reported as Lore holds them, not reconciled. They differ
/// whenever the instance has commits the server has not seen, or the server
/// has commits the instance has not synced, and telling those apart takes the
/// history between them. [`Repository::revision`] with `name@LATEST` is Lore's
/// own answer to "which one should I read".
#[derive(Debug, Clone)]
pub struct BranchInfo {
    pub id: BranchId,
    pub name: String,
    pub category: String,
    pub creator: String,
    /// Seconds since the Unix epoch.
    pub created: u64,
    pub archived: bool,
    latest: Revision,
    latest_remote: Revision,
    parent: BranchId,
    branch_point: Revision,
}

impl BranchInfo {
    /// Tip in the instance's local store: the last revision committed or
    /// synced here. [`None`] when the instance has never had the branch.
    pub fn latest(&self) -> Option<Revision> {
        Some(self.latest).filter(|revision| !revision.is_zero())
    }

    /// Tip on the server. [`None`] when the branch has nothing pushed, or
    /// when there was no server to ask.
    pub fn latest_remote(&self) -> Option<Revision> {
        Some(self.latest_remote).filter(|revision| !revision.is_zero())
    }

    /// The parent branch and the revision on it this branch was created from.
    /// [`None`] for a branch with no parent. Both or neither.
    pub fn branch_point(&self) -> Option<(BranchId, Revision)> {
        (!self.parent.is_zero() && !self.branch_point.is_zero())
            .then_some((self.parent, self.branch_point))
    }
}

/// What Lore did, or would do, to one path: `lore_file_action_t` as an enum.
///
/// [`Self::Other`] rather than `#[non_exhaustive]`: the C type is an integer,
/// so an unknown value is a runtime possibility and the caller should be able
/// to see which one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileAction {
    /// The path stays where it is. Lore has no action for a modification, so
    /// this is what a changed file reports, and what an unstage reports for
    /// every staged change it drops.
    Keep,
    Add,
    Delete,
    Move,
    Copy,
    Other(lore_file_action_t),
}

impl FileAction {
    fn from_raw(action: lore_file_action_t) -> Self {
        match action {
            LORE_FILE_ACTION_KEEP => Self::Keep,
            LORE_FILE_ACTION_ADD => Self::Add,
            LORE_FILE_ACTION_DELETE => Self::Delete,
            LORE_FILE_ACTION_MOVE => Self::Move,
            LORE_FILE_ACTION_COPY => Self::Copy,
            other => Self::Other(other),
        }
    }
}

impl std::fmt::Display for FileAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Keep => f.write_str("keep"),
            Self::Add => f.write_str("add"),
            Self::Delete => f.write_str("delete"),
            Self::Move => f.write_str("move"),
            Self::Copy => f.write_str("copy"),
            Self::Other(action) => write!(f, "unknown action {action}"),
        }
    }
}

/// Lore's tally of a [`stage`](Repository::stage),
/// `lore_file_stage_count_data_t`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StageCount {
    pub directory_modify_count: u64,
    pub directory_add_count: u64,
    pub directory_delete_count: u64,
    pub directory_move_count: u64,
    pub file_modify_count: u64,
    pub file_add_count: u64,
    pub file_delete_count: u64,
    pub file_move_count: u64,
    /// Every item processed.
    pub total_count: u64,
}

impl StageCount {
    fn from_raw(count: lore_file_stage_count_data_t) -> Self {
        Self {
            directory_modify_count: count.directory_modify_count,
            directory_add_count: count.directory_add_count,
            directory_delete_count: count.directory_delete_count,
            directory_move_count: count.directory_move_count,
            file_modify_count: count.file_modify_count,
            file_add_count: count.file_add_count,
            file_delete_count: count.file_delete_count,
            file_move_count: count.file_move_count,
            total_count: count.total_count,
        }
    }
}

/// Lore's tally of an [`unstage`](Repository::unstage),
/// `lore_file_unstage_count_data_t`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnstageCount {
    pub directory_unstaged_count: u64,
    pub directory_discarded_count: u64,
    pub file_unstaged_count: u64,
    pub file_discarded_count: u64,
    /// Every item processed.
    pub total_count: u64,
}

impl UnstageCount {
    fn from_raw(count: lore_file_unstage_count_data_t) -> Self {
        Self {
            directory_unstaged_count: count.directory_unstaged_count,
            directory_discarded_count: count.directory_discarded_count,
            file_unstaged_count: count.file_unstaged_count,
            file_discarded_count: count.file_discarded_count,
            total_count: count.total_count,
        }
    }
}

/// Lore's tally of the files a [`commit`](Repository::commit) processed,
/// `lore_revision_commit_count_data_t`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommitCount {
    pub directory_count: u64,
    pub directory_total: u64,
    pub file_count: u64,
    pub file_total: u64,
    pub directory_delete_count: u64,
    pub file_modify_count: u64,
    pub file_delete_count: u64,
    /// Content bytes.
    pub bytes_transferred: u64,
    pub bytes_total: u64,
    /// Lore had found every file and directory to process.
    pub discovery_complete: bool,
}

impl CommitCount {
    fn from_raw(count: lore_revision_commit_count_data_t) -> Self {
        Self {
            directory_count: count.directory_count,
            directory_total: count.directory_total,
            file_count: count.file_count,
            file_total: count.file_total,
            directory_delete_count: count.directory_delete_count,
            file_modify_count: count.file_modify_count,
            file_delete_count: count.file_delete_count,
            bytes_transferred: count.bytes_transferred,
            bytes_total: count.bytes_total,
            discovery_complete: count.discovery_complete != 0,
        }
    }
}

/// Where the working tree stands, from [`Repository::status`]: Lore's
/// `repository_status_revision`, with the events that come after it.
#[derive(Debug, Clone)]
pub struct Status {
    pub repository: RepositoryId,
    pub branch: BranchId,
    pub branch_name: String,
    /// The revision the working tree is on; [`None`] on an empty branch.
    pub revision: Option<Revision>,
    pub revision_number: u64,
    /// [`None`] when nothing is staged.
    pub revision_staged: Option<Revision>,
    /// The incoming revision of a pending merge.
    pub revision_merged: Option<Revision>,
    /// The last revision merged in from the parent branch. Only reported with
    /// [`RepositoryStatusArgs::sync_point`].
    pub revision_merged_parent_branch: Option<Revision>,
    /// Tip of the branch in the local store.
    pub revision_local: Option<Revision>,
    pub revision_local_number: u64,
    /// Tip of the branch on the remote; [`None`] when unknown, when the branch
    /// is not on the remote, or when the remote is unavailable.
    pub revision_remote: Option<Revision>,
    pub revision_remote_number: u64,
    /// The local store holds revisions that are not on the remote's history
    /// line.
    pub is_local_ahead: bool,
    /// The remote holds revisions that are not in the local store.
    pub is_remote_ahead: bool,
    /// A remote is configured and reachable with a local identity. This is
    /// connectivity only, not authorization.
    pub remote_available: bool,
    /// The remote answered the revision query authoritatively, so the identity
    /// may access the repository.
    pub remote_authorized: bool,
    /// The branch exists on the remote.
    pub remote_branch_exist: bool,
    /// Every path with pending changes or conflicts, or untracked. Empty with
    /// [`RepositoryStatusArgs::revision_only`].
    pub files: Vec<StatusFile>,
    /// The size of the tree. Only with [`RepositoryStatusArgs::count`].
    pub count: Option<StatusCount>,
    /// The changes found. Only with [`RepositoryStatusArgs::scan`] or
    /// [`RepositoryStatusArgs::check_dirty`].
    pub summary: Option<StatusSummary>,
}

/// One path of a [`Status`].
#[derive(Debug, Clone)]
pub struct StatusFile {
    /// Repository-relative.
    pub path: String,
    /// In bytes.
    pub size: u64,
    pub action: FileAction,
    pub kind: NodeKind,
    /// Part of the next commit, as opposed to only changed on disk.
    pub staged: bool,
    /// The change comes from a merge.
    pub merged: bool,
    pub conflict: bool,
    pub conflict_unresolved: bool,
    /// The conflict was resolved automatically.
    pub conflict_automerged: bool,
    /// The conflict was resolved with the local side.
    pub conflict_mine: bool,
    /// The conflict was resolved with the incoming side.
    pub conflict_theirs: bool,
    /// The file differs from the recorded state.
    pub dirty: bool,
    /// Where a [`Move`](FileAction::Move) or [`Copy`](FileAction::Copy) came
    /// from.
    pub from_path: Option<String>,
}

/// The size of the tree a [`Status`] reported on, filtered by the view: the
/// staged state when there is one, the current revision otherwise.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusCount {
    pub directories: u64,
    pub files: u64,
}

/// The changes a [`status`](Repository::status) that scanned or checked
/// dirty files found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusSummary {
    pub adds: u64,
    pub deletes: u64,
    pub modifies: u64,
    pub moves: u64,
    pub copies: u64,
}

/// One path a [`stage`](Repository::stage) or
/// [`unstage`](Repository::unstage) touched.
#[derive(Debug, Clone)]
pub struct FileChange {
    /// Repository-relative.
    pub path: String,
    pub action: FileAction,
    /// Where a [`Move`](FileAction::Move) or [`Copy`](FileAction::Copy) came
    /// from. Only a stage reports it; an unstage never has one.
    pub from_path: Option<String>,
}

/// What a [`stage`](Repository::stage) did.
#[derive(Debug, Clone)]
pub struct Staged {
    /// In the order Lore reported them.
    pub files: Vec<FileChange>,
    pub count: StageCount,
    /// The staged tree Lore wrote for each repository the stage changed: the
    /// repository itself when anything outside its layers changed, then each
    /// layer with staged changes. Each is the hash Lore keeps as that
    /// repository's staged anchor, which the next commit builds its revision
    /// from. Empty when nothing changed. In a dry run Lore reports the same
    /// hashes but does not store them, so they are what would have been
    /// staged.
    pub revisions: Vec<(RepositoryId, Revision)>,
}

/// What an [`unstage`](Repository::unstage) did.
#[derive(Debug, Clone)]
pub struct Unstaged {
    /// In the order Lore reported them.
    pub files: Vec<FileChange>,
    /// [`None`] when nothing was staged, in which case Lore reports nothing at
    /// all.
    pub count: Option<UnstageCount>,
    /// The staged tree Lore left for each repository: the repository itself
    /// when something was unstaged and something remains staged, then each
    /// layer a path went into, all zeroes when nothing remains staged in it.
    /// Each is the hash Lore keeps as that repository's staged anchor.
    pub revisions: Vec<(RepositoryId, Revision)>,
}

/// One revision a [`commit`](Repository::commit) created.
#[derive(Debug, Clone)]
pub struct Commit {
    pub repository: RepositoryId,
    pub branch: BranchId,
    pub revision: Revision,
    pub revision_number: u64,
    /// Lore's tally of the files it processed for this revision. [`None`] for
    /// a linked repository, whose files Lore counts in the tally of the
    /// repository linking it.
    pub count: Option<CommitCount>,
    parents: [Revision; 2],
}

impl Commit {
    /// The parent revisions: none for an initial revision, one normally, two
    /// for a merge.
    pub fn parents(&self) -> impl Iterator<Item = Revision> + '_ {
        self.parents
            .iter()
            .copied()
            .filter(|parent| !parent.is_zero())
    }
}

/// Lore's all-zero "none" as [`None`].
fn present(revision: lore_hash_t) -> Option<Revision> {
    Some(Revision::from_raw(revision)).filter(|revision| !revision.is_zero())
}

fn missing(command: &'static str, expected: &'static str) -> LoreError {
    LoreError::MissingEvent { command, expected }
}

impl Repository {
    /// Reports the working tree: the branch and revision it is on, and the
    /// paths that differ from that revision.
    ///
    /// This corresponds to `lore_sys::Lore::lore_repository_status`.
    pub fn status(&self, args: RepositoryStatusArgs<'_>) -> Result<Status, LoreError> {
        const COMMAND: &str = "repository::status";
        let mut status = None;
        let mut files = Vec::new();
        let mut count = None;
        let mut summary = None;

        crate::call::repository_status(self.lore, COMMAND, &self.globals, args, |event| {
            crate::log_event(&event);

            match event {
                Ok(Event::RepositoryStatusRevision {
                    repository,
                    branch,
                    branch_name,
                    revision,
                    revision_number,
                    revision_staged,
                    revision_merged,
                    revision_merged_parent_branch,
                    revision_local,
                    revision_local_number,
                    revision_remote,
                    revision_remote_number,
                    is_local_ahead,
                    is_remote_ahead,
                    remote_available,
                    remote_authorized,
                    remote_branch_exist,
                }) => {
                    status = Some(Status {
                        repository: RepositoryId::from_raw(repository),
                        branch: BranchId::from_raw(branch),
                        branch_name: branch_name.to_owned(),
                        revision: present(revision),
                        revision_number,
                        revision_staged: present(revision_staged),
                        revision_merged: present(revision_merged),
                        revision_merged_parent_branch: present(revision_merged_parent_branch),
                        revision_local: present(revision_local),
                        revision_local_number,
                        revision_remote: present(revision_remote),
                        revision_remote_number,
                        is_local_ahead,
                        is_remote_ahead,
                        remote_available,
                        remote_authorized,
                        remote_branch_exist,
                        files: Vec::new(),
                        count: None,
                        summary: None,
                    });
                }
                Ok(Event::RepositoryStatusFile {
                    path,
                    size,
                    action,
                    kind,
                    staged,
                    merged,
                    conflict,
                    conflict_unresolved,
                    conflict_automerged,
                    conflict_mine,
                    conflict_theirs,
                    dirty,
                    from_path,
                }) => files.push(StatusFile {
                    path: path.to_owned(),
                    size,
                    action: FileAction::from_raw(action),
                    kind: NodeKind::from_raw(kind),
                    staged,
                    merged,
                    conflict,
                    conflict_unresolved,
                    conflict_automerged,
                    conflict_mine,
                    conflict_theirs,
                    dirty,
                    from_path: Some(from_path.to_owned()).filter(|from| !from.is_empty()),
                }),
                Ok(Event::RepositoryStatusCount { directories, files }) => {
                    count = Some(StatusCount { directories, files });
                }
                Ok(Event::RepositoryStatusSummary {
                    adds,
                    deletes,
                    modifies,
                    moves,
                    copies,
                }) => {
                    summary = Some(StatusSummary {
                        adds,
                        deletes,
                        modifies,
                        moves,
                        copies,
                    });
                }
                _ => {}
            }
        })?;

        let mut status = status.ok_or(missing(COMMAND, "repository_status_revision"))?;
        status.files = files;
        status.count = count;
        status.summary = summary;
        Ok(status)
    }

    /// Stages paths for the next commit and reports each file whose staged
    /// state changed. Lore works out the kind of change itself: a rename
    /// arrives as a delete and an add, and a move only for a change of case.
    ///
    /// This corresponds to `lore_sys::Lore::lore_file_stage`.
    pub fn stage(&self, args: FileStageArgs<'_>) -> Result<Staged, LoreError> {
        const COMMAND: &str = "file::stage";
        let mut files = Vec::new();
        let mut count = None;
        let mut revisions = Vec::new();

        crate::call::file_stage(self.lore, COMMAND, &self.globals, args, |event| {
            crate::log_event(&event);

            match event {
                Ok(Event::FileStageFile {
                    path,
                    action,
                    from_path,
                }) => files.push(FileChange {
                    path: path.to_owned(),
                    action: FileAction::from_raw(action),
                    from_path: Some(from_path.to_owned()).filter(|from| !from.is_empty()),
                }),
                Ok(Event::FileStageEnd { count: tally }) => {
                    count = Some(StageCount::from_raw(tally))
                }
                Ok(Event::FileStageRevision {
                    repository,
                    revision,
                }) => revisions.push((
                    RepositoryId::from_raw(repository),
                    Revision::from_raw(revision),
                )),
                _ => {}
            }
        })?;

        Ok(Staged {
            files,
            count: count.ok_or(missing(COMMAND, "file_stage_end"))?,
            revisions,
        })
    }

    /// Takes paths out of the next commit, leaving the working tree as it is,
    /// and reports each path that was staged.
    ///
    /// This corresponds to `lore_sys::Lore::lore_file_unstage`.
    pub fn unstage(&self, args: FileUnstageArgs<'_>) -> Result<Unstaged, LoreError> {
        const COMMAND: &str = "file::unstage";
        let mut files = Vec::new();
        let mut count = None;
        let mut revisions = Vec::new();

        crate::call::file_unstage(self.lore, COMMAND, &self.globals, args, |event| {
            crate::log_event(&event);

            match event {
                Ok(Event::FileUnstageFile { path, action }) => files.push(FileChange {
                    path: path.to_owned(),
                    action: FileAction::from_raw(action),
                    from_path: None,
                }),
                Ok(Event::FileUnstageEnd { count: tally }) => {
                    count = Some(UnstageCount::from_raw(tally));
                }
                Ok(Event::FileUnstageRevision {
                    repository,
                    revision,
                }) => revisions.push((
                    RepositoryId::from_raw(repository),
                    Revision::from_raw(revision),
                )),
                _ => {}
            }
        })?;

        Ok(Unstaged {
            files,
            count,
            revisions,
        })
    }

    /// Commits the staged changes as new revisions on the branch the
    /// instance is on. The revisions are local until [`push`](Self::push).
    ///
    /// One [`Commit`] per repository that got a new revision, in the order
    /// Lore reported them: the linked repositories inside the repository, the
    /// repository itself, then each layer with the linked repositories inside
    /// it first. Empty when [`GlobalArgs::force`] committed nothing.
    ///
    /// This corresponds to `lore_sys::Lore::lore_revision_commit`.
    pub fn commit(&self, args: RevisionCommitArgs<'_>) -> Result<Vec<Commit>, LoreError> {
        const COMMAND: &str = "revision::commit";
        let mut count = None;
        let mut commits = Vec::new();

        crate::call::revision_commit(self.lore, COMMAND, &self.globals, args, |event| {
            crate::log_event(&event);

            match event {
                // Lore counts once per repository or layer, just before its
                // revision. The linked repositories inside it are counted in
                // that tally and report their revisions before it.
                Ok(Event::RevisionCommitEnd { count: tally }) => {
                    count = Some(CommitCount::from_raw(tally));
                }
                Ok(Event::RevisionCommitRevision {
                    repository,
                    branch,
                    revision,
                    revision_number,
                    parents,
                }) => commits.push(Commit {
                    repository: RepositoryId::from_raw(repository),
                    branch: BranchId::from_raw(branch),
                    revision: Revision::from_raw(revision),
                    revision_number,
                    count: count.take(),
                    parents: parents.map(Revision::from_raw),
                }),
                _ => {}
            }
        })?;

        if commits.iter().any(|commit| commit.revision.is_zero()) {
            return Err(LoreError::InvalidResponse {
                command: COMMAND,
                detail: "a committed revision is all zeroes".to_owned(),
            });
        }
        Ok(commits)
    }
}
