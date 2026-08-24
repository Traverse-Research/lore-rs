use crate::{Event, LoreStringArrayExt, LoreStringExt};
use lore_sys::{
    lore_address_t, lore_event_callback_config_t, lore_event_t, lore_event_tag_t,
    lore_file_info_args_t, lore_global_args_t, lore_partition_t, lore_repository_info_args_t,
    lore_revision_tree_close_args_t, lore_revision_tree_load_args_t, lore_storage_close_args_t,
    lore_storage_get_args_t, lore_storage_get_item_array_t, lore_storage_get_item_t,
    lore_storage_open_args_t, lore_storage_remote_config_t, lore_store_t, lore_string_array_t,
    lore_string_t, LORE_EVENT_COMPLETE, LORE_EVENT_ERROR,
};

/// A Lore call that returned a non-zero status, with whatever the operation
/// said about why. Mirrors `LoreError` in Epic's Go, Python and C# SDKs.
#[derive(Debug)]
pub struct LoreError {
    /// The status code the entry point returned.
    pub status: i32,
    /// The messages of every `LORE_EVENT_ERROR` the call emitted, or the
    /// message of the failed `LORE_EVENT_COMPLETE` when it emitted none.
    /// Empty when the call failed before emitting any event, in which case the
    /// status code is all Lore reports.
    pub messages: Vec<String>,
}

impl std::fmt::Display for LoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "lore call failed with status {}", self.status)?;
        if !self.messages.is_empty() {
            write!(f, ": {}", self.messages.join("; "))?;
        }
        Ok(())
    }
}

impl std::error::Error for LoreError {}

/// Options shared by every Lore operation, the equivalent of
/// `lore_global_args_t` with Rust types.
///
/// [`Default`] is the all-zero configuration the other Lore SDKs default to;
/// zero means "use the library default" for the numeric fields. Set only what
/// you need:
///
/// ```
/// # use lore_rs::GlobalArgs;
/// let globals = GlobalArgs {
///     repository_path: "/path/to/repository",
///     offline: true,
///     ..Default::default()
/// };
/// ```
///
/// Strings are borrowed rather than owned, so no field can outlive the data it
/// points at, which is what keeps the per-command functions safe to call.
#[derive(Debug, Default, Clone, Copy)]
pub struct GlobalArgs<'a> {
    /// Repository path.
    pub repository_path: &'a str,
    /// Correlation ID.
    pub correlation_id: &'a str,
    /// Identity to use.
    pub identity: &'a str,
    /// Force the operation if possible.
    pub force: bool,
    /// Run the operation without connecting to the server.
    pub offline: bool,
    /// Use only local data.
    pub local: bool,
    /// Use only remote data.
    pub remote: bool,
    /// Report what would have been changed without touching the file system.
    pub dry_run: bool,
    /// Avoid recording last access timestamps in the data stores.
    pub no_atime: bool,
    /// Maximum number of parallel connections for bulk data transfer.
    pub max_connections: u32,
    /// Search limit when iterating revisions.
    pub search_limit: u32,
    /// Allow matching the nearest revision when no perfect match exists.
    pub search_nearest: bool,
    /// Prevent the automatic incremental GC for this operation.
    pub no_gc: bool,
    /// Use in-memory stores instead of the file-backed ones.
    pub in_memory: bool,
    /// Maximum number of files processed in parallel.
    pub file_count_limit: u64,
    /// Maximum total size of all files processed in parallel.
    pub file_size_limit: u64,
    /// Maximum number of parallel compression tasks.
    pub compress_task_limit: u64,
    /// Keep store references alive after the call completes, so consecutive
    /// calls in one process skip repeated store open/close cycles.
    pub store_keep_alive: bool,
    /// How long to keep store references alive. Only used with
    /// `store_keep_alive`; zero means the library default of 10 seconds.
    pub store_keep_alive_seconds: u64,
    /// Force syncing data to the storage media during store flush.
    pub sync_data: bool,
    /// Cache fragment payloads fetched from a remote in the local store.
    pub cache: bool,
}

impl GlobalArgs<'_> {
    /// The raw struct to hand to Lore. Borrows from `self`.
    fn to_raw(self) -> lore_global_args_t {
        lore_global_args_t {
            repository_path: raw_str(self.repository_path),
            correlation_id: raw_str(self.correlation_id),
            identity: raw_str(self.identity),
            force: u8::from(self.force),
            offline: u8::from(self.offline),
            local: u8::from(self.local),
            remote: u8::from(self.remote),
            dry_run: u8::from(self.dry_run),
            no_atime: u8::from(self.no_atime),
            max_connections: self.max_connections,
            search_limit: self.search_limit,
            search_nearest: u8::from(self.search_nearest),
            no_gc: u8::from(self.no_gc),
            in_memory: u8::from(self.in_memory),
            file_count_limit: self.file_count_limit,
            file_size_limit: self.file_size_limit,
            compress_task_limit: self.compress_task_limit,
            store_keep_alive: u8::from(self.store_keep_alive),
            store_keep_alive_seconds: self.store_keep_alive_seconds,
            sync_data: u8::from(self.sync_data),
            cache: u8::from(self.cache),
        }
    }
}

/// An empty string is passed as a null pointer rather than as a pointer to
/// zero bytes, so that Lore cannot tell "unset" and "set to the empty string"
/// apart. The other Lore SDKs do the same.
pub(crate) fn raw_str(string: &str) -> lore_string_t {
    if string.is_empty() {
        lore_string_t::EMPTY
    } else {
        lore_string_t::from_str(string)
    }
}

/// Why a call failed, collected from the events that conclude it. Follows the
/// other SDKs: an explicit error event always wins over the message the
/// completion carries.
#[derive(Default)]
struct FailureContext {
    errors: Vec<String>,
    complete: Option<String>,
}

impl FailureContext {
    /// Takes the event the callback is about to be handed, so that an event is
    /// decoded exactly once no matter how many of them a call emits.
    fn record(&mut self, tag: lore_event_tag_t, event: &Result<Event<'_>, std::str::Utf8Error>) {
        match event {
            Ok(Event::Error {
                error_type,
                message,
            }) => self.errors.push(format!("{error_type}: {message}")),
            // Lore puts the reason a failed call failed on its completion, but
            // leaves it empty when the operation reported no detail.
            Ok(Event::Complete { status, error }) if *status != 0 && !error.is_empty() => {
                self.complete = Some((*error).to_owned());
            }
            // Only a terminal event that failed to decode says something about
            // the call as a whole. A malformed path in one of a hundred
            // thousand file events is for the callback to deal with.
            Err(error) if tag == LORE_EVENT_ERROR || tag == LORE_EVENT_COMPLETE => self
                .errors
                .push(format!("lore event {tag} carried invalid UTF-8: {error}")),
            _ => {}
        }
    }

    fn into_messages(self) -> Vec<String> {
        if self.errors.is_empty() {
            self.complete.into_iter().collect()
        } else {
            self.errors
        }
    }
}

/// Runs one Lore operation, decoding every event it emits into [`Event`] and
/// handing it to `callback`. The per-command functions below are thin calls
/// onto this; reach for it directly only for a command that has no wrapper yet.
///
/// # Safety
///
/// `lore_function` must be an entry point of the loaded library that takes
/// `Args` as its arguments struct — pairing one command's function with
/// another's arguments is undefined behaviour. It must also be a **synchronous**
/// entry point (`lore_x`, not `lore_x_async`): this hands Lore a pointer to a
/// context on the caller's stack, which is only sound while the call has not
/// returned.
pub unsafe fn call_with_callback<Args, Callback>(
    lore_function: unsafe extern "C" fn(
        *const lore_global_args_t,
        *const Args,
        lore_event_callback_config_t,
    ) -> i32,
    globals: GlobalArgs<'_>,
    args: &Args,
    callback: Callback,
) -> Result<(), LoreError>
where
    Callback: FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
{
    /// Lore's callback config has room for exactly one pointer-sized
    /// `user_context`, so the callback travels together with the errors
    /// collected on its behalf in one struct.
    struct Context<Callback> {
        callback: Callback,
        failure: FailureContext,
    }

    /// Lore expects a plain C function; to avoid writing one per call site we
    /// pass this one and recover the closure from the `user_context` it hands
    /// back. One instance of it exists per `Callback` type, resolved at
    /// compile time.
    extern "C" fn dispatch<Callback>(event: *const lore_event_t, user_context: u64)
    where
        Callback: FnMut(Result<Event<'_>, std::str::Utf8Error>),
    {
        // SAFETY: `user_context` is the pointer to the `Context` created
        // below, cast back to the type it was cast from. The call this
        // callback belongs to has not returned yet, so the `Context` is still
        // borrowed by the stack frame that owns it, and events of one call
        // arrive one at a time, so no other reference to it exists.
        let context = unsafe { &mut *(user_context as *mut Context<Callback>) };
        // SAFETY: Lore hands us a valid event whose tag matches the union
        // variant it initialized, valid for the duration of this call.
        let (tag, event) = unsafe {
            let event = &*event;
            (event.tag, Event::from_raw(event))
        };
        context.failure.record(tag, &event);
        (context.callback)(event);
    }

    let mut context = Context {
        callback,
        failure: FailureContext::default(),
    };

    // SAFETY: `lore_function`'s own type guarantees it takes global arguments
    // and an `Args` by pointer, both of which are live borrows here, and that
    // it is synchronous: it does not return before it is done calling back, so
    // `context` outlives every use of the pointer handed to Lore.
    let status = unsafe {
        lore_function(
            &globals.to_raw(),
            args,
            lore_event_callback_config_t {
                user_context: std::ptr::addr_of_mut!(context) as u64,
                func: Some(dispatch::<Callback> as unsafe extern "C" fn(*const lore_event_t, u64)),
            },
        )
    };

    if status == 0 {
        Ok(())
    } else {
        // A call that fails before emitting any event (bad arguments, dead
        // connection) reports the failure only through its status code.
        Err(LoreError {
            status,
            messages: context.failure.into_messages(),
        })
    }
}

/// Arguments for [`repository_info`].
#[derive(Debug, Default, Clone, Copy)]
pub struct RepositoryInfoArgs<'a> {
    /// URL of the remote repository to query.
    pub repository_url: &'a str,
}

impl RepositoryInfoArgs<'_> {
    /// The raw struct to hand to Lore, borrowing the same text `self` does.
    fn to_raw(self) -> lore_repository_info_args_t {
        lore_repository_info_args_t {
            repository_url: raw_str(self.repository_url),
        }
    }
}

/// Queries a remote repository's metadata.
///
/// This corresponds to `lore_sys::Lore::lore_repository_info`.
pub fn repository_info(
    lore: &crate::Lore,
    globals: GlobalArgs<'_>,
    args: RepositoryInfoArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `args`, which lives across the call.
    unsafe { call_with_callback(lore.lore_repository_info, globals, &args.to_raw(), callback) }
}

/// Arguments for [`file_info`].
#[derive(Debug, Default, Clone, Copy)]
pub struct FileInfoArgs<'a> {
    /// Paths to report on, relative to the repository root.
    pub paths: &'a [&'a str],
    /// Revision to report on; empty is the revision the repository is on.
    pub revision: &'a str,
    /// Also report the hash and size of the file on the local filesystem.
    pub local: bool,
    /// Also report the repository size with filters applied.
    pub filtered: bool,
}

impl FileInfoArgs<'_> {
    /// The paths as Lore's own string type. Kept separate from [`Self::to_raw`]
    /// because the array has to outlive the call, and a `to_raw` that built it
    /// could only hand back a borrow of its own temporary.
    fn raw_paths(self) -> Vec<lore_string_t> {
        self.paths.iter().copied().map(raw_str).collect()
    }

    /// The raw struct to hand to Lore, borrowing `paths` and the same text
    /// `self` does.
    fn to_raw(self, paths: &[lore_string_t]) -> lore_file_info_args_t {
        lore_file_info_args_t {
            paths: if paths.is_empty() {
                lore_string_array_t::EMPTY
            } else {
                lore_string_array_t {
                    ptr: paths.as_ptr(),
                    count: paths.len(),
                }
            },
            revision: raw_str(self.revision),
            local: u8::from(self.local),
            filtered: u8::from(self.filtered),
        }
    }
}

/// Reports what the repository holds at each path — content hash, context and
/// size — without reading any content.
///
/// One [`Event::FileInfo`] arrives per path. A path the revision does not hold
/// is reported through [`Event::Error`], not by the absence of an event.
///
/// This corresponds to `lore_sys::Lore::lore_file_info`.
pub fn file_info(
    lore: &crate::Lore,
    globals: GlobalArgs<'_>,
    args: FileInfoArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    let paths = args.raw_paths();

    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `paths` and `args`, both of which live across the call.
    unsafe { call_with_callback(lore.lore_file_info, globals, &args.to_raw(&paths), callback) }
}

/// Arguments for [`storage_open`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StorageOpenArgs<'a> {
    /// Path to an existing Lore repository. Must be empty when `in_memory` is
    /// set.
    pub repository_path: &'a str,
    /// Open a fresh in-memory store instead of a file-backed one.
    pub in_memory: bool,
    /// Endpoint of the peer storage service, or [`None`] for a handle without a
    /// remote.
    pub remote_url: Option<&'a str>,
    /// Soft cap on total immutable-store bytes; zero selects the default.
    pub cache_target_bytes: u64,
    /// Soft cap on the immutable-store fragment count; zero selects the default.
    pub cache_target_fragments: u64,
}

impl StorageOpenArgs<'_> {
    /// The raw struct to hand to Lore, borrowing the same text `self` does.
    fn to_raw(self) -> lore_storage_open_args_t {
        lore_storage_open_args_t {
            repository_path: raw_str(self.repository_path),
            in_memory: u8::from(self.in_memory),
            remote_config: lore_storage_remote_config_t {
                remote_url: raw_str(self.remote_url.unwrap_or_default()),
            },
            has_remote_config: u8::from(self.remote_url.is_some()),
            cache_target_bytes: self.cache_target_bytes,
            cache_target_fragments: self.cache_target_fragments,
        }
    }
}

/// Opens a store.
///
/// This corresponds to `lore_sys::Lore::lore_storage_open`.
pub fn storage_open(
    lore: &crate::Lore,
    globals: GlobalArgs<'_>,
    args: StorageOpenArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `args`, which lives across the call.
    unsafe { call_with_callback(lore.lore_storage_open, globals, &args.to_raw(), callback) }
}

/// One buffer for [`storage_get`] to read.
#[derive(Debug, Clone, Copy)]
pub struct StorageGetItem {
    /// Caller-chosen id, echoed back in every event for this item. What tells
    /// the events of one item from another's when a call reads several.
    pub id: u64,
    /// Partition to read from, which is a repository id. The zero partition is
    /// rejected.
    pub partition: lore_partition_t,
    /// Content address to read, as [`Event::FileInfo`] reports it.
    pub address: lore_address_t,
    /// Deliver one [`Event::StorageGetData`] per leaf fragment instead of a
    /// single reassembled buffer, so peak memory follows the fragment size
    /// rather than the content size.
    pub streaming: bool,
    /// Keep bytes fetched from a remote in the local store even when the
    /// producer did not flag them for local caching.
    pub local_cache: bool,
}

impl StorageGetItem {
    /// The raw struct to hand to Lore. Carries no pointers, so it borrows
    /// nothing.
    fn to_raw(self) -> lore_storage_get_item_t {
        lore_storage_get_item_t {
            id: self.id,
            partition: self.partition,
            address: self.address,
            streaming: u8::from(self.streaming),
            local_cache: u8::from(self.local_cache),
        }
    }
}

/// Arguments for [`storage_get`].
#[derive(Debug, Clone, Copy)]
pub struct StorageGetArgs<'a> {
    /// Handle from [`storage_open`].
    pub handle: lore_store_t,
    /// Buffers to read. Each runs independently and emits its own events.
    pub items: &'a [StorageGetItem],
}

impl StorageGetArgs<'_> {
    /// The items as Lore's own type, kept out of [`Self::to_raw`] for the same
    /// reason as [`FileInfoArgs::raw_paths`].
    fn raw_items(self) -> Vec<lore_storage_get_item_t> {
        self.items
            .iter()
            .copied()
            .map(StorageGetItem::to_raw)
            .collect()
    }

    /// The raw struct to hand to Lore, borrowing `items`.
    fn to_raw(self, items: &[lore_storage_get_item_t]) -> lore_storage_get_args_t {
        lore_storage_get_args_t {
            handle: self.handle,
            items: lore_storage_get_item_array_t {
                ptr: if items.is_empty() {
                    std::ptr::null()
                } else {
                    items.as_ptr()
                },
                count: items.len(),
            },
        }
    }
}

/// Reads content-addressed buffers.
///
/// Each item emits [`Event::StorageGetHeader`] with the size of the
/// reassembled content, then that content as one or more
/// [`Event::StorageGetData`], then [`Event::StorageGetItemComplete`] carrying
/// its outcome. The payload bytes are valid only for the duration of the
/// callback that carries them.
///
/// This corresponds to `lore_sys::Lore::lore_storage_get`.
pub fn storage_get(
    lore: &crate::Lore,
    globals: GlobalArgs<'_>,
    args: StorageGetArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    let items = args.raw_items();

    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `items`, which lives across the call.
    unsafe {
        call_with_callback(
            lore.lore_storage_get,
            globals,
            &args.to_raw(&items),
            callback,
        )
    }
}

/// Releases a store handle.
///
/// This corresponds to `lore_sys::Lore::lore_storage_close`.
pub fn storage_close(
    lore: &crate::Lore,
    globals: GlobalArgs<'_>,
    args: lore_storage_close_args_t,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the arguments
    // hold no pointers.
    unsafe { call_with_callback(lore.lore_storage_close, globals, &args, callback) }
}

/// Loads the directory tree of a revision.
///
/// This corresponds to `lore_sys::Lore::lore_revision_tree_load`.
pub fn revision_tree_load(
    lore: &crate::Lore,
    globals: GlobalArgs<'_>,
    args: lore_revision_tree_load_args_t,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the arguments
    // hold no pointers.
    unsafe { call_with_callback(lore.lore_revision_tree_load, globals, &args, callback) }
}

/// Releases a revision-tree handle.
///
/// This corresponds to `lore_sys::Lore::lore_revision_tree_close`.
pub fn revision_tree_close(
    lore: &crate::Lore,
    globals: GlobalArgs<'_>,
    args: lore_revision_tree_close_args_t,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the arguments
    // hold no pointers.
    unsafe { call_with_callback(lore.lore_revision_tree_close, globals, &args, callback) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lore_sys::LORE_EVENT_LOG;

    /// Plays back the events its `args` carry, then returns the scripted
    /// status; each test decides exactly what Lore appears to do.
    fn run(
        events: &[lore_event_t],
        status: i32,
        callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
    ) -> Result<(), LoreError> {
        struct Script<'a> {
            events: &'a [lore_event_t],
            status: i32,
        }

        unsafe extern "C" fn scripted(
            _globals: *const lore_global_args_t,
            args: *const Script<'_>,
            callback: lore_event_callback_config_t,
        ) -> i32 {
            let script = unsafe { &*args };
            let dispatch = callback.func.unwrap();
            for event in script.events {
                unsafe { dispatch(event, callback.user_context) };
            }
            script.status
        }

        // SAFETY: `scripted` behaves like a synchronous Lore entry point: it
        // only calls back before returning, with valid events, and the
        // pointers inside `Script` borrow from `events`, which outlives the
        // call.
        unsafe {
            call_with_callback(
                scripted,
                GlobalArgs::default(),
                &Script { events, status },
                callback,
            )
        }
    }

    fn log_event(message: &'static str) -> lore_event_t {
        let mut event: lore_event_t = unsafe { std::mem::zeroed() };
        event.tag = LORE_EVENT_LOG as lore_event_tag_t;
        event.__bindgen_anon_1.log.message = lore_string_t::from_str(message);
        event
    }

    fn error_event(error_type: u32, message: &'static str) -> lore_event_t {
        let mut event: lore_event_t = unsafe { std::mem::zeroed() };
        event.tag = LORE_EVENT_ERROR as lore_event_tag_t;
        event.__bindgen_anon_1.error.error_type = error_type;
        event.__bindgen_anon_1.error.error_inner = lore_string_t::from_str(message);
        event
    }

    fn complete_event(status: i32, error: &'static str) -> lore_event_t {
        let mut event: lore_event_t = unsafe { std::mem::zeroed() };
        event.tag = LORE_EVENT_COMPLETE as lore_event_tag_t;
        event.__bindgen_anon_1.complete.status = status;
        event.__bindgen_anon_1.complete.error.message = lore_string_t::from_str(error);
        event
    }

    fn invalid_utf8() -> lore_string_t {
        static INVALID: &[u8] = &[0xff, 0xfe];
        lore_string_t {
            string: INVALID.as_ptr().cast(),
            length: INVALID.len(),
        }
    }

    #[test]
    fn global_args_conversion() {
        let raw = GlobalArgs {
            repository_path: "/repo",
            offline: true,
            ..Default::default()
        }
        .to_raw();
        assert_eq!(unsafe { raw.repository_path.try_to_str() }, Ok("/repo"));
        assert!(raw.correlation_id.string.is_null(), "unset is null");
        assert_eq!(raw.offline, 1);
        assert_eq!(raw.force, 0);
    }

    #[test]
    fn file_info_args_conversion() {
        let args = FileInfoArgs {
            paths: &["models/a.gltf", "models/b.bin"],
            local: true,
            ..Default::default()
        };
        let paths = args.raw_paths();
        let raw = args.to_raw(&paths);

        assert_eq!(raw.paths.count, 2);
        assert_eq!(
            raw.paths.ptr,
            paths.as_ptr(),
            "borrows the array it was given"
        );
        assert_eq!(unsafe { paths[1].try_to_str() }, Ok("models/b.bin"));
        assert!(raw.revision.string.is_null(), "unset is null");
        assert_eq!(raw.local, 1);
        assert_eq!(raw.filtered, 0);
    }

    #[test]
    fn file_info_args_without_paths_pass_an_empty_array() {
        let args = FileInfoArgs::default();
        let paths = args.raw_paths();
        let raw = args.to_raw(&paths);

        // An empty `Vec` has a dangling pointer, which is not what "no paths"
        // should reach Lore as.
        assert!(raw.paths.ptr.is_null());
        assert_eq!(raw.paths.count, 0);
    }

    #[test]
    fn storage_get_args_conversion() {
        let item = StorageGetItem {
            id: 7,
            partition: lore_partition_t { data: [1; 16] },
            address: lore_address_t {
                hash: lore_sys::lore_hash_t { data: [2; 32] },
                context: lore_sys::lore_context_t { data: [3; 16] },
            },
            streaming: false,
            local_cache: true,
        };
        let args = StorageGetArgs {
            handle: lore_store_t { handle_id: 42 },
            items: &[item],
        };
        let items = args.raw_items();
        let raw = args.to_raw(&items);

        assert_eq!(raw.handle.handle_id, 42);
        assert_eq!(raw.items.count, 1);
        assert_eq!(raw.items.ptr, items.as_ptr());
        assert_eq!(items[0].id, 7);
        assert_eq!(items[0].address.hash.data, [2; 32]);
        assert_eq!(items[0].streaming, 0);
        assert_eq!(items[0].local_cache, 1);
    }

    #[test]
    fn storage_get_args_without_items_pass_an_empty_array() {
        let args = StorageGetArgs {
            handle: lore_store_t { handle_id: 1 },
            items: &[],
        };
        let items = args.raw_items();
        let raw = args.to_raw(&items);

        assert!(raw.items.ptr.is_null());
        assert_eq!(raw.items.count, 0);
    }

    #[test]
    fn the_callback_sees_every_event_in_order_and_can_borrow_its_captures() {
        let events = [log_event("hello"), error_event(7, "it broke")];
        let mut seen = Vec::new();
        run(&events, 0, |event| match event {
            Ok(Event::Log { message, .. }) => seen.push(message.to_owned()),
            Ok(Event::Error { message, .. }) => seen.push(message.to_owned()),
            _ => {}
        })
        .unwrap();
        assert_eq!(seen, ["hello", "it broke"]);
    }

    #[test]
    fn error_events_win_over_the_completion_message_in_a_failure() {
        let events = [error_event(7, "it broke"), complete_event(5, "fallback")];
        let error = run(&events, 5, |_| {}).unwrap_err();
        assert_eq!(error.status, 5);
        assert_eq!(error.messages, ["7: it broke"]);
        assert_eq!(
            error.to_string(),
            "lore call failed with status 5: 7: it broke"
        );
    }

    #[test]
    fn the_completion_message_describes_a_failure_without_error_events() {
        let error = run(&[complete_event(3, "no such branch")], 3, |_| {}).unwrap_err();
        assert_eq!(error.messages, ["no such branch"]);
    }

    #[test]
    fn a_failure_before_any_event_reports_only_its_status() {
        let error = run(&[], 2, |_| {}).unwrap_err();
        assert_eq!(error.status, 2);
        assert!(error.messages.is_empty());
        assert_eq!(error.to_string(), "lore call failed with status 2");
    }

    #[test]
    fn a_successful_call_is_ok_even_though_it_emitted_an_error_event() {
        let events = [error_event(7, "non-fatal"), complete_event(0, "")];
        assert!(run(&events, 0, |_| {}).is_ok());
    }

    #[test]
    fn invalid_utf8_in_an_error_event_is_recorded() {
        let mut event = error_event(7, "");
        event.__bindgen_anon_1.error.error_inner = invalid_utf8();
        let error = run(&[event], 1, |_| {}).unwrap_err();
        assert_eq!(error.messages.len(), 1);
        assert!(error.messages[0].contains("invalid UTF-8"), "{error}");
    }

    #[test]
    fn invalid_utf8_in_a_log_event_reaches_the_callback_but_not_the_error() {
        let mut event = log_event("");
        event.__bindgen_anon_1.log.message = invalid_utf8();
        let mut callback_saw_it = false;
        let error = run(&[event], 1, |event| callback_saw_it |= event.is_err()).unwrap_err();
        assert!(callback_saw_it);
        assert!(error.messages.is_empty());
    }
}
