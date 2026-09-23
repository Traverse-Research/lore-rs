use lore_sys::{
    lore_branch_location_t, lore_file_action_t, lore_file_stage_count_data_t,
    lore_file_unstage_count_data_t, lore_hash_t, lore_repository_clone_count_data_t,
    lore_revision_commit_count_data_t, LORE_BRANCH_LOCATION_LOCAL, LORE_BRANCH_LOCATION_REMOTE,
    LORE_FILE_ACTION_ADD, LORE_FILE_ACTION_COPY, LORE_FILE_ACTION_DELETE, LORE_FILE_ACTION_KEEP,
    LORE_FILE_ACTION_MOVE,
};

use crate::{
    BranchArchiveArgs, BranchCreateArgs, BranchId, BranchInfoArgs, BranchListArgs, BranchPushArgs,
    BranchSwitchArgs, Event, FileStageArgs, FileUnstageArgs, GlobalArgs, Lore, LoreError, NodeKind,
    RepositoryCloneArgs, RepositoryId, RepositoryInfoArgs, RepositoryStatusArgs, Revision,
    RevisionCommitArgs, RevisionHistoryArgs, RevisionInfoArgs, RevisionSyncArgs,
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
            BranchInfoArgs { branch: name },
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
    /// Seconds since the Unix epoch.
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

/// Which side a branch was found on: `lore_branch_location_t` as an enum,
/// with [`Self::Other`] for the reason [`FileAction`] has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BranchLocation {
    /// The instance's local store.
    Local,
    /// The server.
    Remote,
    Other(lore_branch_location_t),
}

impl BranchLocation {
    fn from_raw(location: lore_branch_location_t) -> Self {
        match location {
            LORE_BRANCH_LOCATION_LOCAL => Self::Local,
            LORE_BRANCH_LOCATION_REMOTE => Self::Remote,
            other => Self::Other(other),
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

/// Lore's tally of a [`clone`](Lore::repository_clone),
/// `lore_repository_clone_count_data_t`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CloneCount {
    /// Files finished.
    pub file_complete: u64,
    /// Files kept because they already matched.
    pub file_retain: u64,
    pub file_replace: u64,
    /// Files discovered to process.
    pub file_count: u64,
    /// Files still being processed.
    pub file_inflight: u64,
    /// Fragment fetches still in flight.
    pub fragment_inflight: u64,
    pub bytes_transferred: u64,
    pub bytes_total: u64,
    /// Lore had found every file to process.
    pub discovery_complete: bool,
}

impl CloneCount {
    fn from_raw(count: lore_repository_clone_count_data_t) -> Self {
        Self {
            file_complete: count.file_complete,
            file_retain: count.file_retain,
            file_replace: count.file_replace,
            file_count: count.file_count,
            file_inflight: count.file_inflight,
            fragment_inflight: count.fragment_inflight,
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
    /// The new staged state per repository: the repository itself when
    /// anything outside its layers changed, then each layer a path went into.
    /// Empty when nothing changed.
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
    /// The staged state left behind; [`None`] when nothing was unstaged or
    /// nothing remains staged.
    pub revision: Option<(RepositoryId, Revision)>,
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

/// What a [`push`](Repository::push) of one repository found on both sides.
#[derive(Debug, Clone)]
pub struct Push {
    pub remote: String,
    pub repository: RepositoryId,
    pub branch: BranchId,
    pub branch_name: String,
    /// [`None`] when the remote does not have the branch yet.
    pub remote_revision: Option<Revision>,
    pub local_revision: Option<Revision>,
    /// Revisions on the remote since the two sides last agreed.
    pub remote_history: u64,
    /// Revisions the push sends.
    pub local_history: u64,
    /// The remote already held everything; nothing was sent.
    pub already_pushed: bool,
    /// The branch is the repository's default branch.
    pub is_default: bool,
    /// The repository is a linked repository.
    pub is_link: bool,
    /// The repository is a layer.
    pub is_layer: bool,
}

/// What a [`push`](Repository::push) did.
#[derive(Debug, Clone)]
pub struct Pushed {
    /// One per repository pushed, in the order Lore reported them: the
    /// repository itself, then each layer, then each linked repository. Lore
    /// also pushes a linked repository for every pushed revision that links
    /// it, so one can appear more than once, usually with
    /// [`already_pushed`](Push::already_pushed) set on the repeats.
    pub pushes: Vec<Push>,
    /// One per revision uploaded. Lore does not say which repository each
    /// one is for.
    pub uploads: Vec<Uploaded>,
}

/// The fragments a [`push`](Repository::push) uploaded for one revision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Uploaded {
    pub fragments: u64,
    pub bytes_transferred: u64,
}

/// The branch a [`switch`](Repository::switch) landed on.
#[derive(Debug, Clone)]
pub struct Switched {
    pub id: BranchId,
    pub name: String,
    /// Tip in the local store; [`None`] when the instance never had the
    /// branch before.
    pub latest_local: Option<Revision>,
    /// Tip on the remote; [`None`] when there is none.
    pub latest_remote: Option<Revision>,
    /// The revision the working tree is now on; [`None`] when the branch has
    /// none.
    pub revision: Option<Revision>,
    /// Which tip the switch went to: the local store's, or the server's,
    /// which is then the local tip too and is what `latest_local` holds.
    pub location: BranchLocation,
}

/// What a [`sync`](Repository::sync) resolved and did.
#[derive(Debug, Clone)]
pub struct Sync {
    pub remote: String,
    pub repository: RepositoryId,
    pub branch: BranchId,
    pub branch_name: String,
    /// The revision the working tree was on; [`None`] on an empty branch.
    pub source_revision: Option<Revision>,
    pub source_revision_number: u64,
    /// The revision the sync resolved to; [`None`] on an empty branch.
    pub target_revision: Option<Revision>,
    pub target_revision_number: u64,
    /// The target is the branch tip.
    pub is_latest: bool,
    /// The target came from the local store rather than the remote.
    pub local: bool,
    /// Where the working tree ended up; [`None`] when it was already on the
    /// target and there was nothing to do.
    pub revision: Option<SyncRevision>,
}

/// The revision a [`sync`](Repository::sync) left the working tree on.
#[derive(Debug, Clone)]
pub struct SyncRevision {
    pub branch: BranchId,
    pub revision: Revision,
    pub revision_number: u64,
    /// The revision is a merge Lore made to reconcile divergent history.
    pub merge: bool,
    /// The merge left conflicts for the caller to resolve.
    pub conflict: bool,
}

/// A branch a [`create_branch`](Repository::create_branch) made in one
/// repository.
#[derive(Debug, Clone)]
pub struct BranchCreated {
    pub name: String,
    /// The revision the new branch points at.
    pub latest: Revision,
    /// Lore made a new revision for the branch, which happens when the
    /// repository has linked repositories that needed their own branch.
    pub is_commit: bool,
}

/// One branch from [`Repository::branches`].
#[derive(Debug, Clone)]
pub struct BranchEntry {
    /// Which side's view of the branch this is. A branch known on both sides
    /// is listed once for each; an archived one is listed as local.
    pub location: BranchLocation,
    pub id: BranchId,
    pub name: String,
    pub category: String,
    /// Tip on the side [`Self::location`] names; [`None`] when it has none.
    pub latest: Option<Revision>,
    /// The branch points this branch was created from, as parent branch and
    /// the revision on it.
    pub stack: Vec<(BranchId, Revision)>,
    pub creator: String,
    /// Lore's timestamp, which is milliseconds since the Unix epoch for a
    /// branch created by a client and seconds for one created on a server.
    pub created: u64,
    /// The instance is on this branch.
    pub is_current: bool,
    pub archived: bool,
}

/// A revision history, from [`Repository::history`].
#[derive(Debug, Clone)]
pub struct History {
    pub repository: RepositoryId,
    pub branch: BranchId,
    /// Newest first.
    pub entries: Vec<HistoryEntry>,
}

/// One revision of a [`History`].
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub revision: Revision,
    pub revision_number: u64,
    parents: [Revision; 2],
}

impl HistoryEntry {
    /// The parent revisions: none for an initial revision, one normally, two
    /// for a merge.
    pub fn parents(&self) -> impl Iterator<Item = Revision> + '_ {
        self.parents
            .iter()
            .copied()
            .filter(|parent| !parent.is_zero())
    }
}

/// What a [`clone`](Lore::repository_clone) checked out.
#[derive(Debug, Clone)]
pub struct Cloned {
    pub branch: String,
    /// [`None`] when the branch has no revision yet.
    pub revision: Option<Revision>,
    pub count: CloneCount,
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
        let mut revision = None;

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
                    revision: staged,
                }) => {
                    revision = Some((
                        RepositoryId::from_raw(repository),
                        Revision::from_raw(staged),
                    ));
                }
                _ => {}
            }
        })?;

        Ok(Unstaged {
            files,
            count,
            revision,
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

    /// Pushes a branch and its local revisions to the remote, creating the
    /// branch there on its first push.
    ///
    /// This corresponds to `lore_sys::Lore::lore_branch_push`.
    pub fn push(&self, args: BranchPushArgs<'_>) -> Result<Pushed, LoreError> {
        const COMMAND: &str = "branch::push";
        let mut pushes = Vec::new();
        let mut uploads = Vec::new();

        crate::call::branch_push(self.lore, COMMAND, &self.globals, args, |event| {
            crate::log_event(&event);

            match event {
                Ok(Event::BranchPush {
                    remote,
                    repository,
                    branch,
                    branch_name,
                    remote_revision,
                    local_revision,
                    remote_history,
                    local_history,
                    already_pushed,
                    is_default,
                    is_link,
                    is_layer,
                }) => pushes.push(Push {
                    remote: remote.to_owned(),
                    repository: RepositoryId::from_raw(repository),
                    branch: BranchId::from_raw(branch),
                    branch_name: branch_name.to_owned(),
                    remote_revision: present(remote_revision),
                    local_revision: present(local_revision),
                    remote_history,
                    local_history,
                    already_pushed,
                    is_default,
                    is_link,
                    is_layer,
                }),
                Ok(Event::BranchPushFragmentEnd {
                    fragments,
                    bytes_transferred,
                }) => uploads.push(Uploaded {
                    fragments,
                    bytes_transferred,
                }),
                _ => {}
            }
        })?;

        if pushes.is_empty() {
            return Err(missing(COMMAND, "branch_push"));
        }
        Ok(Pushed { pushes, uploads })
    }

    /// Switches the working tree to another branch.
    ///
    /// This corresponds to `lore_sys::Lore::lore_branch_switch`.
    pub fn switch(&self, args: BranchSwitchArgs<'_>) -> Result<Switched, LoreError> {
        const COMMAND: &str = "branch::switch";
        let mut switched = None;

        crate::call::branch_switch(self.lore, COMMAND, &self.globals, args, |event| {
            crate::log_event(&event);

            if let Ok(Event::BranchSwitchEnd {
                id,
                name,
                latest_local,
                latest_remote,
                revision,
                location,
            }) = event
            {
                switched = Some(Switched {
                    id: BranchId::from_raw(id),
                    name: name.to_owned(),
                    latest_local: present(latest_local),
                    latest_remote: present(latest_remote),
                    revision: present(revision),
                    location: BranchLocation::from_raw(location),
                });
            }
        })?;

        switched.ok_or(missing(COMMAND, "branch_switch_end"))
    }

    /// Synchronizes the working tree to a revision, merging when the branch
    /// has diverged from it.
    ///
    /// This corresponds to `lore_sys::Lore::lore_revision_sync`.
    pub fn sync(&self, args: RevisionSyncArgs<'_>) -> Result<Sync, LoreError> {
        const COMMAND: &str = "revision::sync";
        let mut sync = None;
        let mut synced = None;

        crate::call::revision_sync(self.lore, COMMAND, &self.globals, args, |event| {
            crate::log_event(&event);

            match event {
                Ok(Event::RevisionSyncTarget {
                    remote,
                    repository,
                    branch,
                    branch_name,
                    source_revision,
                    source_revision_number,
                    target_revision,
                    target_revision_number,
                    is_latest,
                    local,
                }) => {
                    sync = Some(Sync {
                        remote: remote.to_owned(),
                        repository: RepositoryId::from_raw(repository),
                        branch: BranchId::from_raw(branch),
                        branch_name: branch_name.to_owned(),
                        source_revision: present(source_revision),
                        source_revision_number,
                        target_revision: present(target_revision),
                        target_revision_number,
                        is_latest,
                        local,
                        revision: None,
                    });
                }
                Ok(Event::RevisionSyncRevision {
                    branch,
                    revision,
                    revision_number,
                    merge,
                    conflict,
                }) => {
                    synced = Some(SyncRevision {
                        branch: BranchId::from_raw(branch),
                        revision: Revision::from_raw(revision),
                        revision_number,
                        merge,
                        conflict,
                    });
                }
                _ => {}
            }
        })?;

        let mut sync = sync.ok_or(missing(COMMAND, "revision_sync_target"))?;
        sync.revision = synced;
        Ok(sync)
    }

    /// Creates a branch at the revision the instance is on and moves the
    /// instance onto it, leaving the files as they are.
    ///
    /// One [`BranchCreated`] for the repository itself, then one per layer,
    /// all with the same name; Lore does not say which is which. Empty when
    /// the branch has no revision to point at, or when Lore restored an
    /// archived branch instead, neither of which it reports.
    ///
    /// This corresponds to `lore_sys::Lore::lore_branch_create`.
    pub fn create_branch(
        &self,
        args: BranchCreateArgs<'_>,
    ) -> Result<Vec<BranchCreated>, LoreError> {
        let mut created = Vec::new();

        crate::call::branch_create(self.lore, "branch::create", &self.globals, args, |event| {
            crate::log_event(&event);

            if let Ok(Event::BranchCreate {
                name,
                latest,
                is_commit,
            }) = event
            {
                created.push(BranchCreated {
                    name: name.to_owned(),
                    latest: Revision::from_raw(latest),
                    is_commit,
                });
            }
        })?;

        Ok(created)
    }

    /// Archives a branch, which is what Lore has in place of deleting one: it
    /// stops being listed unless archived branches are asked for, and its
    /// history stays. Returns the name of the branch archived, which is how
    /// to tell which branch an id named.
    ///
    /// This corresponds to `lore_sys::Lore::lore_branch_archive`.
    pub fn archive_branch(&self, args: BranchArchiveArgs<'_>) -> Result<String, LoreError> {
        const COMMAND: &str = "branch::archive";
        let mut archived = None;

        crate::call::branch_archive(self.lore, COMMAND, &self.globals, args, |event| {
            crate::log_event(&event);
            if let Ok(Event::BranchArchive { name }) = event {
                archived = Some(name.to_owned());
            }
        })?;

        archived.ok_or(missing(COMMAND, "branch_archive"))
    }

    /// Lists the branches the instance and its remote know.
    ///
    /// This corresponds to `lore_sys::Lore::lore_branch_list`.
    pub fn branches(&self, args: BranchListArgs) -> Result<Vec<BranchEntry>, LoreError> {
        let mut branches = Vec::new();

        crate::call::branch_list(self.lore, "branch::list", &self.globals, args, |event| {
            crate::log_event(&event);

            if let Ok(Event::BranchListEntry {
                location,
                id,
                name,
                category,
                latest,
                stack,
                creator,
                created,
                is_current,
                archived,
            }) = event
            {
                branches.push(BranchEntry {
                    location: BranchLocation::from_raw(location),
                    id: BranchId::from_raw(id),
                    name: name.to_owned(),
                    category: category.to_owned(),
                    latest: present(latest),
                    stack: stack
                        .iter()
                        .map(|point| {
                            (
                                BranchId::from_raw(point.branch),
                                Revision::from_raw(point.revision),
                            )
                        })
                        .collect(),
                    creator: creator.to_owned(),
                    created,
                    is_current,
                    archived,
                });
            }
        })?;

        Ok(branches)
    }

    /// Walks the history back from a revision along its first parents.
    ///
    /// [`None`] when there is no revision to list: Lore reports the repository
    /// and branch only along with the first entry.
    ///
    /// This corresponds to `lore_sys::Lore::lore_revision_history`.
    pub fn history(&self, args: RevisionHistoryArgs<'_>) -> Result<Option<History>, LoreError> {
        let mut history = None;
        let mut entries = Vec::new();

        crate::call::revision_history(
            self.lore,
            "revision::history",
            &self.globals,
            args,
            |event| {
                crate::log_event(&event);

                match event {
                    Ok(Event::RevisionHistory { repository, branch }) => {
                        history = Some((
                            RepositoryId::from_raw(repository),
                            BranchId::from_raw(branch),
                        ));
                    }
                    Ok(Event::RevisionHistoryEntry {
                        revision,
                        revision_number,
                        parents,
                    }) => entries.push(HistoryEntry {
                        revision: Revision::from_raw(revision),
                        revision_number,
                        parents: parents.map(Revision::from_raw),
                    }),
                    _ => {}
                }
            },
        )?;

        Ok(history.map(|(repository, branch)| History {
            repository,
            branch,
            entries,
        }))
    }
}

impl Lore {
    /// Clones a repository into `globals.repository_path`. Afterwards a
    /// [`Repository`] with the same `globals` runs commands against the clone.
    ///
    /// This corresponds to `lore_sys::Lore::lore_repository_clone`.
    pub fn repository_clone(
        &self,
        globals: &GlobalArgs,
        args: RepositoryCloneArgs<'_>,
    ) -> Result<Cloned, LoreError> {
        const COMMAND: &str = "repository::clone";
        let mut cloned = None;

        crate::call::repository_clone(self, COMMAND, globals, args, |event| {
            crate::log_event(&event);

            if let Ok(Event::RepositoryCloneEnd {
                branch,
                revision,
                count,
            }) = event
            {
                cloned = Some(Cloned {
                    branch: branch.to_owned(),
                    revision: present(revision),
                    count: CloneCount::from_raw(count),
                });
            }
        })?;

        cloned.ok_or(missing(COMMAND, "repository_clone_end"))
    }
}
