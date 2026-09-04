//! Identifiers, as distinct types.
//!
//! Lore's C identifiers are aliases, not types: `lore_repository_id_t` *is*
//! `lore_partition_t`, `lore_branch_id_t` *is* `lore_context_t`, and a node id
//! is a bare `u32`. The newtypes here make confusing them a compile error.
//!
//! [`Display`](std::fmt::Display) is lowercase hex, matching what Lore's CLI
//! and logs print, so a revision in an error can be pasted into a `lore`
//! command.

use lore_sys::{lore_address_t, lore_context_t, lore_hash_t, lore_partition_t};

/// Lowercase hex, truncated to the formatter's precision when it has one, so
/// `{id:.8}` abbreviates the way Lore itself does.
fn write_hex(f: &mut std::fmt::Formatter<'_>, bytes: &[u8]) -> std::fmt::Result {
    let characters = f.precision().unwrap_or(bytes.len() * 2);

    for (index, byte) in bytes.iter().enumerate() {
        match characters.saturating_sub(index * 2) {
            0 => break,
            1 => write!(f, "{:x}", byte >> 4)?,
            _ => write!(f, "{byte:02x}")?,
        }
    }

    Ok(())
}

macro_rules! identifier {
    ($(#[$meta:meta])* $name:ident, $raw:ident, $width:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name([u8; $width]);

        impl $name {
            /// The all-zero value, which Lore uses for "none".
            pub const ZERO: Self = Self([0; $width]);

            pub fn is_zero(&self) -> bool {
                self.0 == [0; $width]
            }

            pub const fn as_bytes(&self) -> &[u8; $width] {
                &self.0
            }

            /// The first eight hex characters, which is what Lore abbreviates
            /// to. For logs — an error should carry the full value, since that
            /// is what pastes into a `lore` command.
            pub fn short(&self) -> String {
                format!("{self:.8}")
            }

            pub(crate) const fn from_raw(raw: $raw) -> Self {
                Self(raw.data)
            }

            // Unused for the identifiers Lore only ever hands out, such as
            // a branch id, which the macro cannot know per instantiation.
            #[allow(dead_code)]
            pub(crate) const fn to_raw(self) -> $raw {
                $raw { data: self.0 }
            }
        }

        impl From<[u8; $width]> for $name {
            fn from(bytes: [u8; $width]) -> Self {
                Self(bytes)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write_hex(f, &self.0)
            }
        }

        impl std::fmt::LowerHex for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write_hex(f, &self.0)
            }
        }

        // Hex rather than the derived form, which prints decimal bytes.
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}(", stringify!($name))?;
                write_hex(f, &self.0)?;
                write!(f, ")")
            }
        }
    };
}

identifier!(
    /// A revision, by the hash of its content. [`Self::ZERO`] is Lore's "no
    /// revision" — what a branch with nothing on it reports as its tip.
    Revision,
    lore_hash_t,
    32
);

identifier!(
    /// A repository, which is also the storage partition every read names.
    RepositoryId,
    lore_partition_t,
    16
);

identifier!(
    /// A branch.
    BranchId,
    lore_context_t,
    16
);

identifier!(
    /// The context half of an [`Address`]: which file the bytes belong to,
    /// distinguishing two files that hold identical content.
    ContextId,
    lore_context_t,
    16
);

/// Where content lives — the hash of its bytes plus the file it belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Address {
    pub hash: Revision,
    pub context: ContextId,
}

impl Address {
    pub(crate) const fn from_raw(raw: lore_address_t) -> Self {
        Self {
            hash: Revision::from_raw(raw.hash),
            context: ContextId::from_raw(raw.context),
        }
    }

    pub(crate) const fn to_raw(self) -> lore_address_t {
        lore_address_t {
            hash: self.hash.to_raw(),
            context: self.context.to_raw(),
        }
    }
}

/// `<hash>-<context>`, the form Lore itself prints.
impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.hash, self.context)
    }
}

impl std::fmt::Debug for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Address({self})")
    }
}

/// A node of a loaded revision tree, meaningful only to that tree.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(pub(crate) lore_sys::lore_node_id_t);

impl NodeId {
    /// The root directory of every tree. Also what an empty path resolves to.
    pub const ROOT: Self = Self(0);
}

/// What a revision-tree node holds.
///
/// [`Self::Other`] rather than `#[non_exhaustive]`: Lore's node type is a
/// `c_uint`, so an unknown value is a runtime possibility and the caller
/// should be able to see which one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum NodeKind {
    Directory,
    File,
    /// A sub-repository link.
    Link,
    Other(u32),
}

impl NodeKind {
    pub(crate) fn from_raw(kind: u32) -> Self {
        match kind {
            lore_sys::LORE_NODE_TYPE_DIRECTORY => Self::Directory,
            lore_sys::LORE_NODE_TYPE_FILE => Self::File,
            lore_sys::LORE_NODE_TYPE_LINK => Self::Link,
            other => Self::Other(other),
        }
    }

    /// Whether this node has content to read. A directory and a link do not.
    pub fn is_file(&self) -> bool {
        matches!(self, Self::File)
    }
}
