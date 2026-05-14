//! Fire-and-forget analytics client.
//!
//! Events are buffered in memory and flushed in a background task. Errors are
//! silently discarded — analytics must never affect gameplay or block the UI.

use std::sync::Mutex;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::Value;
use uuid::Uuid;

/// Buffers game-play events and flushes them to `POST /api/analytics`.
///
/// Construct once per session and share via `Arc`. `record` is cheap and
/// can be called from the Bevy main thread; `flush` is async and must be
/// called from a background task.
pub struct AnalyticsClient {
    base_url: String,
    /// Stable across the whole app session — one UUID per launch.
    session_id: String,
    client: Client,
    pending: Mutex<Vec<PendingEvent>>,
}

struct PendingEvent {
    id: String,
    event_type: String,
    payload: Value,
    client_time: DateTime<Utc>,
}

impl AnalyticsClient {
    /// Create a new client for the given server base URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            session_id: Uuid::new_v4().to_string(),
            client: Client::new(),
            pending: Mutex::new(Vec::new()),
        }
    }

    /// Buffer one event. Never blocks; never fails visibly.
    ///
    /// When the buffer exceeds 100 events the oldest 50 are dropped to
    /// prevent unbounded memory growth during extended offline play.
    pub fn record(&self, event_type: &str, payload: Value) {
        let Ok(mut guard) = self.pending.lock() else {
            return;
        };
        guard.push(PendingEvent {
            id: Uuid::new_v4().to_string(),
            event_type: event_type.to_owned(),
            payload,
            client_time: Utc::now(),
        });
        if guard.len() > 100 {
            guard.drain(0..50);
        }
    }

    /// Drain the pending buffer and POST it to the server.
    ///
    /// The buffer is drained *before* the HTTP call so new events recorded
    /// during an in-flight flush are not lost. On network failure the drained
    /// events are silently discarded (fire-and-forget semantics).
    pub async fn flush(&self, user_id: Option<String>) {
        let events = {
            let Ok(mut guard) = self.pending.lock() else {
                return;
            };
            if guard.is_empty() {
                return;
            }
            std::mem::take(&mut *guard)
        };

        let batch = serde_json::json!({
            "session_id": self.session_id,
            "user_id": user_id,
            "events": events.iter().map(|e| serde_json::json!({
                "id": e.id,
                "event_type": e.event_type,
                "payload": e.payload,
                "client_time": e.client_time.to_rfc3339(),
            })).collect::<Vec<_>>(),
        });

        let _ = self
            .client
            .post(format!("{}/api/analytics", self.base_url))
            .json(&batch)
            .send()
            .await;
    }
}
