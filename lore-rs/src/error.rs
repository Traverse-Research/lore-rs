use lore_sys::{
    libloading, lore_error_code_t, LORE_ERROR_CODE_ADDRESS_NOT_FOUND, LORE_ERROR_CODE_INTERNAL,
    LORE_ERROR_CODE_INVALID_ARGUMENTS, LORE_ERROR_CODE_NONE, LORE_ERROR_CODE_SLOW_DOWN,
};

/// The outcome Lore attaches to a terminal event: `lore_error_code_t` as an
/// enum. These are per-item codes (a storage item, a tree lookup), distinct
/// from the FFI status a whole call returns.
///
/// [`Self::Other`] rather than `#[non_exhaustive]`: the C type is an integer,
/// so an unknown value is a runtime possibility and the caller should be able
/// to see which one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// The arguments were invalid. Lore also folds "no such path" in a
    /// revision tree into this one.
    InvalidArguments,
    /// Neither the local store nor the remote holds the address.
    AddressNotFound,
    Internal,
    /// The backing store is overloaded; retry later.
    SlowDown,
    Other(lore_error_code_t),
}

impl ErrorCode {
    /// [`None`] for `LORE_ERROR_CODE_NONE`, which is success.
    pub(crate) fn from_raw(code: lore_error_code_t) -> Option<Self> {
        Some(match code {
            LORE_ERROR_CODE_NONE => return None,
            LORE_ERROR_CODE_INVALID_ARGUMENTS => Self::InvalidArguments,
            LORE_ERROR_CODE_ADDRESS_NOT_FOUND => Self::AddressNotFound,
            LORE_ERROR_CODE_INTERNAL => Self::Internal,
            LORE_ERROR_CODE_SLOW_DOWN => Self::SlowDown,
            other => Self::Other(other),
        })
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArguments => f.write_str("invalid arguments"),
            Self::AddressNotFound => f.write_str("address not found"),
            Self::Internal => f.write_str("internal error"),
            Self::SlowDown => f.write_str("slow down (rate limited)"),
            Self::Other(code) => write!(f, "unknown error code {code}"),
        }
    }
}

/// Everything that can go wrong reaching Lore.
///
/// Every variant names the lore API it came from, as `"branch::info"` rather
/// than `lore_branch_info`.
///
/// Only [`Self::Call`] and [`Self::Failed`] come from Lore saying no. The
/// rest are answers this crate could not use.
#[derive(Debug)]
#[non_exhaustive]
pub enum LoreError {
    /// Non-zero status and no more specific outcome. `status` is the error's
    /// FFI code, which Lore does not yet promise stable. `messages` holds
    /// every `LORE_EVENT_ERROR` message, or the failed completion's message
    /// when there were none, and is empty when the call failed before
    /// emitting anything.
    Call {
        command: &'static str,
        status: i32,
        messages: Vec<String>,
    },

    /// The terminal event of the item or lookup carried a non-zero
    /// [`ErrorCode`]. Wins over [`Self::Call`] when both apply, because the
    /// code is the specific outcome and the status a summary of it; the
    /// call's messages come along.
    Failed {
        command: &'static str,
        code: ErrorCode,
        messages: Vec<String>,
    },

    /// Succeeded, but no event carried the answer. `expected` names the event
    /// that did not arrive.
    MissingEvent {
        command: &'static str,
        expected: &'static str,
    },

    /// Lore answered, but with a value that cannot be used — an all-zero
    /// identifier where one is required, or a node from another tree.
    InvalidResponse {
        command: &'static str,
        detail: String,
    },

    /// A read delivered a different number of bytes than it announced.
    /// `covered` counts bytes actually written, so a skipped range fails here
    /// even when the delivered ranges add up.
    SizeMismatch {
        command: &'static str,
        expected: u64,
        covered: u64,
    },

    /// The caller's sink refused the bytes a read produced.
    Io {
        command: &'static str,
        source: std::io::Error,
    },

    /// The shared library could not be loaded.
    Load(libloading::Error),
}

impl LoreError {
    /// Whether Lore reported that neither the local store nor the remote
    /// holds the address that was asked for.
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Self::Failed {
                code: ErrorCode::AddressNotFound,
                ..
            }
        )
    }

    /// The lore API the error came from, for every variant that has one.
    pub fn command(&self) -> Option<&'static str> {
        match self {
            Self::Call { command, .. }
            | Self::Failed { command, .. }
            | Self::MissingEvent { command, .. }
            | Self::InvalidResponse { command, .. }
            | Self::SizeMismatch { command, .. }
            | Self::Io { command, .. } => Some(command),
            Self::Load(_) => None,
        }
    }

    /// Combines how a call returned with the outcome its terminal event
    /// carried. A non-zero outcome wins: Lore turns any failed item into a
    /// non-zero status, so the status alone would say "1/1 items failed" where
    /// the event says "address not found".
    pub(crate) fn resolve(
        command: &'static str,
        result: Result<(), LoreError>,
        outcome: Option<lore_error_code_t>,
    ) -> Result<(), LoreError> {
        match (result, outcome.and_then(ErrorCode::from_raw)) {
            (Err(Self::Call { messages, .. }), Some(code)) => Err(Self::Failed {
                command,
                code,
                messages,
            }),
            (Err(other), Some(code)) => Err(Self::Failed {
                command,
                code,
                messages: vec![other.to_string()],
            }),
            (Ok(()), Some(code)) => Err(Self::Failed {
                command,
                code,
                messages: Vec::new(),
            }),
            (result, None) => result,
        }
    }
}

impl std::fmt::Display for LoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Call {
                command,
                status,
                messages,
            } => {
                write!(f, "`{command}` failed with status {status}")?;
                if !messages.is_empty() {
                    write!(f, ": {}", messages.join("; "))?;
                }
                Ok(())
            }
            Self::Failed {
                command,
                code,
                messages,
            } => {
                write!(f, "`{command}` failed: {code}")?;
                if !messages.is_empty() {
                    write!(f, " ({})", messages.join("; "))?;
                }
                Ok(())
            }
            Self::MissingEvent { command, expected } => {
                write!(f, "`{command}` succeeded but reported no `{expected}`")
            }
            Self::InvalidResponse { command, detail } => {
                write!(f, "`{command}` answered but {detail}")
            }
            Self::SizeMismatch {
                command,
                expected,
                covered,
            } => write!(
                f,
                "`{command}` announced {expected} bytes but delivered {covered}"
            ),
            Self::Io { command, source } => {
                write!(f, "`{command}`: writing the bytes it produced: {source}")
            }
            Self::Load(error) => write!(f, "loading lore's shared library: {error}"),
        }
    }
}

impl std::error::Error for LoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Load(error) => Some(error),
            _ => None,
        }
    }
}

impl From<libloading::Error> for LoreError {
    fn from(error: libloading::Error) -> Self {
        Self::Load(error)
    }
}
