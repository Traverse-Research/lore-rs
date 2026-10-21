use std::io::Write;
use std::ops::{Bound, RangeBounds};

use lore_sys::{
    lore_error_code_t, lore_fragment_t, lore_storage_close_args_t, lore_storage_flush_args_t,
    lore_store_t, LORE_KEY_TYPE_RESOLVE,
};

use crate::{
    Address, Backend, ContextId, ErrorCode, Event, GlobalArgs, Lore, LoreError, RepositoryId,
    ResolveKey, Revision, RevisionTree, StorageGetArgs, StorageGetItem, StorageGetMetadataArgs,
    StorageGetMetadataItem, StorageGetResolvedArgs, StorageGetResolvedItem,
    StorageMutableCompareAndSwapArgs, StorageMutableCompareAndSwapItem, StorageMutableLoadArgs,
    StorageMutableLoadItem, StorageOpenArgs, StoragePutArgs, StoragePutItem,
    StoragePutResolvedArgs, StoragePutResolvedItem,
};

const PUT: &str = "storage::put";
const PUT_RESOLVED: &str = "storage::put_resolved";
const METADATA: &str = "storage::get_metadata";
const GET_RESOLVED: &str = "storage::get_resolved";
const RESOLVE: &str = "storage::mutable_load";
const COMPARE_AND_SWAP: &str = "storage::mutable_compare_and_swap";
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
    /// Skip re-hashing a loaded payload to check it against the address it
    /// was read from. Off, every byte of every read is verified, which is the
    /// right default for a store nothing else vouches for. Set it when a layer
    /// of your own already assures integrity, such as a store scrubbed on a
    /// schedule.
    pub skip_verify: bool,
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

/// One buffer for [`Store::put_resolved`] to store and publish a key for.
#[derive(Debug, Default, Clone, Copy)]
pub struct PutResolvedItem<'a> {
    /// The partition to write to, which is what a later
    /// [`Store::get_resolved`] is authorized against.
    /// [`RepositoryId::ZERO`] is rejected.
    pub repository: RepositoryId,
    /// The mutable key to publish the stored content under, always under
    /// `LORE_KEY_TYPE_RESOLVE`. The zero key is rejected.
    pub key: ResolveKey,
    /// Which file the bytes belong to: the context half of the resulting
    /// [`Address`], and the context [`Store::get_resolved`] must read `key`
    /// at.
    pub context: ContextId,
    /// The bytes to hash, store and publish `key` for. Empty **removes**
    /// `key`'s mapping instead of publishing one, reported back as the zero
    /// address on [`PutResolvedOutcome::address`]. Without
    /// `remote_write` that only evicts the local mapping, and a key published
    /// remotely resolves again on the next call.
    pub data: &'a [u8],
    pub options: PutOptions,
}

/// What [`Store::put_resolved`] reported for the key it published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PutResolvedOutcome {
    /// The content `key` now resolves to; the zero hash when
    /// [`PutResolvedItem::data`] was empty.
    pub address: Address,
    /// Whether the local store holds the mapping.
    pub stored_local: bool,
    /// Whether the mapping reached the remote, or was already durable there.
    /// Check this, not whether the call succeeded, to confirm the key is
    /// visible to other clients: a remote write that fails still succeeds
    /// here if the local write landed.
    pub stored_remote: bool,
}

/// One key for [`Store::get_resolved_many`] to resolve and read.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GetResolvedItem {
    /// The partition the key is resolved and read in, as for
    /// [`Store::get_resolved`]. [`RepositoryId::ZERO`] is rejected.
    pub repository: RepositoryId,
    /// The mutable key to resolve, under `LORE_KEY_TYPE_RESOLVE`. The zero
    /// key is rejected.
    pub key: ResolveKey,
    /// The context the resolved hash is completed with into an [`Address`].
    pub context: ContextId,
}

/// What resolving a mutable key answered: the address it resolved to,
/// together with what the read produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved<T> {
    /// The address the key resolved to.
    pub address: Address,
    pub data: T,
}

/// What [`Store::compare_and_swap_resolved`] found the key holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapOutcome {
    /// The key held the expected value and now holds the new one.
    Swapped,
    /// The key held something else, and was left untouched.
    Conflict {
        /// What the key holds instead, [`None`] when it has no mapping.
        /// Pass it as the next attempt's `expected` to retry against it.
        current: Option<Revision>,
    },
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
                skip_verify: options.skip_verify,
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
                    ..
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

    /// Stores one buffer and publishes a mutable key naming it, in one call:
    /// [`Self::put`] followed by a mutable-store publish, fused server-side.
    /// The mapping is written only once the content is stored, so `item.key`
    /// never resolves to content that is not there — [`Self::get_resolved`]
    /// reads it back.
    ///
    /// Publishing is last-writer-wins: two callers publishing the same key
    /// concurrently both succeed, and it ends up naming whichever content was
    /// published second, with no signal to the first publisher that it was
    /// overwritten. Detecting a lost update needs [`Self::put`] followed by a
    /// compare-and-swap instead, which this exists to avoid the round trip
    /// for.
    ///
    /// `backend` determines which mutable stores the key is published to
    /// and which immutable stores the data reaches.
    /// [`Backend::Default`] will always write to both the local and remote stores, as long as
    /// the store was opened with both a local and a remote store.
    /// [`Backend::Local`] and [`Backend::Remote`] will limit the write to a single destination.
    ///
    /// Same durability rules as [`Self::put`]: nothing is flushed by the time
    /// this returns.
    ///
    /// This corresponds to `lore_sys::Lore::lore_storage_put_resolved` with
    /// one item.
    pub fn put_resolved(
        &self,
        backend: Backend,
        item: PutResolvedItem<'_>,
    ) -> Result<PutResolvedOutcome, LoreError> {
        let mut completion: Option<(Address, lore_error_code_t, bool, bool)> = None;

        let result = crate::call::storage_put_resolved(
            self.lore,
            PUT_RESOLVED,
            &self.globals.with_backend(backend),
            StoragePutResolvedArgs {
                handle: self.handle,
                items: &[StoragePutResolvedItem {
                    id: 0,
                    partition: item.repository.to_raw(),
                    key: item.key.to_raw(),
                    context: item.context.to_raw(),
                    data: item.data,
                    remote_write: item.options.remote_write,
                    local_cache: item.options.local_cache,
                    fixed_size_chunk: item.options.fixed_size_chunk,
                }],
            },
            |event| {
                crate::log_event(&event);

                if let Ok(Event::StoragePutItemComplete {
                    address,
                    error_code,
                    stored_local,
                    stored_remote,
                    ..
                }) = event
                {
                    completion = Some((
                        Address::from_raw(address),
                        error_code,
                        stored_local,
                        stored_remote,
                    ));
                }
            },
        );

        put_resolved_outcome(completion, result)
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

    /// The address `key` resolves to, or [`None`] when it has no mapping —
    /// [`Self::get_resolved`] without reading the content. The hash comes
    /// from the mutable store, and `context` completes it into an address,
    /// the same way [`Self::get_resolved`] reads it.
    ///
    /// A key published by [`Self::put_resolved`] is only ever mapped once its
    /// content is stored, so a mapping says the content was there when the
    /// key was published. It does not say the content is there now:
    /// obliterating content leaves the keys naming it in place, locally and
    /// on the server alike. Nor does it hold for a mapping written through
    /// the raw mutable store, which carries no such guarantee.
    ///
    /// Reads only one mutable store: the one `backend` names, or with
    /// [`Backend::Default`] the one the handle's flags select, as
    /// `mutable_load` does — the local one by default and under
    /// `globals.local`/`globals.offline`, the server's under
    /// `globals.remote`. `backend` cannot contradict how the handle was
    /// opened, see [`Backend`]. Unlike the immutable reads there is no
    /// fallthrough from one to the other, so a key published only on the server is
    /// [`None`] on a handle that reads locally. A local mapping is answered
    /// without asking the server, so a key another client has since
    /// republished or retracted can still resolve to the address it had here.
    ///
    /// This corresponds to `lore_sys::Lore::lore_storage_mutable_load` under
    /// `LORE_KEY_TYPE_RESOLVE`.
    pub fn resolve(
        &self,
        backend: Backend,
        repository: RepositoryId,
        key: ResolveKey,
        context: ContextId,
    ) -> Result<Option<Address>, LoreError> {
        let mut completion: Option<(Revision, lore_error_code_t)> = None;

        let result = crate::call::storage_mutable_load(
            self.lore,
            RESOLVE,
            &self.globals.with_backend(backend),
            StorageMutableLoadArgs {
                handle: self.handle,
                items: &[StorageMutableLoadItem {
                    id: 0,
                    partition: repository.to_raw(),
                    key: key.to_raw(),
                    key_type: LORE_KEY_TYPE_RESOLVE,
                }],
            },
            |event| {
                crate::log_event(&event);

                if let Ok(Event::StorageMutableLoadItemComplete {
                    value, error_code, ..
                }) = event
                {
                    completion = Some((Revision::from_raw(value), error_code));
                }
            },
        );

        let Some((hash, code)) = completion else {
            result?;
            return Err(LoreError::MissingEvent {
                command: RESOLVE,
                expected: "storage_mutable_load_item_complete",
            });
        };

        // A miss is this call's answer rather than its failure, as for
        // `metadata_outcome`.
        if ErrorCode::from_raw(code) == Some(ErrorCode::AddressNotFound) {
            return Ok(None);
        }

        LoreError::resolve(RESOLVE, result, Some(code))?;

        // The zero hash is how a retracted key is stored.
        let address = if hash.is_zero() {
            None
        } else {
            Some(Address { hash, context })
        };
        Ok(address)
    }

    /// Points `key` at `value`, but only if it still resolves to `expected`:
    /// the building block for read-modify-write on a key several writers
    /// share, where [`Self::put_resolved`]'s last-writer-wins would lose
    /// updates.
    ///
    /// [`None`] stands for no mapping on both sides: an `expected` of
    /// [`None`] succeeds only when the key is unmapped, so it publishes a key
    /// at most once, and a `value` of [`None`] retracts the key. Only the
    /// hash is swapped, as the mutable store holds no context; the context a
    /// reader completes it with is the one it passes to [`Self::resolve`] or
    /// [`Self::get_resolved`].
    ///
    /// Nothing here checks that `value` names content that exists. Store it
    /// first — with [`Self::put`], under [`PutOptions::remote_write`] when
    /// the swap goes to the server — or a reader can resolve the key and find
    /// nothing behind it. A losing attempt leaves that content stored but
    /// unreferenced.
    ///
    /// Swaps the mutable store `backend` picks, as [`Self::resolve`] reads
    /// it. Only the server's is shared with other clients, so
    /// [`Backend::Remote`] is the one to swap for coordination between them.
    ///
    /// This corresponds to
    /// `lore_sys::Lore::lore_storage_mutable_compare_and_swap` under
    /// `LORE_KEY_TYPE_RESOLVE`, with one item.
    pub fn compare_and_swap_resolved(
        &self,
        backend: Backend,
        repository: RepositoryId,
        key: ResolveKey,
        expected: Option<Revision>,
        value: Option<Revision>,
    ) -> Result<SwapOutcome, LoreError> {
        // The zero hash is how the mutable store spells "no mapping".
        let expected = expected.unwrap_or(Revision::ZERO);
        let value = value.unwrap_or(Revision::ZERO);

        let mut completion: Option<(Revision, lore_error_code_t)> = None;

        let result = crate::call::storage_mutable_compare_and_swap(
            self.lore,
            COMPARE_AND_SWAP,
            &self.globals.with_backend(backend),
            StorageMutableCompareAndSwapArgs {
                handle: self.handle,
                items: &[StorageMutableCompareAndSwapItem {
                    id: 0,
                    partition: repository.to_raw(),
                    key: key.to_raw(),
                    expected: expected.to_raw(),
                    value: value.to_raw(),
                    key_type: LORE_KEY_TYPE_RESOLVE,
                }],
            },
            |event| {
                crate::log_event(&event);

                if let Ok(Event::StorageMutableCompareAndSwapItemComplete {
                    previous,
                    error_code,
                    ..
                }) = event
                {
                    completion = Some((Revision::from_raw(previous), error_code));
                }
            },
        );

        let Some((previous, code)) = completion else {
            result?;
            return Err(LoreError::MissingEvent {
                command: COMPARE_AND_SWAP,
                expected: "storage_mutable_compare_and_swap_item_complete",
            });
        };

        LoreError::resolve(COMPARE_AND_SWAP, result, Some(code))?;

        let outcome = if previous == expected {
            SwapOutcome::Swapped
        } else {
            SwapOutcome::Conflict {
                current: Some(previous).filter(|previous| !previous.is_zero()),
            }
        };
        Ok(outcome)
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
        self.get_range(repository, address, ..)
    }

    /// Reads the bytes of one address that fall in `range`, counted from the
    /// start of the decompressed content. Lore fetches and decompresses only
    /// the fragments the range touches, so a small slice of a large item does
    /// not cost the whole item.
    ///
    /// A range reaching past the end of the content is clamped to it, so the
    /// buffer can be shorter than `range`. A range starting past the end is
    /// [`LoreError::Failed`] with
    /// [`ErrorCode::InvalidArguments`](crate::ErrorCode::InvalidArguments).
    /// An empty range reads as an empty buffer without asking Lore, so it
    /// says nothing about whether `address` exists. Otherwise the same rules
    /// as [`Self::get`].
    ///
    /// This corresponds to `lore_sys::Lore::lore_storage_get` with `offset`
    /// and `length` set.
    pub fn get_range(
        &self,
        repository: RepositoryId,
        address: Address,
        range: impl RangeBounds<u64>,
    ) -> Result<Vec<u8>, LoreError> {
        const COMMAND: &str = "storage::get";

        let start = match range.start_bound() {
            Bound::Included(&start) => start,
            Bound::Excluded(&start) => start.saturating_add(1),
            Bound::Unbounded => 0,
        };
        // Lore reads a zero `length` as "to the end", so a bounded range that
        // is empty has to stop here rather than be passed on.
        let length = match range.end_bound() {
            Bound::Included(&end) => Some(end.saturating_add(1).saturating_sub(start)),
            Bound::Excluded(&end) => Some(end.saturating_sub(start)),
            Bound::Unbounded => None,
        };
        if length == Some(0) {
            return Ok(Vec::new());
        }

        let mut data: Option<Vec<u8>> = None;
        // Fragments are not promised in order, so `data` is written by offset
        // and cannot report progress by its own length. `covered` sums what
        // was written and `end` tracks how far it reached: a response that
        // skipped a range fails one or the other even when its ranges add up.
        // Both are relative to `start`, as is `data`.
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
                items: &mut [self.item(repository, address, start, length.unwrap_or(0), false)],
            },
            |event| {
                crate::log_event(&event);

                match event {
                    Ok(Event::StorageGetHeader { size_content, .. }) => {
                        // The header reports the whole content; the range is
                        // clamped to it. A start past the end fails the item,
                        // so the saturation only keeps this from panicking.
                        let range_end = match length {
                            Some(length) => start.saturating_add(length).min(size_content),
                            None => size_content,
                        };
                        data = Some(vec![0u8; range_end.saturating_sub(start) as usize]);
                    }
                    Ok(Event::StorageGetData { offset, bytes, .. }) => {
                        let length = bytes.len() as u64;
                        let Some(buffer) = data.as_mut() else {
                            // Data before the header is not a shape Lore
                            // produces; it fails the size check below.
                            overflow = Some(offset.saturating_add(length));
                            return;
                        };
                        // `offset` is within the whole content, not the range.
                        let Some(offset) = offset.checked_sub(start) else {
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

    /// Resolves a mutable key and reads the content it names, in one call:
    /// [`Self::get`] without first having to learn the address `key` resolves
    /// to. `key` is read under `LORE_KEY_TYPE_RESOLVE`, which is what
    /// [`Self::put_resolved`] publishes.
    ///
    /// The address `key` resolved to comes back alongside the bytes — this is
    /// the only way to learn the key-to-hash mapping, since Lore does not
    /// otherwise expose it. Same `repository`, buffering and size-check rules
    /// as [`Self::get`]. A key with no mapping, or one naming absent content,
    /// is [`LoreError::Failed`] with
    /// [`ErrorCode::AddressNotFound`](crate::ErrorCode::AddressNotFound),
    /// same as [`Self::get`].
    ///
    /// `backend` determines which mutable store is used for the resolution of the key.
    /// [`Backend::Default`] will follow the same precedence the store has been opened with:
    /// local first, followed by remote, unless the store has been opened only
    /// for local or remote operations.
    /// [`Backend::Local`] and [`Backend::Remote`] will limit it to a single mutable store.
    ///
    /// This corresponds to `lore_sys::Lore::lore_storage_get_resolved`.
    pub fn get_resolved(
        &self,
        backend: Backend,
        repository: RepositoryId,
        key: ResolveKey,
        context: ContextId,
    ) -> Result<Resolved<Vec<u8>>, LoreError> {
        let mut reassembly = Reassembly::default();

        let result = crate::call::storage_get_resolved(
            self.lore,
            GET_RESOLVED,
            &self.globals.with_backend(backend),
            StorageGetResolvedArgs {
                handle: self.handle,
                items: &[self.resolved_item(
                    0,
                    GetResolvedItem {
                        repository,
                        key,
                        context,
                    },
                )],
            },
            |event| {
                crate::log_event(&event);

                if let Ok(event) = event {
                    reassembly.record(event);
                }
            },
        );
        LoreError::resolve(
            GET_RESOLVED,
            result,
            reassembly.completion.map(|(_, code)| code),
        )?;

        reassembly.finish()
    }

    /// Resolves several mutable keys and reads the content each names, in one
    /// call: [`Self::get_resolved`] for a batch, which Lore resolves and reads
    /// concurrently in one FFI round trip rather than one per key. The results
    /// come back in the order the items were given.
    ///
    /// **One failed item does not fail the call**, as for [`Self::put_many`]:
    /// a key with no mapping is [`LoreError::Failed`] with
    /// [`ErrorCode::AddressNotFound`] in its own slot, and the rest still
    /// answer. The outer error is left for what goes wrong with the call as a
    /// whole.
    ///
    /// Every item's buffer is held until the call returns, so the whole batch
    /// is in memory at once. Same rules as [`Self::get_resolved`] otherwise,
    /// with `backend` applying to every item.
    ///
    /// This corresponds to `lore_sys::Lore::lore_storage_get_resolved`.
    #[allow(clippy::type_complexity)]
    pub fn get_resolved_many(
        &self,
        backend: Backend,
        items: &[GetResolvedItem],
    ) -> Result<Vec<Result<Resolved<Vec<u8>>, LoreError>>, LoreError> {
        let raw_items = items
            .iter()
            .enumerate()
            // The position is the id, as for `Self::put_many`.
            .map(|(index, item)| self.resolved_item(index as u64, *item))
            .collect::<Vec<_>>();

        let mut reassemblies: Vec<Reassembly> = Vec::new();
        reassemblies.resize_with(items.len(), Reassembly::default);

        let result = crate::call::storage_get_resolved(
            self.lore,
            GET_RESOLVED,
            &self.globals.with_backend(backend),
            StorageGetResolvedArgs {
                handle: self.handle,
                items: &raw_items,
            },
            |event| {
                crate::log_event(&event);

                let Ok(event) = event else {
                    return;
                };
                let id = match event {
                    Event::StorageGetHeader { id, .. }
                    | Event::StorageGetData { id, .. }
                    | Event::StorageGetItemComplete { id, .. } => id,
                    _ => return,
                };
                // An id from outside the batch leaves the item it was meant
                // for unreported, which fails the batch below.
                if let Some(reassembly) = usize::try_from(id)
                    .ok()
                    .and_then(|index| reassemblies.get_mut(index))
                {
                    reassembly.record(event);
                }
            },
        );

        get_resolved_outcomes(reassemblies, result)
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
                items: &mut [self.item(repository, address, 0, 0, true)],
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

    fn item(
        &self,
        repository: RepositoryId,
        address: Address,
        offset: u64,
        length: u64,
        streaming: bool,
    ) -> StorageGetItem<'static> {
        StorageGetItem {
            id: 0,
            partition: repository.to_raw(),
            address: address.to_raw(),
            // A zero `length` reads to the end of the content.
            offset,
            length,
            streaming,
            local_cache: self.local_cache,
            // `Store::get` and `Store::read_to` reassemble through the
            // event stream, not a caller-supplied buffer.
            data_out: None,
        }
    }

    fn resolved_item(&self, id: u64, item: GetResolvedItem) -> StorageGetResolvedItem {
        StorageGetResolvedItem {
            id,
            partition: item.repository.to_raw(),
            key: item.key.to_raw(),
            context: item.context.to_raw(),
            streaming: false,
            local_cache: self.local_cache,
        }
    }
}

/// One resolved read's events, reassembled into its buffer. Fragments are not
/// promised in order, so the buffer is written by offset; see
/// [`Store::get_range`] for what `covered` and `end` track.
#[derive(Default)]
struct Reassembly {
    data: Option<Vec<u8>>,
    covered: u64,
    end: u64,
    overflow: Option<u64>,
    completion: Option<(Address, lore_error_code_t)>,
}

impl Reassembly {
    /// Takes one event of this item's sequence; events of other kinds are
    /// ignored.
    fn record(&mut self, event: Event<'_>) {
        match event {
            Event::StorageGetHeader { size_content, .. } => {
                self.data = Some(vec![0u8; size_content as usize]);
            }
            Event::StorageGetData { offset, bytes, .. } => {
                let length = bytes.len() as u64;
                let Some(buffer) = self.data.as_mut() else {
                    self.overflow = Some(offset.saturating_add(length));
                    return;
                };
                let size = buffer.len() as u64;
                let Some(range_end) = offset
                    .checked_add(length)
                    .filter(|range_end| *range_end <= size)
                else {
                    self.overflow = Some(offset.saturating_add(length));
                    return;
                };

                buffer[offset as usize..range_end as usize].copy_from_slice(bytes);
                self.covered += length;
                self.end = self.end.max(range_end);
            }
            Event::StorageGetItemComplete {
                address,
                error_code,
                ..
            } => {
                self.completion = Some((Address::from_raw(address), error_code));
            }
            _ => {}
        }
    }

    /// Checks the delivery covered exactly what the header announced. Only
    /// meaningful once the item's own code has been checked.
    fn finish(self) -> Result<Resolved<Vec<u8>>, LoreError> {
        let data = self.data.ok_or(LoreError::MissingEvent {
            command: GET_RESOLVED,
            expected: "storage_get_header",
        })?;
        let size = data.len() as u64;

        if let Some(over) = self.overflow {
            return Err(LoreError::SizeMismatch {
                command: GET_RESOLVED,
                expected: size,
                covered: over,
            });
        }
        if self.covered != size || self.end != size {
            return Err(LoreError::SizeMismatch {
                command: GET_RESOLVED,
                expected: size,
                covered: self.covered.min(self.end),
            });
        }

        let (address, _) = self.completion.ok_or(LoreError::MissingEvent {
            command: GET_RESOLVED,
            expected: "storage_get_item_complete",
        })?;

        Ok(Resolved { address, data })
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

/// Folds what a batch of resolved reads reported into one result per item, in
/// the order the items were given, by the same rules as [`put_outcomes`].
#[allow(clippy::type_complexity)]
fn get_resolved_outcomes(
    reassemblies: Vec<Reassembly>,
    result: Result<(), LoreError>,
) -> Result<Vec<Result<Resolved<Vec<u8>>, LoreError>>, LoreError> {
    if reassemblies.iter().any(|item| item.completion.is_none()) {
        result?;
        return Err(LoreError::MissingEvent {
            command: GET_RESOLVED,
            expected: "storage_get_item_complete",
        });
    }

    let outcomes: Vec<Result<Resolved<Vec<u8>>, LoreError>> = reassemblies
        .into_iter()
        .map(|item| {
            let code = item.completion.map(|(_, code)| code);
            match code.and_then(ErrorCode::from_raw) {
                None => item.finish(),
                // As for a put, the call's messages only count its failed
                // items, so an item's code is all there is to report.
                Some(code) => Err(LoreError::Failed {
                    command: GET_RESOLVED,
                    code,
                    messages: Vec::new(),
                }),
            }
        })
        .collect();

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

/// Turns what a resolved put reported into its outcome, or fails the call
/// when nothing was said about the item.
fn put_resolved_outcome(
    completion: Option<(Address, lore_error_code_t, bool, bool)>,
    result: Result<(), LoreError>,
) -> Result<PutResolvedOutcome, LoreError> {
    let Some((address, code, stored_local, stored_remote)) = completion else {
        result?;
        return Err(LoreError::MissingEvent {
            command: PUT_RESOLVED,
            expected: "storage_put_item_complete",
        });
    };

    LoreError::resolve(PUT_RESOLVED, result, Some(code))?;

    Ok(PutResolvedOutcome {
        address,
        stored_local,
        stored_remote,
    })
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

#[cfg(test)]
mod tests {
    use lore_sys::{LORE_ERROR_CODE_ADDRESS_NOT_FOUND, LORE_ERROR_CODE_NONE};

    use super::*;

    fn address(tag: u8) -> Address {
        Address {
            hash: Revision::new([tag; 32]),
            context: ContextId::default(),
        }
    }

    fn header(id: u64, size_content: u64) -> Event<'static> {
        Event::StorageGetHeader {
            id,
            address: Address::default().to_raw(),
            size_content,
        }
    }

    fn data(id: u64, offset: u64, bytes: &'static [u8]) -> Event<'static> {
        Event::StorageGetData { id, offset, bytes }
    }

    fn complete(id: u64, address: Address, error_code: lore_error_code_t) -> Event<'static> {
        Event::StorageGetItemComplete {
            id,
            address: address.to_raw(),
            error_code,
        }
    }

    /// Feeds `events` to a batch of `items` the way `get_resolved_many`
    /// routes them, then folds the batch.
    #[allow(clippy::type_complexity)]
    fn fold(
        items: usize,
        events: Vec<Event<'static>>,
        result: Result<(), LoreError>,
    ) -> Result<Vec<Result<Resolved<Vec<u8>>, LoreError>>, LoreError> {
        let mut reassemblies: Vec<Reassembly> = Vec::new();
        reassemblies.resize_with(items, Reassembly::default);
        for event in events {
            let id = match event {
                Event::StorageGetHeader { id, .. }
                | Event::StorageGetData { id, .. }
                | Event::StorageGetItemComplete { id, .. } => id,
                _ => unreachable!(),
            };
            if let Some(reassembly) = reassemblies.get_mut(id as usize) {
                reassembly.record(event);
            }
        }
        get_resolved_outcomes(reassemblies, result)
    }

    fn call_failed() -> Result<(), LoreError> {
        Err(LoreError::Call {
            command: GET_RESOLVED,
            status: 1,
            messages: vec!["1/2 items failed".to_owned()],
        })
    }

    #[test]
    fn interleaved_items_land_in_their_own_slots() {
        let outcomes = fold(
            2,
            vec![
                header(1, 3),
                header(0, 4),
                data(0, 2, b"cd"),
                data(1, 0, b"xyz"),
                data(0, 0, b"ab"),
                complete(1, address(2), LORE_ERROR_CODE_NONE),
                complete(0, address(1), LORE_ERROR_CODE_NONE),
            ],
            Ok(()),
        )
        .unwrap();

        let outcomes: Vec<_> = outcomes.into_iter().map(Result::unwrap).collect();
        assert_eq!(
            outcomes,
            vec![
                Resolved {
                    address: address(1),
                    data: b"abcd".to_vec(),
                },
                Resolved {
                    address: address(2),
                    data: b"xyz".to_vec(),
                },
            ]
        );
    }

    #[test]
    fn a_missing_key_fails_only_its_own_item() {
        let outcomes = fold(
            2,
            vec![
                complete(0, Address::default(), LORE_ERROR_CODE_ADDRESS_NOT_FOUND),
                header(1, 2),
                data(1, 0, b"ok"),
                complete(1, address(1), LORE_ERROR_CODE_NONE),
            ],
            call_failed(),
        )
        .unwrap();

        assert!(outcomes[0].as_ref().unwrap_err().is_not_found());
        assert_eq!(outcomes[1].as_ref().unwrap().data, b"ok");
    }

    #[test]
    fn a_short_delivery_fails_only_its_own_item() {
        let outcomes = fold(
            2,
            vec![
                header(0, 4),
                data(0, 0, b"ab"),
                complete(0, address(1), LORE_ERROR_CODE_NONE),
                header(1, 0),
                complete(1, address(2), LORE_ERROR_CODE_NONE),
            ],
            Ok(()),
        )
        .unwrap();

        assert!(matches!(
            outcomes[0],
            Err(LoreError::SizeMismatch {
                expected: 4,
                covered: 2,
                ..
            })
        ));
        assert_eq!(outcomes[1].as_ref().unwrap().data, b"");
    }

    #[test]
    fn an_unreported_item_fails_the_batch() {
        let error = fold(
            2,
            vec![header(0, 0), complete(0, address(1), LORE_ERROR_CODE_NONE)],
            Ok(()),
        )
        .unwrap_err();

        assert!(matches!(error, LoreError::MissingEvent { .. }));
    }

    #[test]
    fn a_call_failure_no_item_accounts_for_fails_the_batch() {
        let error = fold(
            1,
            vec![header(0, 0), complete(0, address(1), LORE_ERROR_CODE_NONE)],
            call_failed(),
        )
        .unwrap_err();

        assert!(matches!(error, LoreError::Call { .. }));
    }

    #[test]
    fn an_empty_batch_answers_with_nothing() {
        assert!(fold(0, Vec::new(), Ok(())).unwrap().is_empty());
    }
}
