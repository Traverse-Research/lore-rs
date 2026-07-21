use crate::as_str;
use lore_sys::{
    lore_error_code_t, lore_event_id_t, lore_event_t, lore_file_action_t, lore_hash_t,
    lore_log_level_t, LORE_EVENT_COMPLETE, LORE_EVENT_ERROR, LORE_EVENT_FILE_INFO,
    LORE_EVENT_FILE_STAGE_FILE, LORE_EVENT_FILE_UNSTAGE_FILE, LORE_EVENT_LOG,
    LORE_EVENT_REPOSITORY_STATE_DUMP_NODE, LORE_EVENT_REPOSITORY_STATUS_FILE,
    LORE_EVENT_REPOSITORY_STATUS_REVISION, LORE_EVENT_REVISION_COMMIT_REVISION,
    LORE_EVENT_STORAGE_GET_DATA, LORE_EVENT_STORAGE_GET_ITEM_COMPLETE, LORE_EVENT_STORAGE_OPENED,
};

/// One event decoded from lore's tagged event union. Borrows from the raw
/// event,
/// Not all variants are currently implemented here, more to be added when required.
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
        hash: lore_hash_t,
        context: [u8; 16],
    },
    FileStageFile {
        path: &'a str,
        action: lore_file_action_t,
    },
    FileUnstageFile {
        path: &'a str,
        action: lore_file_action_t,
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
    Other,
}

impl<'a> Event<'a> {
    /// # Safety
    ///
    /// `event` must be a valid Lore event whose tag matches the union variant
    /// that is actually initialized, with any contained pointers valid for
    /// `lifetime 'a`.
    pub unsafe fn from_raw(event: &'a lore_event_t) -> Self {
        let Ok(tag) = lore_event_id_t::try_from(event.tag) else {
            return Self::Other;
        };

        unsafe {
            let data = &event.__bindgen_anon_1;
            match tag {
                LORE_EVENT_LOG => Self::Log {
                    level: data.log.level,
                    message: as_str(&data.log.message).unwrap_or_default(),
                },
                LORE_EVENT_ERROR => Self::Error {
                    error_type: data.error.error_type,
                    message: as_str(&data.error.error_inner).unwrap_or_default(),
                },
                LORE_EVENT_COMPLETE => Self::Complete {
                    status: data.complete.status,
                },
                LORE_EVENT_FILE_INFO => Self::FileInfo {
                    hash: data.file_info.hash,
                    context: data.file_info.context.data,
                },
                LORE_EVENT_FILE_STAGE_FILE => Self::FileStageFile {
                    path: as_str(&data.file_stage_file.path).unwrap_or_default(),
                    action: data.file_stage_file.action,
                },
                LORE_EVENT_FILE_UNSTAGE_FILE => Self::FileUnstageFile {
                    path: as_str(&data.file_unstage_file.path).unwrap_or_default(),
                    action: data.file_unstage_file.action,
                },
                LORE_EVENT_REPOSITORY_STATUS_REVISION => Self::RepositoryStatusRevision {
                    branch_name: as_str(&data.repository_status_revision.branch_name)
                        .unwrap_or_default(),
                },
                LORE_EVENT_REPOSITORY_STATUS_FILE => Self::RepositoryStatusFile {
                    path: as_str(&data.repository_status_file.path).unwrap_or_default(),
                    action: data.repository_status_file.action,
                    staged: data.repository_status_file.flag_staged != 0,
                },
                LORE_EVENT_REPOSITORY_STATE_DUMP_NODE => Self::RepositoryStateDumpNode {
                    path: as_str(&data.repository_state_dump_node.name).unwrap_or_default(),
                },
                LORE_EVENT_REVISION_COMMIT_REVISION => Self::RevisionCommitRevision {
                    revision: data.revision_commit_revision.revision.data,
                    revision_number: data.revision_commit_revision.revision_number,
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
                _ => Self::Other,
            }
        }
    }
}
