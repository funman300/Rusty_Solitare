//! Secure storage for JWT access and refresh tokens using the OS keychain.
//!
//! Tokens are stored under service name `"solitaire_quest_server"` with entry
//! keys `"{username}_access"` and `"{username}_refresh"`.
//!
//! On Linux this requires a running secret service (GNOME Keyring / KWallet).
//! If the keychain is unavailable, operations return
//! [`TokenError::KeychainUnavailable`] — callers should fall back to prompting
//! the user to log in again.
//!
//! # Note: no unit tests — requires live OS keychain.

use keyring::Entry;
use thiserror::Error;

/// Errors that can occur when reading or writing tokens in the OS keychain.
#[derive(Debug, Error)]
pub enum TokenError {
    /// The OS keychain (secret service / keychain daemon) is not available.
    #[error("keychain unavailable: {0}")]
    KeychainUnavailable(String),
    /// No token was found in the keychain for the given username.
    #[error("token not found for user {0}")]
    NotFound(String),
    /// An unexpected keychain error occurred.
    #[error("keychain error: {0}")]
    Keyring(String),
}

/// Service name used to namespace all keychain entries for this application.
const SERVICE: &str = "solitaire_quest_server";

/// Map a `keyring::Error` to the appropriate `TokenError`.
fn map_keyring_err(err: keyring::Error, username: &str) -> TokenError {
    let msg = err.to_string();
    match err {
        keyring::Error::NoStorageAccess(_) => TokenError::KeychainUnavailable(msg),
        keyring::Error::NoEntry => TokenError::NotFound(username.to_string()),
        _ => TokenError::Keyring(msg),
    }
}

/// Store the access and refresh tokens for `username` in the OS keychain.
///
/// Any previously stored tokens for that username are overwritten.
pub fn store_tokens(
    username: &str,
    access_token: &str,
    refresh_token: &str,
) -> Result<(), TokenError> {
    Entry::new(SERVICE, &format!("{username}_access"))
        .map_err(|e| map_keyring_err(e, username))?
        .set_password(access_token)
        .map_err(|e| map_keyring_err(e, username))?;

    Entry::new(SERVICE, &format!("{username}_refresh"))
        .map_err(|e| map_keyring_err(e, username))?
        .set_password(refresh_token)
        .map_err(|e| map_keyring_err(e, username))?;

    Ok(())
}

/// Load the stored access token for `username` from the OS keychain.
///
/// Returns [`TokenError::NotFound`] if no token has been stored yet.
pub fn load_access_token(username: &str) -> Result<String, TokenError> {
    Entry::new(SERVICE, &format!("{username}_access"))
        .map_err(|e| map_keyring_err(e, username))?
        .get_password()
        .map_err(|e| map_keyring_err(e, username))
}

/// Load the stored refresh token for `username` from the OS keychain.
///
/// Returns [`TokenError::NotFound`] if no token has been stored yet.
pub fn load_refresh_token(username: &str) -> Result<String, TokenError> {
    Entry::new(SERVICE, &format!("{username}_refresh"))
        .map_err(|e| map_keyring_err(e, username))?
        .get_password()
        .map_err(|e| map_keyring_err(e, username))
}

/// Delete the stored access and refresh tokens for `username`.
///
/// Intended to be called on logout or account deletion. Missing entries are
/// silently ignored (the tokens are already gone, which is the desired state).
pub fn delete_tokens(username: &str) -> Result<(), TokenError> {
    match Entry::new(SERVICE, &format!("{username}_access"))
        .map_err(|e| map_keyring_err(e, username))?
        .delete_password()
    {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(map_keyring_err(e, username)),
    }

    match Entry::new(SERVICE, &format!("{username}_refresh"))
        .map_err(|e| map_keyring_err(e, username))?
        .delete_password()
    {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(map_keyring_err(e, username)),
    }

    Ok(())
}
