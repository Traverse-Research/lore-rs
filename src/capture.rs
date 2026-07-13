//! Collects the events delivered through the [`lore_event_callback_config_t`]
//! of a single synchronous lore call.

use crate::{
    lore_error_code_t, lore_event_callback_config_t, lore_event_id_t, lore_event_t, lore_string_t,
    LORE_EVENT_COMPLETE, LORE_EVENT_ERROR, LORE_EVENT_FILE_INFO, LORE_EVENT_LOG,
    LORE_EVENT_STORAGE_GET_DATA, LORE_EVENT_STORAGE_GET_ITEM_COMPLETE, LORE_EVENT_STORAGE_OPENED,
};

/// Data copied out of the event stream of a single synchronous call.
#[derive(Default)]
pub struct CapturedEvents {
    pub hash: Option<[u8; 32]>,
    pub context: Option<[u8; 16]>,
    pub handle_id: Option<u64>,
    pub data: Option<Vec<u8>>,
    pub item_error: Option<lore_error_code_t>,
    pub error: Option<String>,
}

/// Runs `call` with a callback configuration that collects the call's events
/// into the returned [`CapturedEvents`]. Events for a single call arrive one
/// at a time, so the callback has exclusive access without locking.
pub fn capture(call: impl FnOnce(lore_event_callback_config_t) -> i32) -> (i32, CapturedEvents) {
    let mut captured = CapturedEvents::default();
    let status = call(lore_event_callback_config_t {
        user_context: (&mut captured as *mut CapturedEvents) as u64,
        func: Some(capture_event),
    });
    (status, captured)
}

/// Copies the text out; only valid while the callback that delivered it runs.
unsafe fn string_to_owned(s: lore_string_t) -> String {
    const MAX_LENGTH: usize = 1 << 20;
    if s.string.is_null() {
        String::new()
    } else if s.length > MAX_LENGTH {
        format!(
            "<corrupt lore string: ptr {:p}, length {:#x}>",
            s.string, s.length
        )
    } else {
        String::from_utf8_lossy(unsafe {
            std::slice::from_raw_parts(s.string.cast::<u8>(), s.length)
        })
        .into_owned()
    }
}

/// Runs on a lore-managed thread; everything the event points to must be
/// copied before returning.
unsafe extern "C" fn capture_event(event: *const lore_event_t, user_context: u64) {
    let captured = unsafe { &mut *(user_context as *mut CapturedEvents) };
    let data = unsafe { &(*event).__bindgen_anon_1 };
    match unsafe { (*event).tag } as lore_event_id_t {
        LORE_EVENT_LOG => {
            let log = unsafe { data.log };
            log::debug!(target: "lore", "{}", unsafe { string_to_owned(log.message) });
        }
        LORE_EVENT_ERROR => {
            let error = unsafe { data.error };
            let message = format!("error {}: {}", error.error_type, unsafe {
                string_to_owned(error.error_inner)
            });
            log::warn!(target: "lore", "{message}");
            captured.error = Some(message);
        }
        LORE_EVENT_FILE_INFO => {
            let file_info = unsafe { data.file_info };
            captured.hash = Some(file_info.hash.data);
            captured.context = Some(file_info.context.data);
        }
        LORE_EVENT_STORAGE_OPENED => {
            captured.handle_id = Some(unsafe { data.storage_opened }.handle_id);
        }
        LORE_EVENT_STORAGE_GET_DATA => {
            const MAX_CONTENT_SIZE: usize = 1 << 40;
            let get_data = unsafe { data.storage_get_data };
            let offset = get_data.offset as usize;
            if offset + get_data.bytes.len > MAX_CONTENT_SIZE {
                captured.error = Some(format!(
                    "corrupt GET_DATA event: offset {:#x} + len {:#x}",
                    offset, get_data.bytes.len
                ));
                return;
            }
            let data = captured.data.get_or_insert_with(Vec::new);
            if data.len() < offset + get_data.bytes.len {
                data.resize(offset + get_data.bytes.len, 0);
            }
            data[offset..offset + get_data.bytes.len].copy_from_slice(unsafe {
                std::slice::from_raw_parts(get_data.bytes.ptr.cast::<u8>(), get_data.bytes.len)
            });
        }
        LORE_EVENT_STORAGE_GET_ITEM_COMPLETE => {
            let item_complete = unsafe { data.storage_get_item_complete };
            if item_complete.error_code != 0 {
                captured.item_error = Some(item_complete.error_code);
            }
        }
        LORE_EVENT_COMPLETE => {
            let complete = unsafe { data.complete };
            if complete.status != 0 && captured.error.is_none() {
                captured.error = Some(format!("status {}", complete.status));
            }
        }
        _ => {}
    }
}
