//! Logging in, and the identity a login resolves to.

use crate::{AuthLoginWithTokenArgs, Event, GlobalArgs, Lore, LoreError};

/// Who Lore says a user is.
#[derive(Debug, Clone)]
pub struct UserInfo {
    /// The identity, which is what [`GlobalArgs::identity`] takes.
    pub id: String,
    /// A display name: the token's `preferred_username`, its `name`, or
    /// [`Self::id`], whichever Lore has first, so never empty.
    pub name: String,
}

impl Lore {
    /// Logs in with a token obtained elsewhere, and reports the identity it
    /// authenticated as.
    ///
    /// Lore writes the token it gets to a store that is per OS user rather
    /// than per process or per repository — the same one its CLI uses — so a
    /// login here is what every later call in this crate authenticates with,
    /// and it outlives this process.
    ///
    /// A rejected token and an unreachable server are both
    /// [`LoreError::Call`], differing only in the message Lore attaches:
    /// there is no error code for either, so nothing here can tell them
    /// apart.
    ///
    /// This corresponds to `lore_sys::Lore::lore_auth_login_with_token`.
    pub fn auth_login_with_token(
        &self,
        globals: &GlobalArgs,
        args: AuthLoginWithTokenArgs<'_>,
    ) -> Result<UserInfo, LoreError> {
        const COMMAND: &str = "auth::login_with_token";
        let mut info = None;

        crate::call::auth_login_with_token(self, COMMAND, globals, args, |event| {
            crate::log_event(&event);

            if let Ok(Event::AuthUserInfo { id, name }) = event {
                info = Some(UserInfo {
                    id: id.to_owned(),
                    name: name.to_owned(),
                });
            }
        })?;

        let info = info.ok_or(LoreError::MissingEvent {
            command: COMMAND,
            expected: "auth_user_info",
        })?;

        if info.id.is_empty() {
            return Err(LoreError::InvalidResponse {
                command: COMMAND,
                detail: "the user identity is empty".to_owned(),
            });
        }

        Ok(info)
    }
}
