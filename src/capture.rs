//! Decodes the events delivered through the [`lore_event_callback_config_t`]
//! of a lore call.

use crate::{
    lore_complete_event_data_t, lore_error_event_data_t, lore_event_id_t, lore_event_t,
    lore_file_info_event_data_t, lore_file_stage_file_event_data_t,
    lore_file_unstage_file_event_data_t, lore_log_event_data_t,
    lore_repository_state_dump_node_event_data_t, lore_repository_status_file_event_data_t,
    lore_repository_status_revision_event_data_t, lore_storage_get_data_event_data_t,
    lore_storage_get_item_complete_event_data_t, lore_storage_opened_event_data_t,
    LORE_EVENT_COMPLETE, LORE_EVENT_ERROR, LORE_EVENT_FILE_INFO, LORE_EVENT_FILE_STAGE_FILE,
    LORE_EVENT_FILE_UNSTAGE_FILE, LORE_EVENT_LOG, LORE_EVENT_REPOSITORY_STATE_DUMP_NODE,
    LORE_EVENT_REPOSITORY_STATUS_FILE, LORE_EVENT_REPOSITORY_STATUS_REVISION,
    LORE_EVENT_STORAGE_GET_DATA, LORE_EVENT_STORAGE_GET_ITEM_COMPLETE, LORE_EVENT_STORAGE_OPENED,
};

/// One event decoded from lore's tagged event union. The payloads are shallow
/// copies: strings inside them are only valid while the callback that
/// delivered the event runs.
#[non_exhaustive]
pub enum Event {
    Log(lore_log_event_data_t),
    Error(lore_error_event_data_t),
    Complete(lore_complete_event_data_t),
    FileInfo(lore_file_info_event_data_t),
    FileStageFile(lore_file_stage_file_event_data_t),
    FileUnstageFile(lore_file_unstage_file_event_data_t),
    RepositoryStatusRevision(lore_repository_status_revision_event_data_t),
    RepositoryStatusFile(lore_repository_status_file_event_data_t),
    RepositoryStateDumpNode(lore_repository_state_dump_node_event_data_t),
    StorageOpened(lore_storage_opened_event_data_t),
    StorageGetData(lore_storage_get_data_event_data_t),
    StorageGetItemComplete(lore_storage_get_item_complete_event_data_t),
    /// Event kinds without a mapped variant (yet); add them here as needed
    Other,
}

impl Event {
    pub fn from_raw(event: &lore_event_t) -> Self {
        let Ok(tag) = lore_event_id_t::try_from(event.tag) else {
            return Self::Other;
        };

        // Safety: `tag` selects the union field the producer initialized; the
        // C side is cbindgen output of the same tagged enum
        let data = &event.__bindgen_anon_1;
        unsafe {
            match tag {
                LORE_EVENT_LOG => Self::Log(data.log),
                LORE_EVENT_ERROR => Self::Error(data.error),
                LORE_EVENT_COMPLETE => Self::Complete(data.complete),
                LORE_EVENT_FILE_INFO => Self::FileInfo(data.file_info),
                LORE_EVENT_FILE_STAGE_FILE => Self::FileStageFile(data.file_stage_file),
                LORE_EVENT_FILE_UNSTAGE_FILE => Self::FileUnstageFile(data.file_unstage_file),
                LORE_EVENT_REPOSITORY_STATUS_REVISION => {
                    Self::RepositoryStatusRevision(data.repository_status_revision)
                }
                LORE_EVENT_REPOSITORY_STATUS_FILE => {
                    Self::RepositoryStatusFile(data.repository_status_file)
                }
                LORE_EVENT_REPOSITORY_STATE_DUMP_NODE => {
                    Self::RepositoryStateDumpNode(data.repository_state_dump_node)
                }
                LORE_EVENT_STORAGE_OPENED => Self::StorageOpened(data.storage_opened),
                LORE_EVENT_STORAGE_GET_DATA => Self::StorageGetData(data.storage_get_data),
                LORE_EVENT_STORAGE_GET_ITEM_COMPLETE => {
                    Self::StorageGetItemComplete(data.storage_get_item_complete)
                }
                _ => Self::Other,
            }
        }
    }
}

