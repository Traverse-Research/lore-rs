use std::io::Write;

use lore_sys::{lore_error_code_t, lore_storage_close_args_t, lore_store_t};

use crate::{
    Address, Event, GlobalArgs, Lore, LoreError, RepositoryId, Revision, RevisionTree,
    StorageGetArgs, StorageGetItem, StorageOpenArgs,
};

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

/// An open content-addressed store, closed on drop.
///
/// The handle is repository-agnostic: it is bound to a location and
/// optionally a server, and every read names the repository it is about,
/// because an [`Address`] alone does not. See [`Self::get`].
///
/// `Send + Sync`; Lore allows concurrent operations on one handle.
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

impl Drop for Store {
    /// Closes the handle. Lore spawns the store's flush rather than waiting
    /// for it; see [`Lore::shutdown`] for what that means at process exit. A
    /// failure to close is logged, since nothing can act on it here.
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
