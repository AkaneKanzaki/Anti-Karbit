//! Group monitoring and claim orchestration.
//!
//! Three design decisions carry the speed and the reliability:
//!
//! - Engines run **in parallel**, and the claim is sent using the fastest
//!   result rather than waiting for every engine to finish.
//! - Each message is handled in its own task, so a claim waiting for
//!   verification does not hold up the next spawn. Without this, anything
//!   spawning during the verification window is lost.
//! - Results are cached by image hash, so a repeat appearance skips the
//!   reverse search entirely.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use grammers_client::client::UpdateStream;
use grammers_client::media::Media;
use grammers_client::update::{Message as UpdateMessage, Update};
use grammers_client::Client;
use grammers_session::types::{PeerId, PeerRef};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::cache::RecognitionCache;
use crate::config;
use crate::recognizer::{CharacterInfo, MultiEngine};
use super::claimer::Claimer;

/// Counters shown on the dashboard.
#[derive(Default)]
pub struct Stats {
    pub detected: AtomicU64,
    pub claimed: AtomicU64,
    pub failed: AtomicU64,
    pub started_at: Mutex<Option<Instant>>,
}

impl Stats {
    pub fn mark_started(&self) {
        if let Ok(mut s) = self.started_at.lock()
            && s.is_none() {
                *s = Some(Instant::now());
            }
    }

    pub fn start_time_unix(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "detected": self.detected.load(Ordering::Relaxed),
            "claimed": self.claimed.load(Ordering::Relaxed),
            "failed": self.failed.load(Ordering::Relaxed),
            "start_time": self.start_time_unix(),
        })
    }
}

#[derive(Clone)]
pub struct WaifuListener {
    client: Client,
    engine: Arc<MultiEngine>,
    claimer: Arc<Claimer>,
    cache: Arc<RecognitionCache>,
    pub stats: Arc<Stats>,
    recent_events: Arc<Mutex<VecDeque<Value>>>,
    /// The application's event bus, not the listener's. This lets SSE clients
    /// subscribe even before the Telegram bot has managed to connect.
    events_tx: broadcast::Sender<Value>,
    pub is_active: Arc<AtomicBool>,
}

impl WaifuListener {
    pub fn new(
        client: Client,
        engine: MultiEngine,
        claimer: Claimer,
        events_tx: broadcast::Sender<Value>,
    ) -> Self {
        let listener = Self {
            client,
            engine: Arc::new(engine),
            claimer: Arc::new(claimer),
            cache: Arc::new(RecognitionCache::new()),
            stats: Arc::new(Stats::default()),
            recent_events: Arc::new(Mutex::new(VecDeque::with_capacity(50))),
            events_tx,
            is_active: Arc::new(AtomicBool::new(true)),
        };
        listener.stats.mark_started();
        listener
    }

    pub fn recent_events(&self) -> Vec<Value> {
        self.recent_events
            .lock()
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn emit(&self, payload: Value) {
        if let Ok(mut q) = self.recent_events.lock() {
            if q.len() >= 50 {
                q.pop_front();
            }
            q.push_back(payload.clone());
        }
        // An error here just means nobody is subscribed yet.
        let _ = self.events_tx.send(payload);
    }

    pub fn toggle(&self) -> bool {
        let now = !self.is_active.load(Ordering::Relaxed);
        self.is_active.store(now, Ordering::Relaxed);
        now
    }

    /// Run the update loop. Never returns until the stream ends.
    pub async fn run(&self, mut stream: UpdateStream) {
        info!("Listener active. Waiting for characters to spawn in groups...");

        loop {
            let update = match stream.next().await {
                Ok(u) => u,
                Err(e) => {
                    warn!("Update stream error: {e}");
                    continue;
                }
            };

            if let Update::NewMessage(msg) = update {
                let this = self.clone();

                // Each message gets its own task, so a claim in flight does not
                // hold up the next spawn.
                tokio::spawn(async move {
                    this.handle_message(msg).await;
                });
            }
        }
    }

    async fn handle_message(&self, msg: UpdateMessage) {
        // A reply to a claim in flight must always be processed, even while
        // the listener is paused.
        let peer = msg.peer_id();
        let text = msg.text().to_string();

        if !text.is_empty() && !msg.outgoing() && self.claimer.deliver_reply(peer, text.clone()) {
            return;
        }

        if !self.is_active.load(Ordering::Relaxed) {
            return;
        }

        let text_lower = text.to_lowercase();
        let matched = config::get()
            .trigger_keywords
            .iter()
            .any(|kw| text_lower.contains(&kw.to_lowercase()));
        if !matched {
            return;
        }

        let Some(media) = msg.media() else {
            return;
        };
        if !media_is_image(&media) {
            return;
        }

        self.stats.detected.fetch_add(1, Ordering::Relaxed);

        let sender_name = msg
            .sender()
            .and_then(|s| s.name())
            .unwrap_or("Unknown")
            .to_string();

        info!("==> CHARACTER SPAWNED! From {sender_name} in chat {peer:?} <==");

        let image_bytes = match self.download_media(&media).await {
            Some(b) if !b.is_empty() => b,
            _ => {
                warn!("Could not download the character image.");
                return;
            }
        };

        info!("Image size: {} bytes. Starting recognition...", image_bytes.len());

        // --- Cache ---
        let cache_key = RecognitionCache::key(&image_bytes);
        let (cached, from_cache) = match self.cache.get(&cache_key) {
            Some(info) => {
                info!("Cache hit for '{}'; skipping reverse search", info.full_name);
                (Some(info), true)
            }
            None => (None, false),
        };

        // Resolve the PeerRef so it carries a valid `access_hash`.
        // `to_ambient_ref()` produces an auth-less ref, and Telegram silently
        // drops sends to channels/supergroups without an access hash: the
        // request never gets a response, so `send_message` hangs forever
        // instead of returning an error. `resolve_peer` -> `to_ref` yields a
        // properly authenticated ref. Only fall back to the ambient ref when
        // resolution genuinely fails, so a transient error degrades to the old
        // behaviour instead of dropping the claim.
        let peer_ref = self
            .resolve_peer_ref(&peer)
            .await
            .unwrap_or_else(|| peer.to_ambient_ref());
        let reply_to = Some(msg.id());

        if let Some(info) = cached {
            self.try_claim(&info, peer, peer_ref, reply_to, &sender_name, from_cache)
                .await;
            return;
        }

        // --- Parallel fan-out: claim with the fastest result ---
        let shared = Arc::new(image_bytes);
        let mut rx = self.engine.spawn_parallel(shared);

        let mut tried: Vec<String> = Vec::new();
        let mut claimed = false;
        let mut last_reason: Option<String> = None;

        while let Some((kind, result)) = rx.recv().await {
            let Some(character) = result else {
                info!("{} found no character candidate.", kind.label());
                continue;
            };

            info!(
                "Karakter ditemukan ({}): {} (Seri: {}) | Confidence: {:.0}%",
                kind.label(),
                character.full_name,
                character.series.as_deref().unwrap_or("Unknown"),
                character.confidence * 100.0
            );

            // Cache it so the next appearance is instant.
            self.cache.put(cache_key.clone(), character.clone());

            let mut names = vec![character.full_name.clone()];
            names.extend(character.alternate_names.iter().cloned());

            for name in names {
                let norm = name.trim().to_lowercase();
                if norm.is_empty() || tried.contains(&norm) {
                    continue;
                }
                tried.push(norm);

                let mut candidate = character.clone();
                candidate.full_name = name.clone();

                let result = self
                    .claimer
                    .execute_claim(
                        &self.client,
                        peer,
                        peer_ref,
                        &candidate,
                        reply_to,
                    )
                    .await;

                if result.success {
                    claimed = true;
                    self.stats.claimed.fetch_add(1, Ordering::Relaxed);
                    info!(
                        "KLAIM BERHASIL via {} dengan nama '{}'",
                        kind.label(),
                        result.name
                    );
                    self.emit(json!({
                        "type": "claim_result",
                        "timestamp": unix_now(),
                        "character_name": name,
                        "first_name": candidate.first_name,
                        "last_name": candidate.last_name,
                        "series": character.series.clone().unwrap_or_else(|| "Unknown".into()),
                        "confidence": (character.confidence * 1000.0).round() / 10.0,
                        "source": character.source,
                        "success": true,
                        "claim_name": result.name,
                        "response": result.response_text,
                        "bot_name": sender_name,
                        "engine": kind.label(),
                    }));
                    break;
                } else {
                    last_reason = result.response_text.clone();
                }
            }

            if claimed {
                break;
            }
            warn!("All candidates from {} were rejected. Waiting for other engines...", kind.label());
        }

        if !claimed {
            self.stats.failed.fetch_add(1, Ordering::Relaxed);
            warn!("Every engine was tried and no claim succeeded.");
            self.emit(json!({
                "type": "claim_failed",
                "timestamp": unix_now(),
                "reason": last_reason.unwrap_or_else(|| {
                    "Karakter tidak dikenali di semua engine pencarian".to_string()
                }),
                "bot_name": sender_name,
            }));
        }
    }

    async fn try_claim(
        &self,
        info: &CharacterInfo,
        peer: PeerId,
        peer_ref: PeerRef,
        reply_to: Option<i32>,
        sender_name: &str,
        from_cache: bool,
    ) {
        let result = self
            .claimer
            .execute_claim(&self.client, peer, peer_ref, info, reply_to)
            .await;

        if result.success {
            self.stats.claimed.fetch_add(1, Ordering::Relaxed);
            info!("Claim succeeded (cache={}) as '{}'", from_cache, result.name);
            self.emit(json!({
                "type": "claim_result",
                "timestamp": unix_now(),
                "character_name": info.full_name,
                "first_name": info.first_name,
                "last_name": info.last_name,
                "series": info.series.clone().unwrap_or_else(|| "Unknown".into()),
                "confidence": (info.confidence * 1000.0).round() / 10.0,
                "source": if from_cache { "cache".to_string() } else { info.source.clone() },
                "success": true,
                "claim_name": result.name,
                "response": result.response_text,
                "bot_name": sender_name,
                "engine": "cache",
            }));
        } else {
            self.stats.failed.fetch_add(1, Ordering::Relaxed);
            self.emit(json!({
                "type": "claim_failed",
                "timestamp": unix_now(),
                "reason": result.response_text.unwrap_or_else(|| "Klaim ditolak".into()),
                "bot_name": sender_name,
            }));
        }
    }

    /// Download media into memory rather than to disk.
    async fn download_media(&self, media: &Media) -> Option<Vec<u8>> {
        let mut iter = self.client.iter_download(media);
        let mut buffer: Vec<u8> = Vec::new();

        loop {
            match iter.next().await {
                Ok(Some(chunk)) => buffer.extend_from_slice(&chunk),
                Ok(None) => break,
                Err(e) => {
                    warn!("Could not download the image: {e}");
                    return None;
                }
            }
        }

        Some(buffer)
    }

    pub fn cache_stats(&self) -> (usize, u64, u64) {
        self.cache.stats()
    }

    /// Resolve a peer into a `PeerRef` that carries a valid `access_hash`.
    ///
    /// This matters for channels and supergroups: an auth-less ref makes
    /// Telegram drop the request without answering, which hangs `send_message`.
    /// Returns `None` when the peer cannot be resolved, so the caller can fall
    /// back to an ambient ref.
    async fn resolve_peer_ref(&self, peer: &PeerId) -> Option<PeerRef> {
        // `resolve_peer` takes a PeerRef, and the only conversion from a bare
        // PeerId is the ambient one. That is fine to *look up*: the server
        // returns the peer together with its access hash, and `to_ref()` then
        // yields an authenticated ref suitable for sending.
        match self.client.resolve_peer(peer.to_ambient_ref()).await {
            Ok(found) => match found.to_ref().await {
                Ok(Some(peer_ref)) => Some(peer_ref),
                Ok(None) => {
                    warn!("Peer {peer:?} has no usable access hash; sends may not be delivered.");
                    None
                }
                Err(e) => {
                    warn!("Could not build a PeerRef for {peer:?}: {e}");
                    None
                }
            },
            Err(e) => {
                warn!("Could not resolve peer {peer:?}: {e}");
                None
            }
        }
    }

    /// Labels of the currently active engines, for the dashboard.
    pub fn active_engines(&self) -> Vec<&'static str> {
        self.engine.active_labels()
    }
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Whether this media is an image that can be sent to reverse image search.
fn media_is_image(media: &Media) -> bool {
    match media {
        Media::Photo(_) => true,
        Media::Document(doc) => doc
            .mime_type()
            .map(|m| m.starts_with("image/"))
            .unwrap_or(false),
        _ => false,
    }
}
