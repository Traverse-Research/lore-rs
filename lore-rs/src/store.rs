use std::io::Write;

use lore_sys::{
    lore_error_code_t, lore_fragment_t, lore_storage_close_args_t, lore_storage_flush_args_t,
    lore_store_t,
};

use crate::{
    Address, ContextId, ErrorCode, Event, GlobalArgs, Lore, LoreError, RepositoryId, Revision,
    RevisionTree, StorageGetArgs, StorageGetItem, StorageGetMetadataArgs, StorageGetMetadataItem,
    StorageOpenArgs, StoragePutArgs, StoragePutItem,
};

const PUT: &str = "storage::put";
const METADATA: &str = "storage::get_metadata";
const FLUSH: &str = "storage::flush";

/// Soft caps on the local store. Zero for either selects Lore's default; zero
/// for **both** leaves the evictor and compactor unspawned, so the store grows
/// without limit. Handles on the same on-disk store inherit the first
/// opener's targets.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CacheTargets {
    pub bytes: u64,
    pub fragments: u64,
}

/// Where a store's data lives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum StoreLocation {
    /// A repository instance's directory, the one holding `.lore`. The stores
    /// inside it back the handle: `.lore/immutable` and `.lore/mutable`, or
    /// the shared store its `config.toml` points at. Lore calls this argument
    /// `repository_path` because that is where on-disk stores live; the
    /// handle itself serves any repository, see [`Store::get`]. Two handles on
    /// the same path share one underlying store.
    OnDisk(String),
    /// A fresh, private store in memory. Nothing outlives the handle.
    #[default]
    InMemory,
}

/// How to open a [`Store`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreOptions {
    pub location: StoreLocation,
    /// The server, as `lore://host:port` without a repository name. Without
    /// one the store can only answer with what it already holds.
    pub remote_url: Option<String>,
    pub cache_targets: CacheTargets,
    /// Keep the bytes a read fetches from the server in the local store, so
    /// the next read of the same address is local. Off, Lore keeps only what
    /// the producer flagged for local caching, and every read of a remote
    /// address is a round trip. Lore's storage API takes this per read item;
    /// the store applies it to every read it makes.
    pub local_cache: bool,
}

/// What Lore should do with one buffer beyond storing it. All-zero is Lore's
/// own default.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PutOptions {
    /// Also write the content to the server. Ignored on a handle without a
    /// `remote_url`, so a put that succeeded offline is durable here only.
    pub remote_write: bool,
    /// Tag the fragments so every later remote read of them is kept locally,
    /// whatever the reader asked for.
    pub local_cache: bool,
    /// Cap on the leaf fragment size a large buffer is split into; zero lets
    /// Lore choose. Ignored below Lore's fragmentation threshold.
    pub fixed_size_chunk: u64,
}

/// What a store holds for one address, without its bytes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FragmentInfo {
    /// Size of the content once reassembled and decompressed, which is how
    /// many bytes [`Store::get`] of this address answers with.
    pub size_content: u64,
    /// Size of the payload as stored, which differs from `size_content` when
    /// Lore compressed the content or split it into fragments.
    pub size_payload: u32,
    /// Lore's fragment flags as it reports them. The C API names none of the
    /// bits, so neither does this.
    pub flags: u32,
}

impl FragmentInfo {
    pub(crate) const fn from_raw(raw: lore_fragment_t) -> Self {
        Self {
            size_content: raw.size_content,
            size_payload: raw.size_payload,
            flags: raw.flags,
        }
    }
}

/// One buffer for [`Store::put`] to store.
#[derive(Debug, Default, Clone, Copy)]
pub struct PutItem<'a> {
    /// The partition to write to, which is what a later read of it is
    /// authorized against; see [`Store::get`]. [`RepositoryId::ZERO`] is
    /// rejected.
    pub repository: RepositoryId,
    /// Which file the bytes belong to, the context half of their [`Address`].
    pub context: ContextId,
    pub data: &'a [u8],
    pub options: PutOptions,
}

/// An open content-addressed store, closed on drop.
///
/// The handle is repository-agnostic: it is bound to a location and
/// optionally a server, and every read and write names the repository it is
/// about, because an [`Address`] alone does not. See [`Self::get`] and
/// [`Self::put`].
///
/// `Send + Sync`; Lore allows concurrent operations on one handle.
///
/// # Making writes stick
///
/// [`put`](Self::put) returns when Lore has the bytes, not when they are on
/// disk. Either call [`flush`](Self::flush), or end the process with
/// [`Lore::shutdown`], which waits for the flush that dropping a handle
/// starts. Do neither and a process that writes and exits can lose what it
/// wrote. Getting the bytes to the *server* is a separate question again,
/// decided per item by [`PutOptions::remote_write`].
pub struct Store {
    lore: &'static Lore,
    globals: GlobalArgs,
    handle: lore_store_t,
    local_cache: bool,
}

impl Store {
    /// Opens a store.
    ///
    /// Lore binds part of `globals` into the handle here, and a later call can
    /// only tighten them, never loosen:
    ///
    /// - `identity` authenticates against `remote_url`.
    /// - `offline` and `local` both forbid the handle from reaching the
    ///   server; `remote` makes reads bypass the local store, and requires a
    ///   `remote_url`. `local` together with `remote` is rejected.
    /// - `no_gc` leaves the evictor and compactor unspawned whatever
    ///   `cache_targets` say.
    ///
    /// `globals.repository_path` is not read; the location is
    /// `options.location`. Lore refuses an [`OnDisk`](StoreLocation::OnDisk)
    /// path without a `.lore` directory rather than fabricating a repository
    /// there. The store keeps a copy of `globals` for the calls it makes.
    ///
    /// This corresponds to `lore_sys::Lore::lore_storage_open`.
    pub fn open(
        lore: &'static Lore,
        globals: &GlobalArgs,
        options: StoreOptions,
    ) -> Result<Store, LoreError> {
        const COMMAND: &str = "storage::open";
        let mut handle = None;

        let (repository_path, in_memory) = match &options.location {
            StoreLocation::OnDisk(path) => (path.as_str(), false),
            StoreLocation::InMemory => ("", true),
        };

        crate::call::storage_open(
            lore,
            COMMAND,
            globals,
            StorageOpenArgs {
                repository_path,
                in_memory,
                remote_url: options.remote_url.as_deref(),
                cache_target_bytes: options.cache_targets.bytes,
                cache_target_fragments: options.cache_targets.fragments,
            },
            |event| {
                crate::log_event(&event);

                if let Ok(Event::StorageOpened { handle_id }) = event {
                    handle = Some(lore_store_t { handle_id });
                }
            },
        )?;

        Ok(Store {
            lore,
            globals: globals.clone(),
            handle: handle.ok_or(LoreError::MissingEvent {
                command: COMMAND,
                expected: "storage_opened",
            })?,
            local_cache: options.local_cache,
        })
    }

    pub fn lore(&self) -> &'static Lore {
        self.lore
    }

    /// The globals the store was opened with, which its own calls use.
    pub fn globals(&self) -> &GlobalArgs {
        &self.globals
    }

    /// The raw handle, for a call this crate does not wrap.
    pub fn handle(&self) -> lore_store_t {
        self.handle
    }

    /// Whether reads keep what they fetch, see
    /// [`StoreOptions::local_cache`].
    pub fn local_cache(&self) -> bool {
        self.local_cache
    }

    /// Loads one revision's tree.
    ///
    /// The returned handle *is* that revision — a lookup through it cannot
    /// reach another one. Its blocks come from this store where it has them
    /// and from the server where it does not, so this works against a `.lore`
    /// holding nothing but `id` and `config.toml`, and fails when the revision
    /// cannot be reached at all. The tree keeps its own reference to the
    /// store inside Lore, so it stays usable after this `Store` is dropped.
    ///
    /// This corresponds to `lore_sys::Lore::lore_revision_tree_load`.
    pub fn load_revision_tree(
        &self,
        repository: RepositoryId,
        revision: Revision,
    ) -> Result<RevisionTree, LoreError> {
        crate::revision_tree::load(self, repository, revision)
    }

    /// Stores one buffer, and returns the address it landed at: the hash of
    /// the bytes together with [`PutItem::context`]. An empty buffer stores
    /// nothing and answers with the zero hash.
    ///
    /// **A put creates no revision-tree entry.** Nothing in the repository
    /// points at the address afterwards, so the caller has to keep it — in a
    /// mutable key, an index of its own, or a commit — or the content is
    /// written and unreachable.
    ///
    /// Nothing is flushed by the time this returns: closing a store only
    /// *spawns* its flush, so a process that writes and exits can lose what
    /// it wrote. See [`Lore::shutdown`](crate::Lore::shutdown).
    ///
    /// This corresponds to `lore_sys::Lore::lore_storage_put` with one item.
    pub fn put(&self, item: PutItem<'_>) -> Result<Address, LoreError> {
        self.put_many(&[item])?
            .into_iter()
            .next()
            .ok_or(LoreError::MissingEvent {
                command: PUT,
                expected: "storage_put_item_complete",
            })?
    }

    /// Stores several buffers in one call, which Lore hashes and writes
    /// concurrently: one FFI round trip rather than one per object. The
    /// results come back in the order the items were given.
    ///
    /// **One failed item does not fail the call.** The outer error is left
    /// for what goes wrong with the call as a whole, and what one item alone
    /// can fail at is inside, so a batch of a thousand with one bad entry
    /// still returns nine hundred and ninety-nine addresses.
    ///
    /// Same rules as [`Self::put`] otherwise.
    ///
    /// This corresponds to `lore_sys::Lore::lore_storage_put`.
    pub fn put_many(
        &self,
        items: &[PutItem<'_>],
    ) -> Result<Vec<Result<Address, LoreError>>, LoreError> {
        let raw_items = items
            .iter()
            .enumerate()
            .map(|(index, item)| StoragePutItem {
                // The position is the id, so a completion lands back in its
                // own item's slot however Lore interleaves them.
                id: index as u64,
                partition: item.repository.to_raw(),
                context: item.context.to_raw(),
                data: item.data,
                remote_write: item.options.remote_write,
                local_cache: item.options.local_cache,
                fixed_size_chunk: item.options.fixed_size_chunk,
            })
            .collect::<Vec<_>>();

        let mut completions: Vec<Option<(Address, lore_error_code_t)>> = vec![None; items.len()];

        let result = crate::call::storage_put(
            self.lore,
            PUT,
            &self.globals,
            StoragePutArgs {
                handle: self.handle,
                items: &raw_items,
            },
            |event| {
                crate::log_event(&event);

                if let Ok(Event::StoragePutItemComplete {
                    id,
                    address,
                    error_code,
                }) = event
                {
                    // An id from outside the batch leaves the item it was
                    // meant for unreported, which fails the batch below.
                    if let Some(slot) = usize::try_from(id)
                        .ok()
                        .and_then(|index| completions.get_mut(index))
                    {
                        *slot = Some((Address::from_raw(address), error_code));
                    }
                }
            },
        );

        put_outcomes(completions, result)
    }

    /// What this store holds for `address`, without fetching its bytes, or
    /// [`None`] when neither this store nor the server has it.
    ///
    /// `repository` is the partition the lookup is authorized against, the
    /// same rule as [`Self::get`]. Lore probes the local store and falls
    /// through to the server, so this answers whether the handle can reach
    /// the content rather than whether it is already here; `globals.local`
    /// confines it to the local store, `globals.remote` to the server.
    /// Nothing is cached either way, there being no bytes to cache.
    ///
    /// The zero hash is content Lore always has, and answers with an empty
    /// fragment — the same address [`Self::get`] reads as an empty buffer and
    /// [`Self::put`] returns for one.
    ///
    /// This corresponds to `lore_sys::Lore::lore_storage_get_metadata`.
    pub fn metadata(
        &self,
        repository: RepositoryId,
        address: Address,
    ) -> Result<Option<FragmentInfo>, LoreError> {
        let mut completion: Option<(lore_fragment_t, lore_error_code_t)> = None;

        let result = crate::call::storage_get_metadata(
            self.lore,
            METADATA,
            &self.globals,
            StorageGetMetadataArgs {
                handle: self.handle,
                items: &[StorageGetMetadataItem {
                    id: 0,
                    partition: repository.to_raw(),
                    address: address.to_raw(),
                }],
            },
            |event| {
                crate::log_event(&event);

                if let Ok(Event::StorageGetMetadataItemComplete {
                    fragment,
                    error_code,
                    ..
                }) = event
                {
                    completion = Some((fragment, error_code));
                }
            },
        );

        metadata_outcome(completion, result)
    }

    /// Whether this store can reach the content at `address`. Reads no bytes:
    /// this is [`Self::metadata`] without the fragment it found.
    pub fn exists(&self, repository: RepositoryId, address: Address) -> Result<bool, LoreError> {
        Ok(self.metadata(repository, address)?.is_some())
    }

    /// Waits for what this store has written to reach the disk.
    /// [`GlobalArgs::sync_data`] decides whether Lore also forces the media
    /// to sync it.
    ///
    /// Call it where a write has to be safe before something else happens:
    /// at the end of a cook, before reporting success, before exiting.
    ///
    /// Dropping a `Store` starts a flush too, and [`Lore::shutdown`] waits
    /// for it, so dropping every handle and then shutting down is durable as
    /// well. The difference is the error: Lore throws away what the flush on
    /// the drop path reports, and this one hands it back, so a disk that
    /// filled up is silent there and an `Err` here.
    ///
    /// Two things this does not do. It sends nothing to the server — that is
    /// [`PutOptions::remote_write`], chosen per item when the write is made.
    /// And it is not limited to this handle's own writes: handles on one
    /// [`OnDisk`](StoreLocation::OnDisk) location share the store underneath,
    /// so this flushes whatever is pending in it. An
    /// [`InMemory`](StoreLocation::InMemory) store has nothing to flush and
    /// succeeds.
    ///
    /// This corresponds to `lore_sys::Lore::lore_storage_flush`.
    pub fn flush(&self) -> Result<(), LoreError> {
        crate::call::storage_flush(
            self.lore,
            FLUSH,
            &self.globals,
            lore_storage_flush_args_t {
                handle: self.handle,
            },
            |event| crate::log_event(&event),
        )
    }

    /// Reads one address in full, fetching from the server whatever this store
    /// does not hold.
    ///
    /// `repository` is the partition the read is authorized against. An
    /// address is `(hash, context)`: the hash says what the bytes are and the
    /// context which file they belong to, but neither says who may read them.
    /// The same bytes can live in many repositories, and Lore never lets a
    /// hash alone reach content in a repository the caller is not entitled to,
    /// so each read names one. [`RevisionTree::repository`] is the one for
    /// anything found through a tree.
    ///
    /// The buffer is sized from the header Lore sends first, and a delivery
    /// that does not cover exactly that is [`LoreError::SizeMismatch`]. An
    /// address neither side holds is [`LoreError::Failed`] with
    /// [`ErrorCode::AddressNotFound`](crate::ErrorCode::AddressNotFound), see
    /// [`LoreError::is_not_found`]. The zero hash reads as an empty buffer.
    ///
    /// Lore reassembles the content before handing it over, so the whole
    /// buffer exists twice for a moment; [`Self::read_to`] streams instead.
    /// Not retried: Lore's own read path backs off on `SlowDown` sixty times
    /// and re-establishes the session on a reconnect.
    ///
    /// This corresponds to `lore_sys::Lore::lore_storage_get`.
    pub fn get(&self, repository: RepositoryId, address: Address) -> Result<Vec<u8>, LoreError> {
        const COMMAND: &str = "storage::get";

        let mut data: Option<Vec<u8>> = None;
        // Fragments are not promised in order, so `data` is written by offset
        // and cannot report progress by its own length. `covered` sums what
        // was written and `end` tracks how far it reached: a response that
        // skipped a range fails one or the other even when its ranges add up.
        let mut covered = 0u64;
        let mut end = 0u64;
        let mut overflow = None;
        let mut outcome: Option<lore_error_code_t> = None;

        let result = crate::call::storage_get(
            self.lore,
            COMMAND,
            &self.globals,
            StorageGetArgs {
                handle: self.handle,
                items: &[self.item(repository, address, false)],
            },
            |event| {
                crate::log_event(&event);

                match event {
                    Ok(Event::StorageGetHeader { size_content, .. }) => {
                        data = Some(vec![0u8; size_content as usize]);
                    }
                    Ok(Event::StorageGetData { offset, bytes, .. }) => {
                        let length = bytes.len() as u64;
                        let Some(buffer) = data.as_mut() else {
                            // Data before the header is not a shape Lore
                            // produces; it fails the size check below.
                            overflow = Some(offset.saturating_add(length));
                            return;
                        };
                        let size = buffer.len() as u64;
                        let Some(range_end) = offset
                            .checked_add(length)
                            .filter(|range_end| *range_end <= size)
                        else {
                            overflow = Some(offset.saturating_add(length));
                            return;
                        };

                        buffer[offset as usize..range_end as usize].copy_from_slice(bytes);
                        covered += length;
                        end = end.max(range_end);
                    }
                    Ok(Event::StorageGetItemComplete { error_code, .. }) => {
                        outcome = Some(error_code);
                    }
                    _ => {}
                }
            },
        );
        LoreError::resolve(COMMAND, result, outcome)?;

        let data = data.ok_or(LoreError::MissingEvent {
            command: COMMAND,
            expected: "storage_get_header",
        })?;
        let size = data.len() as u64;

        if let Some(over) = overflow {
            return Err(LoreError::SizeMismatch {
                command: COMMAND,
                expected: size,
                covered: over,
            });
        }
        if covered != size || end != size {
            return Err(LoreError::SizeMismatch {
                command: COMMAND,
                expected: size,
                covered: covered.min(end),
            });
        }

        Ok(data)
    }

    /// Reads one address into `sink`, one Lore fragment at a time, and
    /// returns how many bytes that was. Peak memory follows the fragment size
    /// rather than the content size, which is the point for the large files
    /// Lore is built for.
    ///
    /// Same rules as [`Self::get`] for `repository`, not-found and size
    /// checks. A `sink` that refuses bytes is [`LoreError::Io`]; the read is
    /// still driven to its end, since Lore delivers the fragments either way.
    /// Lore's streaming read delivers fragments in offset order.
    ///
    /// This corresponds to `lore_sys::Lore::lore_storage_get` with
    /// `streaming` set.
    pub fn read_to<W: Write + Send>(
        &self,
        repository: RepositoryId,
        address: Address,
        sink: &mut W,
    ) -> Result<u64, LoreError> {
        const COMMAND: &str = "storage::get";

        let mut expected: Option<u64> = None;
        let mut written = 0u64;
        let mut out_of_order = false;
        let mut io_error: Option<std::io::Error> = None;
        let mut outcome: Option<lore_error_code_t> = None;

        let result = crate::call::storage_get(
            self.lore,
            COMMAND,
            &self.globals,
            StorageGetArgs {
                handle: self.handle,
                items: &[self.item(repository, address, true)],
            },
            |event| {
                crate::log_event(&event);

                match event {
                    Ok(Event::StorageGetHeader { size_content, .. }) => {
                        expected = Some(size_content);
                    }
                    Ok(Event::StorageGetData { offset, bytes, .. }) => {
                        if io_error.is_some() {
                            return;
                        }
                        if offset != written {
                            out_of_order = true;
                            return;
                        }
                        match sink.write_all(bytes) {
                            Ok(()) => written += bytes.len() as u64,
                            Err(error) => io_error = Some(error),
                        }
                    }
                    Ok(Event::StorageGetItemComplete { error_code, .. }) => {
                        outcome = Some(error_code);
                    }
                    _ => {}
                }
            },
        );
        LoreError::resolve(COMMAND, result, outcome)?;

        if let Some(source) = io_error {
            return Err(LoreError::Io {
                command: COMMAND,
                source,
            });
        }
        let expected = expected.ok_or(LoreError::MissingEvent {
            command: COMMAND,
            expected: "storage_get_header",
        })?;
        if out_of_order || written != expected {
            return Err(LoreError::SizeMismatch {
                command: COMMAND,
                expected,
                covered: written,
            });
        }

        Ok(written)
    }

    fn item(&self, repository: RepositoryId, address: Address, streaming: bool) -> StorageGetItem {
        StorageGetItem {
            id: 0,
            partition: repository.to_raw(),
            address: address.to_raw(),
            streaming,
            local_cache: self.local_cache,
        }
    }
}

/// Folds what a put reported into one result per item, in the order the items
/// were given. Kept out of the event callback, which cannot fail the call.
fn put_outcomes(
    completions: Vec<Option<(Address, lore_error_code_t)>>,
    result: Result<(), LoreError>,
) -> Result<Vec<Result<Address, LoreError>>, LoreError> {
    // Nothing was said about a missing item, so whatever went wrong with the
    // call stands for the batch.
    if completions.iter().any(Option::is_none) {
        result?;
        return Err(LoreError::MissingEvent {
            command: PUT,
            expected: "storage_put_item_complete",
        });
    }

    let outcomes: Vec<Result<Address, LoreError>> = completions
        .into_iter()
        .flatten()
        .map(|(address, code)| match ErrorCode::from_raw(code) {
            None => Ok(address),
            // A put's messages belong to the call and only count its failed
            // items, so an item's code is all there is to report.
            Some(code) => Err(LoreError::Failed {
                command: PUT,
                code,
                messages: Vec::new(),
            }),
        })
        .collect();

    // A call failure the items already account for gives way to them. One no
    // item accounts for is all there is to report.
    if outcomes.iter().all(Result::is_ok) {
        result?;
    }

    Ok(outcomes)
}

/// Turns what a metadata lookup reported into the fragment it found, folding
/// a miss to [`None`] rather than an error.
fn metadata_outcome(
    completion: Option<(lore_fragment_t, lore_error_code_t)>,
    result: Result<(), LoreError>,
) -> Result<Option<FragmentInfo>, LoreError> {
    let Some((fragment, code)) = completion else {
        result?;
        return Err(LoreError::MissingEvent {
            command: METADATA,
            expected: "storage_get_metadata_item_complete",
        });
    };

    // A miss is this call's answer rather than its failure, and it is also
    // what failed the call, so the call's own error goes with it.
    if ErrorCode::from_raw(code) == Some(ErrorCode::AddressNotFound) {
        return Ok(None);
    }

    LoreError::resolve(METADATA, result, Some(code))?;

    Ok(Some(FragmentInfo::from_raw(fragment)))
}

impl Drop for Store {
    /// Closes the handle. Lore spawns the store's flush rather than waiting
    /// for it; see [`Lore::shutdown`] for what that means at process exit,
    /// and [`Store::flush`] for a durability point that reports its own
    /// failure. A failure to close is logged, since nothing can act on it
    /// here.
    fn drop(&mut self) {
        let result = crate::call::storage_close(
            self.lore,
            "storage::close",
            &self.globals,
            lore_storage_close_args_t {
                handle: self.handle,
            },
            |event| crate::log_event(&event),
        );
        if let Err(error) = result {
            log::error!(target: "lore", "{error}");
        }
    }
}
