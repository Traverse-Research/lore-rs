use crate::LoreStringExt;
use lore_sys::{
    lore_address_t, lore_branch_id_t, lore_context_t, lore_error_code_t, lore_event_t,
    lore_event_tag_t, lore_file_action_t, lore_hash_t, lore_log_level_t, lore_node_id_t,
    lore_repository_id_t, LORE_EVENT_BRANCH_INFO, LORE_EVENT_COMPLETE, LORE_EVENT_END,
    LORE_EVENT_ERROR, LORE_EVENT_FILE_INFO, LORE_EVENT_FILE_STAGE_FILE,
    LORE_EVENT_FILE_UNSTAGE_FILE, LORE_EVENT_LOG, LORE_EVENT_REPOSITORY_DATA,
    LORE_EVENT_REPOSITORY_STATE_DUMP_NODE, LORE_EVENT_REPOSITORY_STATUS_FILE,
    LORE_EVENT_REPOSITORY_STATUS_REVISION, LORE_EVENT_REVISION_COMMIT_REVISION,
    LORE_EVENT_REVISION_TREE_CHILD, LORE_EVENT_REVISION_TREE_CLOSE_COMPLETE,
    LORE_EVENT_REVISION_TREE_INFO, LORE_EVENT_REVISION_TREE_LIST_CHILDREN_BEGIN,
    LORE_EVENT_REVISION_TREE_LOADED, LORE_EVENT_REVISION_TREE_NODE_INFO,
    LORE_EVENT_REVISION_TREE_NODE_PATH, LORE_EVENT_REVISION_TREE_RESOLVE_PATH_COMPLETE,
    LORE_EVENT_STORAGE_GET_DATA, LORE_EVENT_STORAGE_GET_HEADER,
    LORE_EVENT_STORAGE_GET_ITEM_COMPLETE, LORE_EVENT_STORAGE_OPENED, LORE_LOG_LEVEL_ERROR,
    LORE_LOG_LEVEL_INFO, LORE_LOG_LEVEL_TRACE, LORE_LOG_LEVEL_WARN,
};

/// One event decoded from lore's tagged event union. Borrows from the raw
/// event. Not all variants are implemented yet; more will be added as
/// required.
///
/// Variants never store raw Lore types that carry pointers internally, such
/// as `lore_string_t`, to enforce correct lifetimes. Instead those fields are
/// decoded to references (`&str`, `&[u8]`) bound to `'a`.
///
/// Every call ends with [`Self::Complete`] followed by [`Self::End`]. A
/// terminal failure is reported on the completion alone: its `status` and
/// `error_code` are the same value, and no [`Self::Error`] precedes it. An
/// [`Self::Error`] mid-stream is a non-fatal one.
#[non_exhaustive]
pub enum Event<'a> {
    Log {
        level: lore_log_level_t,
        message: &'a str,
    },
    /// A non-fatal error. Its `error_type` is a legacy code that does not
    /// match the completion's `error_code` for most errors.
    Error {
        error_type: u32,
        message: &'a str,
    },
    Complete {
        /// Zero on success, otherwise the same code as `error_code`.
        status: i32,
        /// The error's FFI code; zero on success, `-1` for an internal error.
        /// Lore does not yet promise these values stable.
        error_code: i32,
        /// The failure's message, empty when the call succeeded.
        message: &'a str,
    },
    /// The last event of a call, after [`Self::Complete`]. Nothing arrives
    /// after it.
    End,
    /// The `stack` of branch points the raw event also carries is not decoded:
    /// it is an array, which no variant here has needed yet, and
    /// [`Self::BranchInfo::branch_point`] is its first entry.
    BranchInfo {
        id: lore_branch_id_t,
        name: &'a str,
        category: &'a str,
        /// Tip in the local mutable store, zero when there is none.
        latest: lore_hash_t,
        /// Tip on the remote, zero when there is no remote to ask or nothing
        /// has been pushed.
        latest_remote: lore_hash_t,
        parent: lore_branch_id_t,
        /// Revision on `parent` this branch was created from.
        branch_point: lore_hash_t,
        creator: &'a str,
        created: u64,
        archived: bool,
    },
    FileInfo {
        path: &'a str,
        hash: lore_hash_t,
        context: lore_context_t,
        is_file: bool,
        size: u64,
    },
    FileStageFile {
        path: &'a str,
        action: lore_file_action_t,
    },
    FileUnstageFile {
        path: &'a str,
        action: lore_file_action_t,
    },
    RepositoryData {
        remote_url: &'a str,
        id: lore_repository_id_t,
        name: &'a str,
        description: &'a str,
        default_branch: lore_branch_id_t,
        default_branch_name: &'a str,
        creator: &'a str,
        created: u64,
    },
    RepositoryStatusRevision {
        branch_name: &'a str,
        /// The revision the repository is on.
        revision: lore_hash_t,
    },
    RepositoryStatusFile {
        path: &'a str,
        action: lore_file_action_t,
        staged: bool,
    },
    RepositoryStateDumpNode {
        path: &'a str,
    },
    RevisionCommitRevision {
        revision: [u8; 32],
        revision_number: u64,
    },
    RevisionTreeLoaded {
        handle_id: u64,
    },
    RevisionTreeResolvePathComplete {
        id: u64,
        node_id: lore_node_id_t,
        /// The tree the node belongs to, which differs from the handle's when
        /// the path crossed a sub-repository link.
        repository: lore_repository_id_t,
        revision: lore_hash_t,
        error_code: lore_error_code_t,
    },
    /// Emitted once before the children of a listing, carrying the outcome
    /// and the tree the children belong to.
    RevisionTreeListChildrenBegin {
        id: u64,
        repository: lore_repository_id_t,
        revision: lore_hash_t,
        error_code: lore_error_code_t,
    },
    RevisionTreeChild {
        id: u64,
        node_id: lore_node_id_t,
        name: &'a str,
        parent_id: lore_node_id_t,
        kind: u32,
        mode: u16,
        size: u64,
        address: lore_address_t,
        error_code: lore_error_code_t,
    },
    RevisionTreeNodeInfo {
        id: u64,
        node_id: lore_node_id_t,
        repository: lore_repository_id_t,
        revision: lore_hash_t,
        name: &'a str,
        parent_id: lore_node_id_t,
        kind: u32,
        mode: u16,
        size: u64,
        address: lore_address_t,
        file_id: lore_context_t,
        error_code: lore_error_code_t,
    },
    RevisionTreeNodePath {
        id: u64,
        repository: lore_repository_id_t,
        revision: lore_hash_t,
        /// The path from the root to the node.
        path: &'a str,
        error_code: lore_error_code_t,
    },
    RevisionTreeInfo {
        id: u64,
        repository: lore_repository_id_t,
        revision: lore_hash_t,
        /// The parent revisions, zero where there is none.
        parents: [lore_hash_t; 2],
        creation_timestamp: i64,
        author_identity: &'a str,
        metadata_key_count: u32,
        error_code: lore_error_code_t,
    },
    RevisionTreeCloseComplete {
        id: u64,
        error_code: lore_error_code_t,
    },
    StorageOpened {
        handle_id: u64,
    },
    StorageGetHeader {
        id: u64,
        address: lore_address_t,
        /// Size of the item's reassembled content, before any data arrives.
        size_content: u64,
    },
    StorageGetData {
        /// Which item of the call these bytes belong to.
        id: u64,
        offset: u64,
        bytes: &'a [u8],
    },
    StorageGetItemComplete {
        id: u64,
        /// The item's address on success, zero on failure.
        address: lore_address_t,
        error_code: lore_error_code_t,
    },
    /// Event kinds without a mapped variant (yet), add them here as needed
    Other {
        tag: lore_event_tag_t,
    },
}

impl<'a> Event<'a> {
    /// # Safety
    ///
    /// `event` must be a valid Lore event whose tag matches the union variant
    /// that is actually initialized, with any contained pointers valid for
    /// `lifetime 'a`.
    pub unsafe fn from_raw(event: &'a lore_event_t) -> Result<Self, std::str::Utf8Error> {
        let tag = event.tag;

        unsafe {
            let data = &event.__bindgen_anon_1;
            Ok(match tag {
                LORE_EVENT_LOG => Self::Log {
                    level: data.log.level,
                    message: data.log.message.try_to_str()?,
                },
                LORE_EVENT_ERROR => Self::Error {
                    error_type: data.error.error_type,
                    message: data.error.error_inner.try_to_str()?,
                },
                LORE_EVENT_COMPLETE => Self::Complete {
                    status: data.complete.status,
                    error_code: data.complete.error.error_code,
                    message: data.complete.error.message.try_to_str()?,
                },
                LORE_EVENT_END => Self::End,
                LORE_EVENT_BRANCH_INFO => Self::BranchInfo {
                    id: data.branch_info.id,
                    name: data.branch_info.name.try_to_str()?,
                    category: data.branch_info.category.try_to_str()?,
                    latest: data.branch_info.latest,
                    latest_remote: data.branch_info.latest_remote,
                    parent: data.branch_info.parent,
                    branch_point: data.branch_info.branch_point,
                    creator: data.branch_info.creator.try_to_str()?,
                    created: data.branch_info.created,
                    archived: data.branch_info.archived != 0,
                },
                LORE_EVENT_FILE_INFO => Self::FileInfo {
                    path: data.file_info.path.try_to_str()?,
                    hash: data.file_info.hash,
                    context: data.file_info.context,
                    is_file: data.file_info.is_file != 0,
                    size: data.file_info.size,
                },
                LORE_EVENT_FILE_STAGE_FILE => Self::FileStageFile {
                    path: data.file_stage_file.path.try_to_str()?,
                    action: data.file_stage_file.action,
                },
                LORE_EVENT_FILE_UNSTAGE_FILE => Self::FileUnstageFile {
                    path: data.file_unstage_file.path.try_to_str()?,
                    action: data.file_unstage_file.action,
                },
                LORE_EVENT_REPOSITORY_DATA => Self::RepositoryData {
                    remote_url: data.repository_data.remote_url.try_to_str()?,
                    id: data.repository_data.id,
                    name: data.repository_data.name.try_to_str()?,
                    description: data.repository_data.description.try_to_str()?,
                    default_branch: data.repository_data.default_branch,
                    default_branch_name: data.repository_data.default_branch_name.try_to_str()?,
                    creator: data.repository_data.creator.try_to_str()?,
                    created: data.repository_data.created,
                },
                LORE_EVENT_REPOSITORY_STATUS_REVISION => Self::RepositoryStatusRevision {
                    branch_name: data.repository_status_revision.branch_name.try_to_str()?,
                    revision: data.repository_status_revision.revision,
                },
                LORE_EVENT_REPOSITORY_STATUS_FILE => Self::RepositoryStatusFile {
                    path: data.repository_status_file.path.try_to_str()?,
                    action: data.repository_status_file.action,
                    staged: data.repository_status_file.flag_staged != 0,
                },
                LORE_EVENT_REPOSITORY_STATE_DUMP_NODE => Self::RepositoryStateDumpNode {
                    path: data.repository_state_dump_node.name.try_to_str()?,
                },
                LORE_EVENT_REVISION_COMMIT_REVISION => Self::RevisionCommitRevision {
                    revision: data.revision_commit_revision.revision.data,
                    revision_number: data.revision_commit_revision.revision_number,
                },
                LORE_EVENT_REVISION_TREE_LOADED => Self::RevisionTreeLoaded {
                    handle_id: data.revision_tree_loaded.handle_id,
                },
                LORE_EVENT_REVISION_TREE_RESOLVE_PATH_COMPLETE => {
                    Self::RevisionTreeResolvePathComplete {
                        id: data.revision_tree_resolve_path_complete.id,
                        node_id: data.revision_tree_resolve_path_complete.node_id,
                        repository: data.revision_tree_resolve_path_complete.repository,
                        revision: data.revision_tree_resolve_path_complete.revision,
                        error_code: data.revision_tree_resolve_path_complete.error_code,
                    }
                }
                LORE_EVENT_REVISION_TREE_LIST_CHILDREN_BEGIN => {
                    Self::RevisionTreeListChildrenBegin {
                        id: data.revision_tree_list_children_begin.id,
                        repository: data.revision_tree_list_children_begin.repository,
                        revision: data.revision_tree_list_children_begin.revision,
                        error_code: data.revision_tree_list_children_begin.error_code,
                    }
                }
                LORE_EVENT_REVISION_TREE_CHILD => Self::RevisionTreeChild {
                    id: data.revision_tree_child.id,
                    node_id: data.revision_tree_child.node_id,
                    name: data.revision_tree_child.name.try_to_str()?,
                    parent_id: data.revision_tree_child.parent_id,
                    kind: data.revision_tree_child.kind,
                    mode: data.revision_tree_child.mode,
                    size: data.revision_tree_child.size,
                    address: data.revision_tree_child.address,
                    error_code: data.revision_tree_child.error_code,
                },
                LORE_EVENT_REVISION_TREE_NODE_INFO => Self::RevisionTreeNodeInfo {
                    id: data.revision_tree_node_info.id,
                    node_id: data.revision_tree_node_info.node_id,
                    repository: data.revision_tree_node_info.repository,
                    revision: data.revision_tree_node_info.revision,
                    name: data.revision_tree_node_info.name.try_to_str()?,
                    parent_id: data.revision_tree_node_info.parent_id,
                    kind: data.revision_tree_node_info.kind,
                    mode: data.revision_tree_node_info.mode,
                    size: data.revision_tree_node_info.size,
                    address: data.revision_tree_node_info.address,
                    file_id: data.revision_tree_node_info.file_id,
                    error_code: data.revision_tree_node_info.error_code,
                },
                LORE_EVENT_REVISION_TREE_NODE_PATH => Self::RevisionTreeNodePath {
                    id: data.revision_tree_node_path.id,
                    repository: data.revision_tree_node_path.repository,
                    revision: data.revision_tree_node_path.revision,
                    path: data.revision_tree_node_path.path.try_to_str()?,
                    error_code: data.revision_tree_node_path.error_code,
                },
                LORE_EVENT_REVISION_TREE_INFO => Self::RevisionTreeInfo {
                    id: data.revision_tree_info.id,
                    repository: data.revision_tree_info.repository,
                    revision: data.revision_tree_info.revision,
                    parents: data.revision_tree_info.parent,
                    creation_timestamp: data.revision_tree_info.creation_timestamp,
                    author_identity: data.revision_tree_info.author_identity.try_to_str()?,
                    metadata_key_count: data.revision_tree_info.metadata_key_count,
                    error_code: data.revision_tree_info.error_code,
                },
                LORE_EVENT_REVISION_TREE_CLOSE_COMPLETE => Self::RevisionTreeCloseComplete {
                    id: data.revision_tree_close_complete.id,
                    error_code: data.revision_tree_close_complete.error_code,
                },
                LORE_EVENT_STORAGE_OPENED => Self::StorageOpened {
                    handle_id: data.storage_opened.handle_id,
                },
                LORE_EVENT_STORAGE_GET_HEADER => Self::StorageGetHeader {
                    id: data.storage_get_header.id,
                    address: data.storage_get_header.address,
                    size_content: data.storage_get_header.size_content,
                },
                LORE_EVENT_STORAGE_GET_DATA => Self::StorageGetData {
                    id: data.storage_get_data.id,
                    offset: data.storage_get_data.offset,
                    bytes: std::slice::from_raw_parts(
                        data.storage_get_data.bytes.ptr.cast::<u8>(),
                        data.storage_get_data.bytes.len,
                    ),
                },
                LORE_EVENT_STORAGE_GET_ITEM_COMPLETE => Self::StorageGetItemComplete {
                    id: data.storage_get_item_complete.id,
                    address: data.storage_get_item_complete.address,
                    error_code: data.storage_get_item_complete.error_code,
                },
                _ => Self::Other { tag },
            })
        }
    }
}

/// Forwards an [`Event::Log`] to the `log` crate, ignoring anything else. Call
/// it from a callback to report what Lore says while a call runs; the events a
/// callback does not match on are otherwise dropped.
pub fn log_event(event: &Result<Event<'_>, std::str::Utf8Error>) {
    let Ok(Event::Log { level, message }) = event else {
        return;
    };

    let level = match *level {
        LORE_LOG_LEVEL_ERROR => log::Level::Error,
        LORE_LOG_LEVEL_WARN => log::Level::Warn,
        LORE_LOG_LEVEL_INFO => log::Level::Info,
        LORE_LOG_LEVEL_TRACE => log::Level::Trace,
        _ => log::Level::Debug,
    };

    log::log!(target: "lore", level, "{message}");
}
