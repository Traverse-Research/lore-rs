use lore_sys::lore_global_args_t;

use crate::string::raw_str;

/// Options every Lore call takes: the equivalent of `lore_global_args_t` with
/// Rust types, owned so that a handle can keep the ones it was created with.
///
/// [`Default`] is the all-zero configuration, which is what Lore itself and
/// every official SDK default to. Set only what you need:
///
/// ```
/// # use lore_rs::GlobalArgs;
/// let globals = GlobalArgs {
///     repository_path: "/path/to/repository".into(),
///     offline: true,
///     ..Default::default()
/// };
/// ```
///
/// Not every field means something to every call. Lore has four families of
/// verbs, and the docs on each field say which of them read it:
///
/// - **Repository verbs** (`repository_*`, `branch_*`, `file_*`, `revision_*`,
///   and the rest of the working-tree commands) run against the local
///   repository instance that `repository_path` names, and fail when there is
///   no `.lore` directory there. See [`Repository`](crate::Repository).
/// - **Storage verbs** (`storage_*`) ignore `repository_path`; a store takes
///   its own location when it is opened. `offline`, `local` and `remote` are
///   *bound into the handle* at open time, and a later call can only tighten
///   them. See [`Store`](crate::Store).
/// - **Revision-tree verbs** (`revision_tree_*`) read nothing here beyond
///   `identity` and `correlation_id`; the loaded tree already knows its store,
///   repository and revision.
/// - **Auth verbs** (`auth_*`) read `repository_path` only to resolve a server
///   the arguments did not name, and write a token store that is per OS user
///   rather than part of any repository. See
///   [`Lore::auth_login_with_token`](crate::Lore::auth_login_with_token).
///
/// Empty strings reach Lore as null pointers, which is how it spells "unset".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobalArgs {
    /// The local repository instance: the directory holding `.lore`. Read by
    /// the repository verbs only.
    pub repository_path: String,
    /// Echoed back in Lore's logs, for correlating one caller's calls.
    pub correlation_id: String,
    /// Identity to authenticate with, as
    /// [`UserInfo::id`](crate::UserInfo::id) reports it. Unset means the
    /// identity Lore resolves itself, from the repository config or the token
    /// store a [login](crate::Lore::auth_login_with_token) writes. Storage
    /// handles bind it at open.
    pub identity: String,
    /// Force the operation if possible.
    pub force: bool,
    /// Run the operation without connecting to the server.
    pub offline: bool,
    /// Use only local data. For a store this means a cold store has nothing
    /// to answer with.
    pub local: bool,
    /// Use only remote data, bypassing the local store. A store opened with
    /// this set needs a remote URL.
    pub remote: bool,
    /// Report what would have been changed without touching the file system.
    pub dry_run: bool,
    /// Avoid recording last access timestamps in the data stores.
    pub no_atime: bool,
    /// Allow matching the nearest revision when no perfect match exists.
    pub search_nearest: bool,
    /// Prevent the automatic incremental GC for this operation. At
    /// [`Store::open`](crate::Store::open) this leaves the evictor and
    /// compactor unspawned whatever the cache targets say.
    pub no_gc: bool,
    /// Use in-memory stores instead of the file-backed ones. Repository verbs
    /// only; a store is opened in memory through
    /// [`StoreLocation::InMemory`](crate::StoreLocation).
    pub in_memory: bool,
    /// Keep store references alive after a repository call completes, so
    /// consecutive calls skip repeated open/close cycles. The stores stay open
    /// for `store_keep_alive_seconds` after each call.
    pub store_keep_alive: bool,
    /// Force syncing data to the storage media during store flush.
    pub sync_data: bool,
    /// Cache fragment payloads fetched from a remote in the local store.
    /// Repository verbs only; a store caches per read through
    /// [`StoreOptions::local_cache`](crate::StoreOptions::local_cache).
    pub cache: bool,
    // Zero means Lore's own default for each of the numeric fields below.
    /// Maximum number of parallel connections for bulk data transfer.
    pub max_connections: u32,
    /// Search limit when iterating revisions.
    pub search_limit: u32,
    /// Maximum number of files processed in parallel.
    pub file_count_limit: u64,
    /// Maximum total size of all files processed in parallel.
    pub file_size_limit: u64,
    /// Maximum number of parallel compression tasks.
    pub compress_task_limit: u64,
    /// How long `store_keep_alive` keeps stores open; zero is Lore's default
    /// of ten seconds.
    pub store_keep_alive_seconds: u64,
}

impl GlobalArgs {
    /// The raw struct to hand to Lore. Borrows `self` through raw pointers,
    /// so `self` must outlive every use of the result.
    pub(crate) fn to_raw(&self) -> lore_global_args_t {
        lore_global_args_t {
            repository_path: raw_str(&self.repository_path),
            correlation_id: raw_str(&self.correlation_id),
            identity: raw_str(&self.identity),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LoreStringExt;

    #[test]
    fn global_args_conversion() {
        // Bound to a variable: the raw struct borrows the strings through
        // pointers without a lifetime, so the `GlobalArgs` must outlive it.
        let globals = GlobalArgs {
            repository_path: "/repo".into(),
            offline: true,
            ..Default::default()
        };
        let raw = globals.to_raw();
        assert_eq!(unsafe { raw.repository_path.try_to_str() }, Ok("/repo"));
        assert!(raw.correlation_id.string.is_null(), "unset is null");
        assert_eq!(raw.offline, 1);
        assert_eq!(raw.force, 0);
        assert_eq!(raw.cache, 0, "the default is Lore's all-zero configuration");
        assert_eq!(raw.store_keep_alive, 0);
    }
}
