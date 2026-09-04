use crate::{
    BranchId, BranchInfoArgs, Event, GlobalArgs, Lore, LoreError, RepositoryId, RepositoryInfoArgs,
    Revision, RevisionInfoArgs,
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
