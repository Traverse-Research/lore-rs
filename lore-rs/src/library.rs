use lore_sys::{lore_log_config_t, lore_log_level_t};

use crate::string::raw_str;
use crate::LoreError;

/// The loaded Lore library. The raw functions are reachable through
/// [`Deref`](std::ops::Deref); the process-level operations Lore has —
/// version, thread limit, logging, shutdown — have safe methods here, and the
/// handle types ([`Repository`](crate::Repository), [`Store`](crate::Store),
/// [`RevisionTree`](crate::RevisionTree)) take a `&'static Lore` from
/// [`load`].
///
/// # Never dropped
///
/// [`load`] parks the one `Lore` in a `static`, and Rust runs no destructor
/// for those. Dropping one would unload the shared library, since
/// `lore_sys::Lore` owns the `libloading::Library`, and that is never safe:
///
/// - Lore's worker threads live in the image. `lore_shutdown` asks its tokio
///   runtime to stop with a ten-second timeout and returns whether or not it
///   did, and its rayon compute pool is never stopped at all; Lore relies on
///   process exit for those threads. Unmapping the code they run is a crash.
/// - Shutdown is scoped to the process, not to a value: it closes every
///   storage handle the process holds, drops every connection and finalizes
///   rpmalloc, Lore's global allocator. Nothing can be called after it.
///
/// So there is exactly one `Lore` per process, it lives as long as the
/// process, and [`shutdown`](Self::shutdown) is a deliberate, final act
/// rather than something a value's drop could do.
///
/// # Shutdown, or not
///
/// Not calling [`shutdown`](Self::shutdown) is fine for a reader: every
/// handle closes on drop and there is nothing to lose. It is not entirely
/// free, though. Closing a store only *spawns* its flush, and shutdown is
/// what waits for spawned work, so a process that exits right after its last
/// drop can lose the fragments a read cached locally. A writer, or a reader
/// that counts on its cache being warm next time, calls `shutdown` at the end
/// of `main` once every handle is gone.
pub struct Lore(pub(crate) lore_sys::Lore);

impl std::ops::Deref for Lore {
    type Target = lore_sys::Lore;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// The raw struct is 263 function pointers; its name is the useful part.
impl std::fmt::Debug for Lore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Lore")
    }
}

impl Lore {
    /// The version the loaded library reports for itself, such as `0.8.5`.
    /// Compare it with the release the bindings were generated for when a
    /// mismatch would be surprising; the bindings themselves cannot tell.
    pub fn version(&self) -> &str {
        // SAFETY: `lore_version` returns a pointer to a NUL-terminated string
        // the library owns and never frees, and the library is never unloaded.
        let version = unsafe { self.0.lore_version() };
        if version.is_null() {
            return "";
        }
        unsafe { std::ffi::CStr::from_ptr(version) }
            .to_str()
            .unwrap_or("")
    }

    /// Caps the total number of threads Lore sizes its pools for; zero means
    /// no limit, the default. Must run before the first operation, while the
    /// runtime is still unconstructed. Returns `false` when it was too late or
    /// a limit was already set, in which case nothing changes.
    ///
    /// The `LORE_MAX_THREADS` environment variable overrides this when set
    /// above zero.
    pub fn set_thread_limit(&self, count: usize) -> bool {
        // SAFETY: a plain call into the loaded library with a scalar argument.
        unsafe { self.0.lore_set_thread_limit(count) == 0 }
    }

    /// Configures Lore's own logging: the file it writes and the minimum
    /// level. Independent of the [`Event::Log`](crate::Event::Log) events a
    /// call's callback receives, which [`log_event`](crate::log_event) can
    /// forward to the `log` crate.
    pub fn configure_log(&self, config: &LogConfig) -> Result<(), LoreError> {
        let raw = config.to_raw();
        // SAFETY: the raw struct borrows `config`, which lives across the
        // call, and Lore copies what it keeps.
        let status = unsafe { self.0.lore_log_configure(&raw) };
        if status == 0 {
            Ok(())
        } else {
            Err(LoreError::Call {
                command: "log::configure",
                status,
                messages: Vec::new(),
            })
        }
    }

    /// Shuts the library down: closes every storage handle the process still
    /// holds, drops every connection, waits up to ten seconds for Lore's
    /// worker threads, then finalizes Lore's allocator. Call it once, at the
    /// end of the process, or not at all — see the type docs for when it
    /// matters.
    ///
    /// # Safety
    ///
    /// No Lore call may run concurrently with this one or after it, which
    /// includes the close a [`Store`](crate::Store) or
    /// [`RevisionTree`](crate::RevisionTree) runs on drop: every handle must
    /// already be gone. Lore's allocator is finalized here, so a later call
    /// is undefined behaviour rather than an error.
    pub unsafe fn shutdown(&self) -> Result<(), LoreError> {
        // SAFETY: the caller's contract above is this function's contract.
        let status = unsafe { self.0.lore_shutdown() };
        if status == 0 {
            Ok(())
        } else {
            Err(LoreError::Call {
                command: "shutdown",
                status,
                messages: Vec::new(),
            })
        }
    }
}

/// Lore's own logging, the equivalent of `lore_log_config_t`. [`Default`] is
/// the zero configuration: no file, and a level of `LORE_LOG_LEVEL_NONE`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogConfig {
    /// Write a log file.
    pub file: bool,
    /// Roll the log file over daily.
    pub file_rolling: bool,
    /// Directory the log file is written to.
    pub file_path: String,
    /// Prefix for log file names.
    pub file_prefix: String,
    /// Minimum level to log, one of `lore_sys::LORE_LOG_LEVEL_*`.
    pub level: lore_log_level_t,
    /// Category bit flags: local, remote, transport.
    pub categories: u32,
    /// Maximum size of one log file; zero is Lore's default.
    pub file_max_size: u32,
    /// Maximum number of log files kept; zero is Lore's default.
    pub file_max_count: u32,
}

impl LogConfig {
    /// The raw struct to hand to Lore. Borrows `self` through raw pointers,
    /// so `self` must outlive every use of the result.
    fn to_raw(&self) -> lore_log_config_t {
        lore_log_config_t {
            file: u8::from(self.file),
            file_rolling: u8::from(self.file_rolling),
            file_path: raw_str(&self.file_path),
            file_prefix: raw_str(&self.file_prefix),
            level: self.level,
            categories: self.categories,
            file_max_size: self.file_max_size,
            file_max_count: self.file_max_count,
        }
    }
}

static LORE: std::sync::OnceLock<Lore> = std::sync::OnceLock::new();

/// Loads the Lore library, once per process.
///
/// Call it as often as you like: the first call loads, the rest hand back the
/// same library and ignore `path`. The library is never unloaded — see [`Lore`]
/// — so there is no handle to keep, share or hand back.
///
/// # Safety
///
/// `path` must refer to a Lore dynamic library matching the version these
/// bindings were generated for (`LORE_VERSION` in `lore-bin`). Loading it runs
/// the library's initialization code, and mismatched function signatures are
/// undefined behaviour on any later call.
pub unsafe fn load<P: AsRef<std::ffi::OsStr>>(path: P) -> Result<&'static Lore, LoreError> {
    if let Some(lore) = LORE.get() {
        return Ok(lore);
    }

    // SAFETY: the caller's contract above is this function's contract.
    let lore = Lore(unsafe { lore_sys::Lore::new(path) }?);
    log::debug!(target: "lore", "loaded Lore {}", lore.version());

    // A concurrent first call may have won; either library is the same one.
    Ok(LORE.get_or_init(|| lore))
}
