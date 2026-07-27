use crate::LoreStringExt;
use lore_sys::{
    lore_address_t, lore_branch_id_t, lore_context_t, lore_error_code_t, lore_event_id_t,
    lore_event_t, lore_event_tag_t, lore_file_action_t, lore_hash_t, lore_log_level_t,
    lore_node_id_t, lore_repository_id_t, LORE_EVENT_COMPLETE, LORE_EVENT_ERROR,
    LORE_EVENT_FILE_INFO, LORE_EVENT_FILE_STAGE_FILE, LORE_EVENT_FILE_UNSTAGE_FILE, LORE_EVENT_LOG,
    LORE_EVENT_REPOSITORY_DATA, LORE_EVENT_REPOSITORY_STATE_DUMP_NODE,
    LORE_EVENT_REPOSITORY_STATUS_FILE, LORE_EVENT_REPOSITORY_STATUS_REVISION,
    LORE_EVENT_REVISION_COMMIT_REVISION, LORE_EVENT_REVISION_TREE_CHILD,
    LORE_EVENT_REVISION_TREE_LOADED, LORE_EVENT_REVISION_TREE_NODE_INFO,
    LORE_EVENT_REVISION_TREE_RESOLVE_PATH_COMPLETE, LORE_EVENT_STORAGE_GET_DATA,
    LORE_EVENT_STORAGE_GET_ITEM_COMPLETE, LORE_EVENT_STORAGE_OPENED,
};

/// One event decoded from lore's tagged event union. Borrows from the raw
/// event. Not all variants are implemented yet; more will be added as
/// required.
///
/// Variants never store raw Lore types that carry pointers internally, such
/// as `lore_string_t` to enforce correct lifetimes. Instead those fields are decoded to
/// references (`&str`, `&[u8]`) bound to `'a`.
#[non_exhaustive]
pub enum Event<'a> {
    Log {
        level: lore_log_level_t,
        message: &'a str,
    },
    Error {
        error_type: u32,
        message: &'a str,
    },
    Complete {
        status: i32,
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
    StorageOpened {
        handle_id: u64,
    },
    StorageGetData {
        offset: u64,
        bytes: &'a [u8],
    },
    StorageGetItemComplete {
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
            Ok(match tag as lore_event_id_t {
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
                LORE_EVENT_STORAGE_OPENED => Self::StorageOpened {
                    handle_id: data.storage_opened.handle_id,
                },
                LORE_EVENT_STORAGE_GET_DATA => Self::StorageGetData {
                    offset: data.storage_get_data.offset,
                    bytes: std::slice::from_raw_parts(
                        data.storage_get_data.bytes.ptr.cast::<u8>(),
                        data.storage_get_data.bytes.len,
                    ),
                },
                LORE_EVENT_STORAGE_GET_ITEM_COMPLETE => Self::StorageGetItemComplete {
                    error_code: data.storage_get_item_complete.error_code,
                },
                _ => Self::Other { tag },
            })
        }
    }
}
