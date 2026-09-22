//! Rust bindings for the C API of [Lore], Epic Games' version control system.
//!
//! Three layers, each usable on its own:
//!
//! 1. [`lore_sys`]: the generated bindings, 263 function pointers loaded from
//!    the shared library. Every struct and constant of `lore.h`.
//! 2. [`call`]: one safe function per Lore command, still shaped like the C
//!    API — arguments in, [`Event`]s out through a callback — plus
//!    [`call_with_callback`] for the commands without a function yet.
//! 3. Handle types that return data: [`Lore`] for the process-level
//!    operations, including the [login](Lore::auth_login_with_token) every
//!    other call authenticates with, [`Repository`] for the commands that run
//!    against a local repository instance, and [`Store`] with
//!    [`RevisionTree`] for Lore's storage API, which is independent of any
//!    instance.
//!
//! Every call takes a [`GlobalArgs`], Lore's `lore_global_args_t`, whose
//! defaults are Lore's own.
//!
//! [Lore]: https://github.com/EpicGames/lore

pub use lore_sys;
pub use lore_sys::libloading;

pub mod call;
pub use call::{
    call_with_callback, AuthLoginWithTokenArgs, BranchInfoArgs, FileInfoArgs, RepositoryInfoArgs,
    RepositoryStatusArgs, RevisionInfoArgs, RevisionTreeResolvePathArgs, StorageGetArgs,
    StorageGetItem, StorageGetMetadataArgs, StorageGetMetadataItem, StorageOpenArgs,
    StoragePutArgs, StoragePutItem,
};

mod string;
pub use string::{LoreStringArrayExt, LoreStringExt};

mod globals;
pub use globals::GlobalArgs;

mod event;
pub use event::{log_event, Event};

mod error;
pub use error::{ErrorCode, LoreError};

mod types;
pub use types::{Address, BranchId, ContextId, NodeId, NodeKind, RepositoryId, Revision};

mod library;
pub use library::{load, LogConfig, Lore};

mod auth;
pub use auth::UserInfo;

mod repository;
pub use repository::{BranchInfo, Repository, RepositoryInfo, RevisionInfo};

mod store;
pub use store::{
    CacheTargets, FragmentInfo, PutItem, PutOptions, Store, StoreLocation, StoreOptions,
};

mod revision_tree;
pub use revision_tree::{Child, Node, RevisionTree, TreeInfo};
