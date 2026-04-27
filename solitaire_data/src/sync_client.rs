//! Concrete [`SyncProvider`] implementations and a factory for constructing
//! the correct provider from a [`SyncBackend`] setting.
//!
//! # Backends
//!
//! | Struct | Backend |
//! |---|---|
//! | [`LocalOnlyProvider`] | No-op; used when sync is disabled |
//! | [`SolitaireServerClient`] | Self-hosted Solitaire Quest server (JWT auth) |
//!
//! Use [`provider_for_backend`] to obtain a `Box<dyn SyncProvider + Send + Sync>`
//! without matching on [`SyncBackend`] anywhere else in the codebase.

use async_trait::async_trait;
use solitaire_sync::{ChallengeGoal, LeaderboardEntry, SyncPayload, SyncResponse};

use crate::{
    auth_tokens::{load_access_token, load_refresh_token, store_tokens},
    settings::SyncBackend,
    SyncError, SyncProvider,
};

// ---------------------------------------------------------------------------
// LocalOnlyProvider
// ---------------------------------------------------------------------------

/// A no-op sync provider used when the player has not configured any backend.
///
/// Both [`pull`](SyncProvider::pull) and [`push`](SyncProvider::push) always
/// return [`SyncError::UnsupportedPlatform`], so callers know no remote data
/// is available without treating it as a fatal error.
pub struct LocalOnlyProvider;

#[async_trait]
impl SyncProvider for LocalOnlyProvider {
    async fn pull(&self) -> Result<SyncPayload, SyncError> {
        Err(SyncError::UnsupportedPlatform)
    }

    async fn push(&self, _payload: &SyncPayload) -> Result<SyncResponse, SyncError> {
        Err(SyncError::UnsupportedPlatform)
    }

    fn backend_name(&self) -> &'static str {
        "local"
    }

    fn is_authenticated(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// SolitaireServerClient
// ---------------------------------------------------------------------------

/// HTTP sync client for the self-hosted Solitaire Quest server.
///
/// Authenticates via JWT stored in the OS keychain. On a 401 response the
/// client automatically attempts a token refresh and retries the request once
/// before returning an error.
pub struct SolitaireServerClient {
    /// Base URL of the server, e.g. `"https://solitaire.example.com"`.
    /// Trailing slashes are stripped on construction.
    base_url: String,
    /// The player's username on this server — used as the keychain key.
    username: String,
    /// Shared `reqwest` client (keeps connection pools alive across calls).
    client: reqwest::Client,
}

impl SolitaireServerClient {
    /// Construct a new client for the given server URL and username.
    ///
    /// The `base_url` trailing slash is stripped so URL construction is
    /// consistent regardless of how the user entered the setting.
    pub fn new(base_url: impl Into<String>, username: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            username: username.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Attempt to refresh the access token using the stored refresh token.
    ///
    /// On success the new access token is persisted to the OS keychain,
    /// replacing the previous one. The refresh token itself is unchanged.
    async fn refresh_token(&self) -> Result<(), SyncError> {
        let refresh = load_refresh_token(&self.username)
            .map_err(|e| SyncError::Auth(e.to_string()))?;

        let resp = self
            .client
            .post(format!("{}/api/auth/refresh", self.base_url))
            .json(&serde_json::json!({ "refresh_token": refresh }))
            .send()
            .await
            .map_err(|e| SyncError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(SyncError::Auth("refresh failed".into()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SyncError::Serialization(e.to_string()))?;

        let new_access = body["access_token"]
            .as_str()
            .ok_or_else(|| SyncError::Serialization("missing access_token in refresh response".into()))?;

        // store_tokens replaces both access and refresh; we keep the old
        // refresh token unchanged so its 30-day TTL is preserved.
        store_tokens(&self.username, new_access, &refresh)
            .map_err(|e| SyncError::Auth(e.to_string()))
    }

    /// Load the current access token from the OS keychain.
    fn access_token(&self) -> Result<String, SyncError> {
        load_access_token(&self.username).map_err(|e| SyncError::Auth(e.to_string()))
    }
}

#[async_trait]
impl SyncProvider for SolitaireServerClient {
    /// Fetch the latest sync payload from the server.
    ///
    /// On HTTP 401 the client refreshes the access token and retries once.
    async fn pull(&self) -> Result<SyncPayload, SyncError> {
        let token = self.access_token()?;
        let url = format!("{}/api/sync/pull", self.base_url);

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SyncError::Network(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            // Token expired — refresh and retry once.
            self.refresh_token().await?;
            let new_token = self.access_token()?;
            let resp = self
                .client
                .get(&url)
                .bearer_auth(new_token)
                .send()
                .await
                .map_err(|e| SyncError::Network(e.to_string()))?;
            return extract_pull_body(resp).await;
        }

        extract_pull_body(resp).await
    }

    /// Push the local payload to the server and return the merged response.
    ///
    /// On HTTP 401 the client refreshes the access token and retries once.
    async fn push(&self, payload: &SyncPayload) -> Result<SyncResponse, SyncError> {
        let token = self.access_token()?;
        let url = format!("{}/api/sync/push", self.base_url);

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(payload)
            .send()
            .await
            .map_err(|e| SyncError::Network(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            // Token expired — refresh and retry once.
            self.refresh_token().await?;
            let new_token = self.access_token()?;
            let resp = self
                .client
                .post(&url)
                .bearer_auth(new_token)
                .json(payload)
                .send()
                .await
                .map_err(|e| SyncError::Network(e.to_string()))?;
            return extract_push_body(resp).await;
        }

        extract_push_body(resp).await
    }

    fn backend_name(&self) -> &'static str {
        "solitaire_server"
    }

    /// Returns `true` if a valid access token is present in the OS keychain.
    fn is_authenticated(&self) -> bool {
        load_access_token(&self.username).is_ok()
    }

    /// Fetch today's daily challenge from the server.
    ///
    /// Does not require authentication — the endpoint is public. Returns `None`
    /// on any non-success HTTP status so the caller falls back to the local seed.
    async fn fetch_daily_challenge(&self) -> Result<Option<ChallengeGoal>, SyncError> {
        let url = format!("{}/api/daily-challenge", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SyncError::Network(e.to_string()))?;

        if resp.status().is_success() {
            let goal: ChallengeGoal = resp
                .json()
                .await
                .map_err(|e| SyncError::Serialization(e.to_string()))?;
            Ok(Some(goal))
        } else {
            // Non-fatal — caller will use the locally computed seed instead.
            Ok(None)
        }
    }

    async fn opt_in_leaderboard(&self, display_name: &str) -> Result<(), SyncError> {
        let token = self.access_token()?;
        let url = format!("{}/api/leaderboard/opt-in", self.base_url);

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(&serde_json::json!({ "display_name": display_name }))
            .send()
            .await
            .map_err(|e| SyncError::Network(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.refresh_token().await?;
            let new_token = self.access_token()?;
            let resp = self
                .client
                .post(&url)
                .bearer_auth(new_token)
                .json(&serde_json::json!({ "display_name": display_name }))
                .send()
                .await
                .map_err(|e| SyncError::Network(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(SyncError::Auth(format!("opt-in failed: {}", resp.status())));
            }
            return Ok(());
        }

        if !resp.status().is_success() {
            return Err(SyncError::Auth(format!("opt-in failed: {}", resp.status())));
        }
        Ok(())
    }

    async fn opt_out_leaderboard(&self) -> Result<(), SyncError> {
        let token = self.access_token()?;
        let url = format!("{}/api/leaderboard/opt-in", self.base_url);

        let resp = self
            .client
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SyncError::Network(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.refresh_token().await?;
            let new_token = self.access_token()?;
            let resp = self
                .client
                .delete(&url)
                .bearer_auth(new_token)
                .send()
                .await
                .map_err(|e| SyncError::Network(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(SyncError::Auth(format!("opt-out failed: {}", resp.status())));
            }
            return Ok(());
        }

        if !resp.status().is_success() {
            return Err(SyncError::Auth(format!("opt-out failed: {}", resp.status())));
        }
        Ok(())
    }

    async fn delete_account(&self) -> Result<(), SyncError> {
        let token = self.access_token()?;
        let url = format!("{}/api/account", self.base_url);

        let resp = self
            .client
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SyncError::Network(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.refresh_token().await?;
            let new_token = self.access_token()?;
            let resp = self
                .client
                .delete(&url)
                .bearer_auth(new_token)
                .send()
                .await
                .map_err(|e| SyncError::Network(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(SyncError::Auth(format!("delete account failed: {}", resp.status())));
            }
            return Ok(());
        }

        if !resp.status().is_success() {
            return Err(SyncError::Auth(format!("delete account failed: {}", resp.status())));
        }
        Ok(())
    }

    async fn fetch_leaderboard(&self) -> Result<Vec<LeaderboardEntry>, SyncError> {
        let token = self.access_token()?;
        let url = format!("{}/api/leaderboard", self.base_url);

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SyncError::Network(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.refresh_token().await?;
            let new_token = self.access_token()?;
            let resp = self
                .client
                .get(&url)
                .bearer_auth(new_token)
                .send()
                .await
                .map_err(|e| SyncError::Network(e.to_string()))?;
            return extract_leaderboard_body(resp).await;
        }

        extract_leaderboard_body(resp).await
    }
}

// ---------------------------------------------------------------------------
// Response extraction helpers
// ---------------------------------------------------------------------------

/// Deserialize a pull response body as [`SyncResponse`] and return its
/// `merged` field, or map non-200 statuses to the appropriate [`SyncError`].
async fn extract_pull_body(resp: reqwest::Response) -> Result<SyncPayload, SyncError> {
    let status = resp.status();
    if status.is_success() {
        let sync_resp: SyncResponse = resp
            .json()
            .await
            .map_err(|e| SyncError::Serialization(e.to_string()))?;
        Ok(sync_resp.merged)
    } else {
        Err(SyncError::Auth(format!("server returned {status}")))
    }
}

/// Deserialize a leaderboard response body as `Vec<LeaderboardEntry>`.
async fn extract_leaderboard_body(resp: reqwest::Response) -> Result<Vec<LeaderboardEntry>, SyncError> {
    let status = resp.status();
    if status.is_success() {
        resp.json()
            .await
            .map_err(|e| SyncError::Serialization(e.to_string()))
    } else {
        Err(SyncError::Network(format!("server returned {status}")))
    }
}

/// Deserialize a push response body as [`SyncResponse`], or map non-200
/// statuses to the appropriate [`SyncError`].
async fn extract_push_body(resp: reqwest::Response) -> Result<SyncResponse, SyncError> {
    let status = resp.status();
    if status.is_success() {
        resp.json()
            .await
            .map_err(|e| SyncError::Serialization(e.to_string()))
    } else {
        Err(SyncError::Auth(format!("server returned {status}")))
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Construct the appropriate [`SyncProvider`] for the given [`SyncBackend`]
/// setting.
///
/// This is the **one** place in the codebase that matches on [`SyncBackend`]
/// variants. All other code receives a `Box<dyn SyncProvider + Send + Sync>`
/// and remains backend-agnostic.
///
/// `GooglePlayGames` is Android-only; on desktop it silently falls back to
/// [`LocalOnlyProvider`].
pub fn provider_for_backend(backend: &SyncBackend) -> Box<dyn SyncProvider + Send + Sync> {
    match backend {
        SyncBackend::Local => Box::new(LocalOnlyProvider),
        SyncBackend::SolitaireServer { url, username } => {
            Box::new(SolitaireServerClient::new(url.clone(), username.clone()))
        }
        SyncBackend::GooglePlayGames => {
            // GPGS is Android-only; fall back to no-op on desktop.
            Box::new(LocalOnlyProvider)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_provider_backend_name() {
        assert_eq!(LocalOnlyProvider.backend_name(), "local");
    }

    #[test]
    fn local_provider_not_authenticated() {
        assert!(!LocalOnlyProvider.is_authenticated());
    }

    #[tokio::test]
    async fn local_provider_pull_returns_unsupported() {
        let err = LocalOnlyProvider.pull().await.unwrap_err();
        assert!(matches!(err, SyncError::UnsupportedPlatform));
    }

    #[test]
    fn server_client_strips_trailing_slash() {
        let c = SolitaireServerClient::new("https://example.com/", "alice");
        assert_eq!(c.base_url, "https://example.com");
    }

    #[test]
    fn server_client_backend_name() {
        let c = SolitaireServerClient::new("https://example.com", "alice");
        assert_eq!(c.backend_name(), "solitaire_server");
    }

    #[test]
    fn factory_local_returns_local_provider() {
        let provider = provider_for_backend(&SyncBackend::Local);
        assert_eq!(provider.backend_name(), "local");
    }

    #[test]
    fn factory_gpgs_falls_back_to_local() {
        let provider = provider_for_backend(&SyncBackend::GooglePlayGames);
        assert_eq!(provider.backend_name(), "local");
    }

    #[test]
    fn factory_server_returns_server_client() {
        let provider = provider_for_backend(&SyncBackend::SolitaireServer {
            url: "https://example.com".to_string(),
            username: "bob".to_string(),
        });
        assert_eq!(provider.backend_name(), "solitaire_server");
    }
}
