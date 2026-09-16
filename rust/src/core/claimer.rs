//! Claim delivery and verification.
//!
//! Replies from the game bot are **not** polled for. The claimer registers a
//! wait slot (`oneshot`) for the relevant chat, and the listener fills it the
//! moment the reply actually arrives on the update stream. The result is zero
//! extra API calls and no polling interval burning time on the hot path.
//!
//! Settings are read from configuration **when a claim is sent**, not copied at
//! startup, so changes made in the web dashboard take effect immediately.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use grammers_client::message::InputMessage;
use grammers_client::Client;
use grammers_session::types::{PeerId, PeerRef};
use rand::RngExt;
use tokio::sync::oneshot;
use tracing::{info, warn};

use crate::config;
use crate::recognizer::CharacterInfo;

/// The outcome of a single claim attempt.
#[derive(Clone, Debug)]
pub struct ClaimResult {
    pub name: String,
    pub success: bool,
    pub response_text: Option<String>,
}

/// Klasifikasi teks balasan game bot.
/// `Some(true)` accepted, `Some(false)` rejected, `None` unrecognised.
pub fn detect_result(response_text: &str, success: &[String], fail: &[String]) -> Option<bool> {
    let lower = response_text.to_lowercase();
    if success.iter().any(|kw| lower.contains(kw.as_str())) {
        return Some(true);
    }
    if fail.iter().any(|kw| lower.contains(kw.as_str())) {
        return Some(false);
    }
    None
}

pub struct Claimer {
    /// Chats currently waiting on a reply from the game bot.
    pending: Mutex<HashMap<PeerId, oneshot::Sender<String>>>,
}

impl Claimer {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Register a wait slot for a chat before the claim is sent.
    fn register_pending(&self, peer: PeerId) -> oneshot::Receiver<String> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut map) = self.pending.lock() {
            map.insert(peer, tx);
        }
        rx
    }

    fn clear_pending(&self, peer: PeerId) {
        if let Ok(mut map) = self.pending.lock() {
            map.remove(&peer);
        }
    }

    /// Called by the listener when a message arrives from someone else.
    /// Returns `true` when the message was consumed as a claim reply.
    pub fn deliver_reply(&self, peer: PeerId, text: String) -> bool {
        let Ok(mut map) = self.pending.lock() else {
            return false;
        };
        match map.remove(&peer) {
            Some(tx) => {
                let _ = tx.send(text);
                true
            }
            None => false,
        }
    }

    /// Send the claim command and wait for the reply.
    pub async fn execute_claim(
        &self,
        client: &Client,
        peer: PeerId,
        peer_ref: PeerRef,
        character: &CharacterInfo,
        reply_to_msg_id: Option<i32>,
    ) -> ClaimResult {
        // Read fresh so dashboard changes take effect immediately.
        let cfg = config::get();

        // Initial delay. Defaults to 0 because any wait here directly
        // reduces the chance of winning the claim.
        let delay = if cfg.max_delay_seconds > 0.0 {
            let lo = cfg.min_delay_seconds.min(cfg.max_delay_seconds);
            let hi = cfg.min_delay_seconds.max(cfg.max_delay_seconds);
            if hi > 0.0 {
                let mut rng = rand::rng();
                rng.random_range(lo..=hi)
            } else {
                0.0
            }
        } else {
            0.0
        };

        if delay > 0.0 {
            info!("Waiting {delay:.2}s before claiming...");
            tokio::time::sleep(Duration::from_secs_f64(delay)).await;
        }

        let candidates = character.claim_names(&cfg.name_format);

        for (idx, name) in candidates.iter().enumerate() {
            let cmd = format!("{} {}", cfg.claim_command, name.trim());

            if idx > 0 {
                info!("Retry attempt {} with an alternate name...", idx + 1);
                tokio::time::sleep(Duration::from_secs_f64(0.8)).await;
            }

            // Register the wait slot BEFORE sending, so a very fast reply is
            // not missed.
            let rx = self.register_pending(peer);

            info!("Sending: '{cmd}'");
            let sent = client
                .send_message(
                    peer_ref,
                    InputMessage::new().text(cmd.clone()).reply_to(reply_to_msg_id),
                )
                .await;

            if let Err(e) = sent {
                warn!("Could not send '{cmd}': {e}");
                self.clear_pending(peer);
                continue;
            }

            info!("Waiting for the game bot reply ({}s timeout)...", cfg.verify_timeout_seconds);

            match tokio::time::timeout(Duration::from_secs_f64(cfg.verify_timeout_seconds), rx).await
            {
                Ok(Ok(response)) => {
                    info!("Game bot replied: \"{}\"", truncate(&response, 120));
                    match detect_result(&response, &cfg.success_keywords, &cfg.fail_keywords) {
                        Some(true) => {
                            info!("Claim succeeded! '{name}' was accepted.");
                            return ClaimResult {
                                name: name.clone(),
                                success: true,
                                response_text: Some(response),
                            };
                        }
                        Some(false) => {
                            warn!("Claim '{name}' was rejected. Trying the next name...");
                            continue;
                        }
                        None => {
                            info!("Unrecognised reply for '{name}'; treating as success.");
                            return ClaimResult {
                                name: name.clone(),
                                success: true,
                                response_text: Some(response),
                            };
                        }
                    }
                }
                // Timed out. Treat as success: the game bot does not always reply,
                // and waiting longer only wastes time.
                Err(_) => {
                    warn!(
                        "No reply within {}s for '{name}'. Treating as success.",
                        cfg.verify_timeout_seconds
                    );
                    self.clear_pending(peer);
                    return ClaimResult {
                        name: name.clone(),
                        success: true,
                        response_text: None,
                    };
                }
                Ok(Err(_)) => {
                    warn!("Wait slot dropped for '{name}'.");
                    return ClaimResult {
                        name: name.clone(),
                        success: false,
                        response_text: None,
                    };
                }
            }
        }

        warn!(
            "No name candidate could be claimed for '{}'.",
            character.full_name
        );
        ClaimResult {
            name: candidates.first().cloned().unwrap_or_default(),
            success: false,
            response_text: None,
        }
    }
}

impl Default for Claimer {
    fn default() -> Self {
        Self::new()
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect::<String>() + "..."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kw(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn mendeteksi_balasan_sukses() {
        let success = kw(&["now protected", "added to your harem"]);
        let fail = kw(&["not quite right", "already claimed"]);
        assert_eq!(
            detect_result("Kaguya is now protected!", &success, &fail),
            Some(true)
        );
    }

    #[test]
    fn mendeteksi_balasan_gagal() {
        let success = kw(&["now protected"]);
        let fail = kw(&["not quite right", "already claimed"]);
        assert_eq!(
            detect_result("Not quite right, try again", &success, &fail),
            Some(false)
        );
    }

    #[test]
    fn balasan_tidak_dikenali() {
        let success = kw(&["now protected"]);
        let fail = kw(&["not quite right"]);
        assert_eq!(detect_result("hmm menarik", &success, &fail), None);
    }

    #[test]
    fn slot_tunggu_dapat_diisi_dan_dibersihkan() {
        let claimer = Claimer::new();
        let peer = PeerId::user_unchecked(1);
        let rx = claimer.register_pending(peer);
        assert!(claimer.deliver_reply(peer, "now protected".into()));
        assert!(!claimer.deliver_reply(peer, "duplikat".into()));
        assert!(rx.blocking_recv().is_ok());
    }
}
