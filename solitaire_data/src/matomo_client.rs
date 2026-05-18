//! Matomo HTTP Tracking API client.
//!
//! Buffers game-play events and flushes them via the Matomo bulk tracking
//! endpoint. Errors are silently discarded — analytics must never affect
//! gameplay or block the UI.

use std::sync::Mutex;

use reqwest::Client;
use uuid::Uuid;

/// Sends game-play events to a self-hosted Matomo instance via the
/// [HTTP Tracking API](https://developer.matomo.org/api-reference/tracking-api).
///
/// Construct once per session and share via `Arc`. `event` is cheap and
/// can be called from the Bevy main thread; `flush` is async and must be
/// called from a background task.
pub struct MatomoClient {
    tracking_url: String,
    site_id: u32,
    /// 16 hex-char visitor ID, stable for the lifetime of this client.
    visitor_id: String,
    uid: Option<String>,
    client: Client,
    /// Pre-encoded query strings, one per buffered event.
    pending: Mutex<Vec<String>>,
}

impl MatomoClient {
    /// Create a new client targeting `base_url` (e.g. `"https://analytics.example.com"`).
    pub fn new(base_url: impl AsRef<str>, site_id: u32, uid: Option<String>) -> Self {
        let base = base_url.as_ref().trim_end_matches('/');
        let tracking_url = format!("{}/matomo.php", base);
        // Take the lower 64 bits of a v4 UUID and format as 16 hex chars.
        let visitor_id = format!("{:016x}", Uuid::new_v4().as_u128() as u64);
        Self {
            tracking_url,
            site_id,
            visitor_id,
            uid,
            client: Client::new(),
            pending: Mutex::new(Vec::new()),
        }
    }

    /// Buffer one Matomo custom event. Never blocks; never fails visibly.
    ///
    /// When the buffer exceeds 100 events the oldest 50 are dropped to
    /// prevent unbounded memory growth during extended offline play.
    pub fn event(
        &self,
        category: &str,
        action: &str,
        name: Option<&str>,
        value: Option<f64>,
    ) {
        let Ok(mut guard) = self.pending.lock() else {
            return;
        };

        let mut qs = format!(
            "idsite={}&rec=1&apiv=1&send_image=0\
             &url=game%3A%2F%2Fsolitaire%2Fevent\
             &_id={}&e_c={}&e_a={}",
            self.site_id,
            self.visitor_id,
            url_encode(category),
            url_encode(action),
        );
        if let Some(n) = name {
            qs.push_str(&format!("&e_n={}", url_encode(n)));
        }
        if let Some(v) = value {
            qs.push_str(&format!("&e_v={v}"));
        }
        if let Some(uid) = &self.uid {
            qs.push_str(&format!("&uid={}", url_encode(uid)));
        }

        guard.push(qs);
        if guard.len() > 100 {
            guard.drain(0..50);
        }
    }

    /// Drain the pending buffer and POST it to Matomo's bulk tracking endpoint.
    ///
    /// The buffer is drained *before* the HTTP call so events recorded during
    /// an in-flight flush are not lost. Network errors are silently discarded.
    pub async fn flush(&self) {
        let pending = {
            let Ok(mut guard) = self.pending.lock() else {
                return;
            };
            if guard.is_empty() {
                return;
            }
            std::mem::take(&mut *guard)
        };

        let requests: Vec<String> = pending.into_iter().map(|qs| format!("?{qs}")).collect();
        let body = serde_json::json!({ "requests": requests });

        let _ = self
            .client
            .post(&self.tracking_url)
            .json(&body)
            .send()
            .await;
    }
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .flat_map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![b as char]
            }
            b => format!("%{b:02X}").chars().collect(),
        })
        .collect()
}
