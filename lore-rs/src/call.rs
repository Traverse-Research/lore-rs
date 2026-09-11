//! One function per Lore command, each still shaped like the C API: arguments
//! in, events out through a callback. This is the escape hatch for the
//! commands the handle types in this crate do not cover, which is most of
//! them.

use crate::string::{raw_str, raw_str_array};
use crate::{Event, GlobalArgs, LoreError};
use lore_sys::{
    lore_address_t, lore_auth_login_with_token_args_t, lore_branch_info_args_t, lore_bytes_t,
    lore_context_t, lore_event_callback_config_t, lore_event_t, lore_event_tag_t,
    lore_file_info_args_t, lore_global_args_t, lore_partition_t, lore_repository_info_args_t,
    lore_repository_status_args_t, lore_revision_info_args_t, lore_revision_tree_close_args_t,
    lore_revision_tree_info_args_t, lore_revision_tree_list_children_args_t,
    lore_revision_tree_load_args_t, lore_revision_tree_node_info_args_t,
    lore_revision_tree_node_path_args_t, lore_revision_tree_resolve_path_args_t,
    lore_revision_tree_t, lore_storage_close_args_t, lore_storage_flush_args_t,
    lore_storage_get_args_t, lore_storage_get_item_array_t, lore_storage_get_item_t,
    lore_storage_get_metadata_args_t, lore_storage_get_metadata_item_array_t,
    lore_storage_get_metadata_item_t, lore_storage_open_args_t, lore_storage_put_args_t,
    lore_storage_put_item_array_t, lore_storage_put_item_t, lore_storage_remote_config_t,
    lore_store_t, lore_string_t, LORE_EVENT_COMPLETE, LORE_EVENT_ERROR,
};

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
            Ok(Event::Complete {
                status, message, ..
            }) if *status != 0 && !message.is_empty() => {
                self.complete = Some((*message).to_owned());
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
/// # Threading
///
/// Lore runs the callback on one of its own worker threads while this call
/// blocks; the synchronous entry points do not return before the final `End`
/// event has been delivered. That is why `callback` must be [`Send`]. Events of
/// one call arrive one at a time, so the closure is never run concurrently
/// with itself.
///
/// The callback must not call back into Lore on the handle the call is about:
/// Lore's own contract says re-entering the storage API from a callback can
/// deadlock against its in-flight accounting. Record what you need and act
/// after the call returns. Dropping a [`Store`](crate::Store) or
/// [`RevisionTree`](crate::RevisionTree) inside a callback is such a call.
///
/// A panic inside `callback` is caught on Lore's thread, the callback is not
/// invoked for the rest of the call, and the panic resumes on the calling
/// thread once the call has returned. Letting it unwind through the C frame
/// would abort the process.
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
    command: &'static str,
    globals: &GlobalArgs,
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
        /// The first panic the callback raised, to resume on the caller's
        /// thread. Once set, the callback is not invoked again.
        panic: Option<Box<dyn std::any::Any + Send>>,
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

        if context.panic.is_some() {
            return;
        }
        // Nothing here observes a half-updated closure after a panic: the
        // closure is never called again, and the panic is resumed before the
        // caller can look at anything the closure wrote.
        let callback = std::panic::AssertUnwindSafe(|| (context.callback)(event));
        if let Err(payload) = std::panic::catch_unwind(callback) {
            context.panic = Some(payload);
        }
    }

    let mut context = Context {
        callback,
        failure: FailureContext::default(),
        panic: None,
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

    if let Some(payload) = context.panic.take() {
        std::panic::resume_unwind(payload);
    }

    if status == 0 {
        Ok(())
    } else {
        // A call that fails before emitting any event (bad arguments, dead
        // connection) reports the failure only through its status code.
        Err(LoreError::Call {
            command,
            status,
            messages: context.failure.into_messages(),
        })
    }
}

/// Arguments for [`auth_login_with_token`].
// `Debug` is written out below rather than derived: `token` is a credential,
// and a derive would put it in any log line that formats these arguments.
#[derive(Default, Clone, Copy)]
pub struct AuthLoginWithTokenArgs<'a> {
    /// Server to log in against, as `lore://host:port`. Empty resolves it from
    /// the `.lore/config.toml` under `globals.repository_path`.
    pub remote_url: &'a str,
    pub token: &'a str,
    /// `"lore"` for a token that already is a Lore JWT, which Lore validates
    /// and stores as it stands. Anything else — `"eg1"`, `"api-key"` — is
    /// handed to the auth service's exchange, which answers with one.
    pub token_type: &'a str,
    /// Auth service with its scheme, as `ucs-auth://auth.example.com`. Set,
    /// this skips asking the server which auth service to use, and the token
    /// is validated against this domain instead. Required when there is no
    /// server to ask.
    pub auth_url: &'a str,
}

impl std::fmt::Debug for AuthLoginWithTokenArgs<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthLoginWithTokenArgs")
            .field("remote_url", &self.remote_url)
            .field("token", &"[token]")
            .field("token_type", &self.token_type)
            .field("auth_url", &self.auth_url)
            .finish()
    }
}

impl AuthLoginWithTokenArgs<'_> {
    /// The raw struct to hand to Lore, borrowing the same text `self` does.
    fn to_raw(self) -> lore_auth_login_with_token_args_t {
        lore_auth_login_with_token_args_t {
            remote_url: raw_str(self.remote_url),
            token: raw_str(self.token),
            token_type: raw_str(self.token_type),
            auth_url: raw_str(self.auth_url),
        }
    }
}

/// Logs in with a token obtained elsewhere, storing the Lore token it is
/// exchanged for on [`Event::AuthUserInfo`], which a success always emits.
///
/// The token store this writes is per OS user, not per process or per
/// repository: it is the same one Lore's own CLI reads and writes, and a login
/// here outlives the process that made it.
///
/// With `args.auth_url` empty Lore asks the server for its environment to find
/// the auth service, so the login needs that server reachable even though the
/// exchange itself happens elsewhere.
///
/// This corresponds to `lore_sys::Lore::lore_auth_login_with_token`.
pub fn auth_login_with_token(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: AuthLoginWithTokenArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `args`, which lives across the call.
    unsafe {
        call_with_callback(
            lore.lore_auth_login_with_token,
            command,
            globals,
            &args.to_raw(),
            callback,
        )
    }
}

/// Arguments for [`repository_info`].
#[derive(Debug, Default, Clone, Copy)]
pub struct RepositoryInfoArgs<'a> {
    /// URL of the remote repository to query, as `lore://host:port/name`.
    /// Empty asks about the repository `globals.repository_path` holds.
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

/// Queries a repository's metadata.
///
/// With a URL this needs no local repository at all; Lore runs it against
/// in-memory stores. With an empty URL Lore assembles one from the
/// `.lore/config.toml` and `.lore/id` under `globals.repository_path`, and
/// with `globals.local` set it answers from that repository's own stores
/// instead of asking the server.
///
/// This corresponds to `lore_sys::Lore::lore_repository_info`.
pub fn repository_info(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: RepositoryInfoArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `args`, which lives across the call.
    unsafe {
        call_with_callback(
            lore.lore_repository_info,
            command,
            globals,
            &args.to_raw(),
            callback,
        )
    }
}

/// Arguments for [`branch_info`].
#[derive(Debug, Default, Clone, Copy)]
pub struct BranchInfoArgs<'a> {
    /// Branch to report on; empty is the branch the repository is on.
    pub branch: &'a str,
}

impl BranchInfoArgs<'_> {
    /// The raw struct to hand to Lore, borrowing the same text `self` does.
    fn to_raw(self) -> lore_branch_info_args_t {
        lore_branch_info_args_t {
            branch: raw_str(self.branch),
        }
    }
}

/// Reports one branch: what it is, and which revision it points at.
///
/// [`Event::BranchInfo`] carries the tip twice. `latest` is what the local
/// mutable store holds, `latest_remote` what the remote says; either is the
/// zero hash when that side has nothing — no remote configured, an offline
/// call, a branch created but never pushed, or a branch this repository has
/// never had locally. Resolving the tip to read means preferring
/// `latest_remote` and falling back to `latest`.
///
/// A branch the local store does not know is looked up on the remote by name,
/// so a name that has never been materialized here still resolves.
///
/// A repository verb: `globals.repository_path` must hold a `.lore`
/// directory, and the remote it reports on is that repository's own, from its
/// `.lore/config.toml`.
///
/// This corresponds to `lore_sys::Lore::lore_branch_info`.
pub fn branch_info(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: BranchInfoArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `args`, which lives across the call.
    unsafe {
        call_with_callback(
            lore.lore_branch_info,
            command,
            globals,
            &args.to_raw(),
            callback,
        )
    }
}

/// Arguments for [`revision_info`].
#[derive(Debug, Default, Clone, Copy)]
pub struct RevisionInfoArgs<'a> {
    /// The revision to report on, in any form Lore resolves: a full hash,
    /// `branch@LATEST`, `branch@<number>`, or `@LATEST` for the branch the
    /// instance is on. Empty is the revision the instance is on.
    pub revision: &'a str,
    /// Also report the changes against the parent revision.
    pub delta: bool,
    /// Also report the revision's metadata entries.
    pub metadata: bool,
}

impl RevisionInfoArgs<'_> {
    /// The raw struct to hand to Lore, borrowing the same text `self` does.
    fn to_raw(self) -> lore_revision_info_args_t {
        lore_revision_info_args_t {
            revision: raw_str(self.revision),
            delta: u8::from(self.delta),
            metadata: u8::from(self.metadata),
        }
    }
}

/// Resolves a revision signature and reports the revision: its hash, number
/// and parents on [`Event::RevisionInfo`].
///
/// `branch@LATEST` is resolved the way Lore itself does it: the local tip
/// unless the remote's is strictly ahead of it, decided by walking the
/// history between the two. A branch with nothing on it resolves to the zero
/// hash. A repository verb: `globals.repository_path` must hold a `.lore`
/// directory.
///
/// This corresponds to `lore_sys::Lore::lore_revision_info`.
pub fn revision_info(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: RevisionInfoArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `args`, which lives across the call.
    unsafe {
        call_with_callback(
            lore.lore_revision_info,
            command,
            globals,
            &args.to_raw(),
            callback,
        )
    }
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
            paths: raw_str_array(paths),
            revision: raw_str(self.revision),
            local: u8::from(self.local),
            filtered: u8::from(self.filtered),
        }
    }
}

/// Reports what the repository holds at each path — content hash, context and
/// size.
///
/// One [`Event::FileInfo`] arrives per path. A path the revision does not hold
/// is reported through [`Event::Error`], not by the absence of an event.
///
/// Not a metadata-only call, whatever `local` is set to. Lore deserializes the
/// revision state on every call, and reports whether a path is modified by
/// stat-ing the working-tree file and hashing all of it when its size matches
/// the revision's. Reach for [`revision_tree_resolve_path`] and
/// [`revision_tree_node_info`] against a loaded tree to look up an address
/// without any of that.
///
/// This corresponds to `lore_sys::Lore::lore_file_info`.
pub fn file_info(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: FileInfoArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    let paths = args.raw_paths();

    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `paths` and `args`, both of which live across the call.
    unsafe {
        call_with_callback(
            lore.lore_file_info,
            command,
            globals,
            &args.to_raw(&paths),
            callback,
        )
    }
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

/// Opens a store. See [`Store::open`](crate::Store::open) for what Lore binds
/// into the handle at this point.
///
/// This corresponds to `lore_sys::Lore::lore_storage_open`.
pub fn storage_open(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: StorageOpenArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `args`, which lives across the call.
    unsafe {
        call_with_callback(
            lore.lore_storage_open,
            command,
            globals,
            &args.to_raw(),
            callback,
        )
    }
}

/// One buffer for [`storage_put`] to store.
///
/// Lore borrows `data` rather than copying it, and requires the bytes to stay
/// valid until the item's completion fires. The lifetime is what holds that:
/// [`storage_put`] does not return before its last event.
#[derive(Debug, Clone, Copy)]
pub struct StoragePutItem<'a> {
    /// Caller-chosen id, echoed back on the item's
    /// [`Event::StoragePutItemComplete`]. What tells the completions of one
    /// item from another's when a call writes several.
    pub id: u64,
    /// Partition to write to, which is a repository id. The zero partition is
    /// rejected, as this item's own outcome rather than the call's.
    pub partition: lore_partition_t,
    /// Which file the bytes belong to, stored next to the content hash in the
    /// resulting address.
    pub context: lore_context_t,
    /// The bytes to hash and store. Empty stores nothing and completes with
    /// the zero hash.
    pub data: &'a [u8],
    /// Also write the content to the server; ignored without a remote.
    pub remote_write: bool,
    /// Tag the fragments so every later remote read of them is cached
    /// locally, whatever the reader asked for.
    pub local_cache: bool,
    /// Cap on the leaf fragment size a large buffer is split into; zero lets
    /// Lore choose.
    pub fixed_size_chunk: u64,
}

impl StoragePutItem<'_> {
    /// The raw struct to hand to Lore, borrowing the same bytes `self` does.
    fn to_raw(self) -> lore_storage_put_item_t {
        lore_storage_put_item_t {
            id: self.id,
            partition: self.partition,
            context: self.context,
            data: lore_bytes_t {
                // An empty slice's pointer is dangling, which is not what "no
                // bytes" should reach Lore as.
                ptr: if self.data.is_empty() {
                    std::ptr::null()
                } else {
                    self.data.as_ptr().cast()
                },
                len: self.data.len(),
            },
            remote_write: u8::from(self.remote_write),
            local_cache: u8::from(self.local_cache),
            fixed_size_chunk: self.fixed_size_chunk,
        }
    }
}

/// Arguments for [`storage_put`].
#[derive(Debug, Clone, Copy)]
pub struct StoragePutArgs<'a> {
    /// Handle from [`storage_open`].
    pub handle: lore_store_t,
    /// Buffers to store. Each runs independently and completes on its own,
    /// carrying the item's `id`.
    pub items: &'a [StoragePutItem<'a>],
}

impl StoragePutArgs<'_> {
    /// The items as Lore's own type, kept out of [`Self::to_raw`] for the same
    /// reason as [`FileInfoArgs::raw_paths`].
    fn raw_items(self) -> Vec<lore_storage_put_item_t> {
        self.items
            .iter()
            .copied()
            .map(StoragePutItem::to_raw)
            .collect()
    }

    /// The raw struct to hand to Lore, borrowing `items`.
    fn to_raw(self, items: &[lore_storage_put_item_t]) -> lore_storage_put_args_t {
        lore_storage_put_args_t {
            handle: self.handle,
            items: lore_storage_put_item_array_t {
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

/// Stores content-addressed buffers.
///
/// Each item is hashed and written independently and emits one
/// [`Event::StoragePutItemComplete`] carrying the address its bytes landed at
/// and its own outcome, which is an error code and nothing more.
///
/// Any failed item makes the whole call return non-zero, with a message that
/// counts the failures rather than naming one.
///
/// This corresponds to `lore_sys::Lore::lore_storage_put`.
pub fn storage_put(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: StoragePutArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    let items = args.raw_items();

    // SAFETY: the entry point is the loaded library's own; the raw struct
    // borrows from `items`, and those borrow the caller's buffers. Both live
    // across the call.
    unsafe {
        call_with_callback(
            lore.lore_storage_put,
            command,
            globals,
            &args.to_raw(&items),
            callback,
        )
    }
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
    /// Buffers to read. Each runs independently and emits its own events,
    /// all carrying the item's `id`.
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
/// Any failed item makes the whole call return non-zero; the item's own
/// outcome is on its [`Event::StorageGetItemComplete`].
///
/// This corresponds to `lore_sys::Lore::lore_storage_get`.
pub fn storage_get(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: StorageGetArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    let items = args.raw_items();

    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `items`, which lives across the call.
    unsafe {
        call_with_callback(
            lore.lore_storage_get,
            command,
            globals,
            &args.to_raw(&items),
            callback,
        )
    }
}

/// One address for [`storage_get_metadata`] to look up.
#[derive(Debug, Clone, Copy)]
pub struct StorageGetMetadataItem {
    /// Caller-chosen id, echoed back on the item's
    /// [`Event::StorageGetMetadataItemComplete`].
    pub id: u64,
    /// Partition to look up in, which is a repository id. The zero partition
    /// is rejected, as this item's own outcome rather than the call's.
    pub partition: lore_partition_t,
    /// Content address to look up.
    pub address: lore_address_t,
}

impl StorageGetMetadataItem {
    /// The raw struct to hand to Lore. Carries no pointers, so it borrows
    /// nothing.
    fn to_raw(self) -> lore_storage_get_metadata_item_t {
        lore_storage_get_metadata_item_t {
            id: self.id,
            partition: self.partition,
            address: self.address,
        }
    }
}

/// Arguments for [`storage_get_metadata`].
#[derive(Debug, Clone, Copy)]
pub struct StorageGetMetadataArgs<'a> {
    /// Handle from [`storage_open`].
    pub handle: lore_store_t,
    /// Addresses to look up. Each runs independently and completes on its
    /// own, carrying the item's `id`.
    pub items: &'a [StorageGetMetadataItem],
}

impl StorageGetMetadataArgs<'_> {
    /// The items as Lore's own type, kept out of [`Self::to_raw`] for the same
    /// reason as [`FileInfoArgs::raw_paths`].
    fn raw_items(self) -> Vec<lore_storage_get_metadata_item_t> {
        self.items
            .iter()
            .copied()
            .map(StorageGetMetadataItem::to_raw)
            .collect()
    }

    /// The raw struct to hand to Lore, borrowing `items`.
    fn to_raw(
        self,
        items: &[lore_storage_get_metadata_item_t],
    ) -> lore_storage_get_metadata_args_t {
        lore_storage_get_metadata_args_t {
            handle: self.handle,
            items: lore_storage_get_metadata_item_array_t {
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

/// Reports what a store holds for content addresses, without their bytes.
///
/// Each item emits one [`Event::StorageGetMetadataItemComplete`] carrying the
/// fragment and its own outcome, and nothing else: no header, no data. Lore
/// probes the local store and falls through to the remote, and caches nothing
/// of what it finds there, since there are no bytes to cache.
///
/// An address neither side holds completes with
/// [`ErrorCode::AddressNotFound`](crate::ErrorCode::AddressNotFound), which
/// like any other failed item makes the whole call return non-zero.
///
/// This corresponds to `lore_sys::Lore::lore_storage_get_metadata`.
pub fn storage_get_metadata(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: StorageGetMetadataArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    let items = args.raw_items();

    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `items`, which lives across the call.
    unsafe {
        call_with_callback(
            lore.lore_storage_get_metadata,
            command,
            globals,
            &args.to_raw(&items),
            callback,
        )
    }
}

/// Releases a store handle. Lore does not wait for the flush this spawns.
///
/// This corresponds to `lore_sys::Lore::lore_storage_close`.
pub fn storage_close(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: lore_storage_close_args_t,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the arguments
    // hold no pointers.
    unsafe { call_with_callback(lore.lore_storage_close, command, globals, &args, callback) }
}

/// Flushes a store's pending writes to its backing media and waits for them,
/// honoring `globals.sync_data`. Emits no events of its own; the completion is
/// the whole answer.
///
/// This is the immutable and the mutable store behind the handle, both of
/// them, so it covers everything pending there rather than only what this
/// handle wrote. An in-memory store has nothing to flush and succeeds.
///
/// This corresponds to `lore_sys::Lore::lore_storage_flush`.
pub fn storage_flush(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: lore_storage_flush_args_t,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the arguments
    // hold no pointers.
    unsafe { call_with_callback(lore.lore_storage_flush, command, globals, &args, callback) }
}

/// Arguments for [`repository_status`].
#[derive(Debug, Default, Clone, Copy)]
pub struct RepositoryStatusArgs<'a> {
    /// Paths to report on; empty reports on the whole repository.
    pub paths: &'a [&'a str],
    /// Include the staged state in the report.
    pub staged: bool,
    /// Walk the filesystem under each path and refresh every dirty flag.
    pub scan: bool,
    /// Re-examine the files already marked dirty, without a full scan.
    pub check_dirty: bool,
    /// Clear the tracked dirty state.
    pub reset: bool,
    /// Report the last revision merged in from the parent branch.
    pub sync_point: bool,
    /// Report only [`Event::RepositoryStatusRevision`], no per-file events.
    pub revision_only: bool,
    /// Report the number of changed files rather than the files themselves.
    pub count: bool,
}

impl RepositoryStatusArgs<'_> {
    /// The paths as Lore's own string type, separate from [`Self::to_raw`] for
    /// the reason [`FileInfoArgs::raw_paths`] is.
    fn raw_paths(self) -> Vec<lore_string_t> {
        self.paths.iter().copied().map(raw_str).collect()
    }

    /// The raw struct to hand to Lore, borrowing `paths`.
    fn to_raw(self, paths: &[lore_string_t]) -> lore_repository_status_args_t {
        lore_repository_status_args_t {
            staged: u8::from(self.staged),
            scan: u8::from(self.scan),
            check_dirty: u8::from(self.check_dirty),
            reset: u8::from(self.reset),
            sync_point: u8::from(self.sync_point),
            revision_only: u8::from(self.revision_only),
            count: u8::from(self.count),
            paths: raw_str_array(paths),
        }
    }
}

/// Reports where a repository stands: the revision and branch it is on, and
/// what has changed against them.
///
/// [`Event::RepositoryStatusRevision`] carries the revision the repository is
/// on, which is the one to hand [`revision_tree_load`] to read that same
/// revision. With `scan` and `check_dirty` left off no filesystem read happens
/// beyond the dirty flags already recorded.
///
/// This corresponds to `lore_sys::Lore::lore_repository_status`.
pub fn repository_status(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: RepositoryStatusArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    let paths = args.raw_paths();
    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `paths` and `args`, which both live across the call.
    unsafe {
        call_with_callback(
            lore.lore_repository_status,
            command,
            globals,
            &args.to_raw(&paths),
            callback,
        )
    }
}

/// Loads the directory tree of a revision.
///
/// This corresponds to `lore_sys::Lore::lore_revision_tree_load`.
pub fn revision_tree_load(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: lore_revision_tree_load_args_t,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the arguments
    // hold no pointers.
    unsafe {
        call_with_callback(
            lore.lore_revision_tree_load,
            command,
            globals,
            &args,
            callback,
        )
    }
}

/// Arguments for [`revision_tree_resolve_path`]. No [`Default`]: there is no
/// meaningful tree handle to default to.
#[derive(Debug, Clone, Copy)]
pub struct RevisionTreeResolvePathArgs<'a> {
    /// Echoed back on the event, to tell concurrent calls apart.
    pub id: u64,
    /// The tree to resolve against.
    pub handle: lore_revision_tree_t,
    /// Path relative to the tree root; empty resolves to the root node.
    pub path: &'a str,
}

impl RevisionTreeResolvePathArgs<'_> {
    /// The raw struct to hand to Lore, borrowing the same text `self` does.
    fn to_raw(self) -> lore_revision_tree_resolve_path_args_t {
        lore_revision_tree_resolve_path_args_t {
            id: self.id,
            handle: self.handle,
            path: raw_str(self.path),
        }
    }
}

/// Resolves a path in a loaded revision tree to the node that holds it.
///
/// Answered from the loaded tree: no filesystem read, and no revision state
/// deserialized per call the way [`file_info`] does it. One
/// [`Event::RevisionTreeResolvePathComplete`] concludes the call, carrying
/// either the node or the reason there is none in its `error_code`. A path that
/// crosses a sub-repository link resolves in the link's target tree, which the
/// event names through its `repository` and `revision`.
///
/// This corresponds to `lore_sys::Lore::lore_revision_tree_resolve_path`.
pub fn revision_tree_resolve_path(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: RevisionTreeResolvePathArgs<'_>,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the raw struct
    // borrows from `args`, which lives across the call.
    unsafe {
        call_with_callback(
            lore.lore_revision_tree_resolve_path,
            command,
            globals,
            &args.to_raw(),
            callback,
        )
    }
}

/// Reports one node of a loaded revision tree: its address, size and kind.
///
/// Answered from the loaded tree, like [`revision_tree_resolve_path`]. One
/// [`Event::RevisionTreeNodeInfo`] concludes the call, carrying the failure in
/// its `error_code` when there is one.
///
/// This corresponds to `lore_sys::Lore::lore_revision_tree_node_info`.
pub fn revision_tree_node_info(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: lore_revision_tree_node_info_args_t,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the arguments
    // hold no pointers.
    unsafe {
        call_with_callback(
            lore.lore_revision_tree_node_info,
            command,
            globals,
            &args,
            callback,
        )
    }
}

/// Streams the children of a directory node.
///
/// One [`Event::RevisionTreeListChildrenBegin`] first, carrying the outcome
/// and the tree the children belong to — a link's target tree when the parent
/// is a sub-repository link — then one [`Event::RevisionTreeChild`] per entry.
/// An empty directory emits none before the completion.
///
/// This corresponds to `lore_sys::Lore::lore_revision_tree_list_children`.
pub fn revision_tree_list_children(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: lore_revision_tree_list_children_args_t,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the arguments
    // hold no pointers.
    unsafe {
        call_with_callback(
            lore.lore_revision_tree_list_children,
            command,
            globals,
            &args,
            callback,
        )
    }
}

/// Reconstructs the path of a node by walking its parents. One
/// [`Event::RevisionTreeNodePath`] concludes the call.
///
/// This corresponds to `lore_sys::Lore::lore_revision_tree_node_path`.
pub fn revision_tree_node_path(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: lore_revision_tree_node_path_args_t,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the arguments
    // hold no pointers.
    unsafe {
        call_with_callback(
            lore.lore_revision_tree_node_path,
            command,
            globals,
            &args,
            callback,
        )
    }
}

/// Reports the loaded revision itself: parents, author and creation time. One
/// [`Event::RevisionTreeInfo`] concludes the call.
///
/// This corresponds to `lore_sys::Lore::lore_revision_tree_info`.
pub fn revision_tree_info(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: lore_revision_tree_info_args_t,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the arguments
    // hold no pointers.
    unsafe {
        call_with_callback(
            lore.lore_revision_tree_info,
            command,
            globals,
            &args,
            callback,
        )
    }
}

/// Releases a revision-tree handle.
///
/// This corresponds to `lore_sys::Lore::lore_revision_tree_close`.
pub fn revision_tree_close(
    lore: &crate::Lore,
    command: &'static str,
    globals: &GlobalArgs,
    args: lore_revision_tree_close_args_t,
    callback: impl FnMut(Result<Event<'_>, std::str::Utf8Error>) + Send,
) -> Result<(), LoreError> {
    // SAFETY: the entry point is the loaded library's own, and the arguments
    // hold no pointers.
    unsafe {
        call_with_callback(
            lore.lore_revision_tree_close,
            command,
            globals,
            &args,
            callback,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LoreStringExt;
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
                "test::scripted",
                &GlobalArgs::default(),
                &Script { events, status },
                callback,
            )
        }
    }

    /// The status and messages of a `LoreError::Call`, which is what every
    /// failure below produces.
    fn call_failure(error: LoreError) -> (i32, Vec<String>) {
        match error {
            LoreError::Call {
                status, messages, ..
            } => (status, messages),
            other => panic!("expected a call failure, got {other:?}"),
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
    fn auth_login_with_token_args_conversion() {
        // The shape a login against the repository's own server takes: only
        // the token and its type, the two URLs left to Lore to resolve, which
        // it tells from unset rather than from a pointer to zero bytes.
        let raw = AuthLoginWithTokenArgs {
            token: "eg1~token",
            token_type: "eg1",
            ..Default::default()
        }
        .to_raw();

        assert_eq!(unsafe { raw.token.try_to_str() }, Ok("eg1~token"));
        assert_eq!(unsafe { raw.token_type.try_to_str() }, Ok("eg1"));
        assert!(raw.remote_url.string.is_null());
        assert!(raw.auth_url.string.is_null());
    }

    #[test]
    fn auth_login_with_token_args_do_not_print_the_token() {
        let printed = format!(
            "{:?}",
            AuthLoginWithTokenArgs {
                token: "eg1~token",
                token_type: "eg1",
                ..Default::default()
            }
        );

        assert!(!printed.contains("eg1~token"), "{printed}");
        assert!(printed.contains("eg1"), "the type is not a secret");
    }

    #[test]
    fn an_auth_user_info_event_decodes_the_identity() {
        let mut event: lore_event_t = unsafe { std::mem::zeroed() };
        event.tag = lore_sys::LORE_EVENT_AUTH_USER_INFO as lore_event_tag_t;
        event.__bindgen_anon_1.auth_user_info.id = lore_string_t::from_str("user-id");
        event.__bindgen_anon_1.auth_user_info.name = lore_string_t::from_str("Some One");

        let mut seen = None;
        run(&[event], 0, |event| {
            if let Ok(Event::AuthUserInfo { id, name }) = event {
                seen = Some((id.to_owned(), name.to_owned()));
            }
        })
        .unwrap();

        assert_eq!(seen, Some(("user-id".to_owned(), "Some One".to_owned())));
    }

    #[test]
    fn an_auth_identity_event_decodes_its_resource_and_expiry() {
        let mut event: lore_event_t = unsafe { std::mem::zeroed() };
        event.tag = lore_sys::LORE_EVENT_AUTH_IDENTITY as lore_event_tag_t;
        event.__bindgen_anon_1.auth_identity.user_id = lore_string_t::from_str("user-id");
        event.__bindgen_anon_1.auth_identity.authorized_domains =
            lore_string_t::from_str("example.com, auth.example.com");
        event.__bindgen_anon_1.auth_identity.expires = 1_700_000_000_000;

        let mut seen = None;
        run(&[event], 0, |event| {
            if let Ok(Event::AuthIdentity {
                resource,
                user_id,
                authorized_domains,
                expires,
                ..
            }) = event
            {
                seen = Some((
                    resource.to_owned(),
                    user_id.to_owned(),
                    authorized_domains.to_owned(),
                    expires,
                ));
            }
        })
        .unwrap();

        // An empty resource is an authentication token, and the expiry is
        // milliseconds, passed on as Lore reports it.
        assert_eq!(
            seen,
            Some((
                String::new(),
                "user-id".to_owned(),
                "example.com, auth.example.com".to_owned(),
                1_700_000_000_000,
            ))
        );
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
    fn branch_info_args_conversion() {
        let raw = BranchInfoArgs { branch: "main" }.to_raw();
        assert_eq!(unsafe { raw.branch.try_to_str() }, Ok("main"));
    }

    #[test]
    fn branch_info_args_without_a_branch_pass_a_null_pointer() {
        // An empty branch means "the one the repository is on", which lore
        // tells from unset rather than from a pointer to zero bytes.
        let raw = BranchInfoArgs::default().to_raw();
        assert!(raw.branch.string.is_null());
        assert_eq!(raw.branch.length, 0);
    }

    #[test]
    fn a_branch_info_event_decodes_both_tips() {
        let mut event: lore_event_t = unsafe { std::mem::zeroed() };
        event.tag = lore_sys::LORE_EVENT_BRANCH_INFO as lore_event_tag_t;
        event.__bindgen_anon_1.branch_info.name = lore_string_t::from_str("main");
        event.__bindgen_anon_1.branch_info.latest = lore_sys::lore_hash_t { data: [1; 32] };
        event.__bindgen_anon_1.branch_info.latest_remote = lore_sys::lore_hash_t { data: [2; 32] };
        event.__bindgen_anon_1.branch_info.archived = 1;

        let mut seen = None;
        run(&[event], 0, |event| {
            if let Ok(Event::BranchInfo {
                name,
                latest,
                latest_remote,
                archived,
                ..
            }) = event
            {
                seen = Some((name.to_owned(), latest.data, latest_remote.data, archived));
            }
        })
        .unwrap();

        assert_eq!(seen, Some(("main".to_owned(), [1; 32], [2; 32], true)));
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
        assert_eq!(
            error.to_string(),
            "`test::scripted` failed with status 5: 7: it broke"
        );
        assert_eq!(call_failure(error), (5, vec!["7: it broke".to_owned()]));
    }

    #[test]
    fn the_completion_message_describes_a_failure_without_error_events() {
        let error = run(&[complete_event(3, "no such branch")], 3, |_| {}).unwrap_err();
        assert_eq!(call_failure(error).1, ["no such branch"]);
    }

    #[test]
    fn a_failure_before_any_event_reports_only_its_status() {
        let error = run(&[], 2, |_| {}).unwrap_err();
        assert_eq!(error.to_string(), "`test::scripted` failed with status 2");
        assert_eq!(call_failure(error), (2, vec![]));
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
        let messages = call_failure(error).1;
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("invalid UTF-8"), "{messages:?}");
    }

    #[test]
    fn invalid_utf8_in_a_log_event_reaches_the_callback_but_not_the_error() {
        let mut event = log_event("");
        event.__bindgen_anon_1.log.message = invalid_utf8();
        let mut callback_saw_it = false;
        let error = run(&[event], 1, |event| callback_saw_it |= event.is_err()).unwrap_err();
        assert!(callback_saw_it);
        assert!(call_failure(error).1.is_empty());
    }
}
