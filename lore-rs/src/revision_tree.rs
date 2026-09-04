use lore_sys::{
    lore_hash_t, lore_repository_id_t, lore_revision_tree_close_args_t,
    lore_revision_tree_info_args_t, lore_revision_tree_list_children_args_t,
    lore_revision_tree_load_args_t, lore_revision_tree_node_info_args_t,
    lore_revision_tree_node_path_args_t, lore_revision_tree_t,
};

use crate::{
    Address, Event, GlobalArgs, Lore, LoreError, NodeId, NodeKind, RepositoryId, Revision,
    RevisionTreeResolvePathArgs, Store,
};

/// One node of a revision tree.
#[derive(Debug, Clone, Copy)]
pub struct Node {
    pub id: NodeId,
    pub address: Address,
    pub size: u64,
    pub kind: NodeKind,
    /// Lore's `mode` bits.
    pub mode: u16,
}

/// One entry of a directory listing.
#[derive(Debug, Clone)]
pub struct Child {
    pub id: NodeId,
    pub name: String,
    pub parent: NodeId,
    pub kind: NodeKind,
    /// Lore's `mode` bits.
    pub mode: u16,
    pub size: u64,
    pub address: Address,
}

/// The loaded revision itself, as opposed to its nodes.
#[derive(Debug, Clone)]
pub struct TreeInfo {
    pub repository: RepositoryId,
    pub revision: Revision,
    parents: [Revision; 2],
    /// Lore's creation timestamp.
    pub created: i64,
    /// Identity of the author.
    pub author: String,
    pub metadata_key_count: u32,
}

impl TreeInfo {
    /// The parent revisions: none for an initial revision, one normally, two
    /// for a merge.
    pub fn parents(&self) -> impl Iterator<Item = Revision> + '_ {
        self.parents
            .iter()
            .copied()
            .filter(|parent| !parent.is_zero())
    }
}

/// A loaded revision tree, closed on drop.
///
/// The handle carries the revision's deserialized state, so it *is* that
/// revision: no lookup through it can reach another one. Every lookup is
/// answered from that state — no filesystem read, no server round trip once
/// the blocks are local.
///
/// Sub-repository links are not followed: a path that crosses one resolves in
/// the link's target tree, whose node ids mean nothing to this handle, so such
/// a result is refused as [`LoreError::InvalidResponse`] rather than handed
/// out. `Send + Sync`; Lore allows concurrent operations on one handle.
pub struct RevisionTree {
    lore: &'static Lore,
    globals: GlobalArgs,
    handle: lore_revision_tree_t,
    repository: RepositoryId,
    revision: Revision,
}

pub(crate) fn load(
    store: &Store,
    repository: RepositoryId,
    revision: Revision,
) -> Result<RevisionTree, LoreError> {
    const COMMAND: &str = "revision_tree::load";
    let lore = store.lore();
    let globals = store.globals().clone();
    let mut handle = None;

    crate::call::revision_tree_load(
        lore,
        COMMAND,
        &globals,
        lore_revision_tree_load_args_t {
            store: store.handle(),
            repository: repository.to_raw(),
            revision_hash: revision.to_raw(),
        },
        |event| {
            crate::log_event(&event);

            if let Ok(Event::RevisionTreeLoaded { handle_id }) = event {
                handle = Some(lore_revision_tree_t { handle_id });
            }
        },
    )?;

    Ok(RevisionTree {
        lore,
        globals,
        handle: handle.ok_or(LoreError::MissingEvent {
            command: COMMAND,
            expected: "revision_tree_loaded",
        })?,
        repository,
        revision,
    })
}

impl RevisionTree {
    pub fn repository(&self) -> RepositoryId {
        self.repository
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn lore(&self) -> &'static Lore {
        self.lore
    }

    /// The globals the store this tree was loaded through was opened with,
    /// which the tree's own calls use.
    pub fn globals(&self) -> &GlobalArgs {
        &self.globals
    }

    /// The raw handle, for a call this crate does not wrap.
    pub fn handle(&self) -> lore_revision_tree_t {
        self.handle
    }

    /// The root directory, where a walk with [`Self::children`] starts.
    pub fn root(&self) -> NodeId {
        NodeId::ROOT
    }

    /// Resolves a path relative to the tree root. Empty is the root itself.
    ///
    /// A path the revision does not hold fails the call outright — Lore folds
    /// the whole not-found family into
    /// [`ErrorCode::InvalidArguments`](crate::ErrorCode::InvalidArguments) —
    /// rather than reporting an absence.
    ///
    /// This corresponds to `lore_sys::Lore::lore_revision_tree_resolve_path`.
    pub fn resolve_path(&self, path: &str) -> Result<NodeId, LoreError> {
        const COMMAND: &str = "revision_tree::resolve_path";
        let mut outcome = None;

        let result = crate::call::revision_tree_resolve_path(
            self.lore,
            COMMAND,
            &self.globals,
            RevisionTreeResolvePathArgs {
                id: 0,
                handle: self.handle,
                path,
            },
            |event| {
                crate::log_event(&event);

                if let Ok(Event::RevisionTreeResolvePathComplete {
                    node_id,
                    repository,
                    revision,
                    error_code,
                    ..
                }) = event
                {
                    outcome = Some((node_id, repository, revision, error_code));
                }
            },
        );
        LoreError::resolve(COMMAND, result, outcome.map(|(.., code)| code))?;

        let (node_id, repository, revision, _) = outcome.ok_or(LoreError::MissingEvent {
            command: COMMAND,
            expected: "revision_tree_resolve_path_complete",
        })?;
        self.expect_own_tree(COMMAND, repository, revision)?;

        Ok(NodeId(node_id))
    }

    /// Reports one node.
    ///
    /// This corresponds to `lore_sys::Lore::lore_revision_tree_node_info`.
    pub fn node(&self, id: NodeId) -> Result<Node, LoreError> {
        const COMMAND: &str = "revision_tree::node_info";
        let mut outcome = None;

        let result = crate::call::revision_tree_node_info(
            self.lore,
            COMMAND,
            &self.globals,
            lore_revision_tree_node_info_args_t {
                id: 0,
                handle: self.handle,
                node_id: id.0,
            },
            |event| {
                crate::log_event(&event);

                if let Ok(Event::RevisionTreeNodeInfo {
                    kind,
                    mode,
                    size,
                    address,
                    error_code,
                    ..
                }) = event
                {
                    outcome = Some((kind, mode, size, address, error_code));
                }
            },
        );
        LoreError::resolve(COMMAND, result, outcome.map(|(.., code)| code))?;

        let (kind, mode, size, address, _) = outcome.ok_or(LoreError::MissingEvent {
            command: COMMAND,
            expected: "revision_tree_node_info",
        })?;

        Ok(Node {
            id,
            address: Address::from_raw(address),
            size,
            kind: NodeKind::from_raw(kind),
            mode,
        })
    }

    /// [`Self::resolve_path`] then [`Self::node`].
    pub fn node_at(&self, path: &str) -> Result<Node, LoreError> {
        self.node(self.resolve_path(path)?)
    }

    /// Lists a directory. A directory that is empty lists as an empty `Vec`;
    /// a node that is not a directory fails.
    ///
    /// This corresponds to `lore_sys::Lore::lore_revision_tree_list_children`.
    pub fn children(&self, parent: NodeId) -> Result<Vec<Child>, LoreError> {
        const COMMAND: &str = "revision_tree::list_children";
        let mut begin = None;
        let mut children = Vec::new();

        let result = crate::call::revision_tree_list_children(
            self.lore,
            COMMAND,
            &self.globals,
            lore_revision_tree_list_children_args_t {
                id: 0,
                handle: self.handle,
                parent_node_id: parent.0,
            },
            |event| {
                crate::log_event(&event);

                match event {
                    Ok(Event::RevisionTreeListChildrenBegin {
                        repository,
                        revision,
                        error_code,
                        ..
                    }) => begin = Some((repository, revision, error_code)),
                    Ok(Event::RevisionTreeChild {
                        node_id,
                        name,
                        parent_id,
                        kind,
                        mode,
                        size,
                        address,
                        ..
                    }) => children.push(Child {
                        id: NodeId(node_id),
                        name: name.to_owned(),
                        parent: NodeId(parent_id),
                        kind: NodeKind::from_raw(kind),
                        mode,
                        size,
                        address: Address::from_raw(address),
                    }),
                    _ => {}
                }
            },
        );
        LoreError::resolve(COMMAND, result, begin.map(|(.., code)| code))?;

        let (repository, revision, _) = begin.ok_or(LoreError::MissingEvent {
            command: COMMAND,
            expected: "revision_tree_list_children_begin",
        })?;
        self.expect_own_tree(COMMAND, repository, revision)?;

        Ok(children)
    }

    /// The path of a node from the root, reconstructed by walking its
    /// parents. For display and logging; a walk that needs paths for every
    /// node is cheaper building them from [`Child::name`].
    ///
    /// This corresponds to `lore_sys::Lore::lore_revision_tree_node_path`.
    pub fn path_of(&self, id: NodeId) -> Result<String, LoreError> {
        const COMMAND: &str = "revision_tree::node_path";
        let mut outcome = None;

        let result = crate::call::revision_tree_node_path(
            self.lore,
            COMMAND,
            &self.globals,
            lore_revision_tree_node_path_args_t {
                id: 0,
                handle: self.handle,
                node_id: id.0,
            },
            |event| {
                crate::log_event(&event);

                if let Ok(Event::RevisionTreeNodePath {
                    path, error_code, ..
                }) = event
                {
                    outcome = Some((path.to_owned(), error_code));
                }
            },
        );
        LoreError::resolve(COMMAND, result, outcome.as_ref().map(|(_, code)| *code))?;

        let (path, _) = outcome.ok_or(LoreError::MissingEvent {
            command: COMMAND,
            expected: "revision_tree_node_path",
        })?;

        Ok(path)
    }

    /// Reports the loaded revision: its parents, author and creation time.
    ///
    /// This corresponds to `lore_sys::Lore::lore_revision_tree_info`.
    pub fn info(&self) -> Result<TreeInfo, LoreError> {
        const COMMAND: &str = "revision_tree::info";
        let mut outcome = None;

        let result = crate::call::revision_tree_info(
            self.lore,
            COMMAND,
            &self.globals,
            lore_revision_tree_info_args_t {
                id: 0,
                handle: self.handle,
            },
            |event| {
                crate::log_event(&event);

                if let Ok(Event::RevisionTreeInfo {
                    repository,
                    revision,
                    parents,
                    creation_timestamp,
                    author_identity,
                    metadata_key_count,
                    error_code,
                    ..
                }) = event
                {
                    outcome = Some((
                        TreeInfo {
                            repository: RepositoryId::from_raw(repository),
                            revision: Revision::from_raw(revision),
                            parents: [
                                Revision::from_raw(parents[0]),
                                Revision::from_raw(parents[1]),
                            ],
                            created: creation_timestamp,
                            author: author_identity.to_owned(),
                            metadata_key_count,
                        },
                        error_code,
                    ));
                }
            },
        );
        LoreError::resolve(COMMAND, result, outcome.as_ref().map(|(_, code)| *code))?;

        let (info, _) = outcome.ok_or(LoreError::MissingEvent {
            command: COMMAND,
            expected: "revision_tree_info",
        })?;

        Ok(info)
    }

    /// Refuses a result that lives in another tree, which is what a path
    /// through a sub-repository link produces.
    fn expect_own_tree(
        &self,
        command: &'static str,
        repository: lore_repository_id_t,
        revision: lore_hash_t,
    ) -> Result<(), LoreError> {
        let repository = RepositoryId::from_raw(repository);
        let revision = Revision::from_raw(revision);
        if repository == self.repository && revision == self.revision {
            return Ok(());
        }
        Err(LoreError::InvalidResponse {
            command,
            detail: format!(
                "the result lives in revision {revision} of repository {repository}, not in \
                 this tree's revision {} of {}; the path crosses a sub-repository link, which \
                 this crate does not follow",
                self.revision, self.repository
            ),
        })
    }
}

impl Drop for RevisionTree {
    /// Closes the handle, which also releases the reference it holds on its
    /// store inside Lore. A failure to close is logged, since nothing can act
    /// on it here.
    fn drop(&mut self) {
        let result = crate::call::revision_tree_close(
            self.lore,
            "revision_tree::close",
            &self.globals,
            lore_revision_tree_close_args_t {
                id: 0,
                handle: self.handle,
            },
            |event| crate::log_event(&event),
        );
        if let Err(error) = result {
            log::error!(target: "lore", "{error}");
        }
    }
}
