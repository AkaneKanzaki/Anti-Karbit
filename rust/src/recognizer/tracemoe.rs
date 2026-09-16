//! Trace.moe recognizer.
//!
//! Finds the source anime scene, then resolves a character name through AniList
//! so the bot never claims an anime title as if it were a character.

use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde_json::Value;
use tracing::{info, warn};

use super::anilist::lookup_characters_by_media_id;
use super::base::{detect_mime, split_name, CharacterInfo};
use crate::http::CLIENT;

static RE_TV_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\(TV\)\s*").expect("regex TV valid"));

/// Trace.moe recognizer.
///
/// The similarity threshold is read from configuration at search time, so
/// dashboard changes take effect without a restart.
#[derive(Default)]
pub struct TraceMoeRecognizer;

impl TraceMoeRecognizer {
    pub async fn identify(&self, image_bytes: &[u8]) -> Option<CharacterInfo> {
        let min_similarity = crate::config::get().tracemoe_min_similarity;
        let endpoint = "https://api.trace.moe/search?anilistInfo=1";
        let mime = detect_mime(image_bytes);

        info!("Sending image to Trace.moe (anime scene search)...");

        let resp = CLIENT
            .post(endpoint)
            .header("Content-Type", mime)
            .timeout(Duration::from_secs(15))
            .body(image_bytes.to_vec())
            .send()
            .await;

        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                warn!("Trace.moe request failed: {e}");
                return None;
            }
        };

        if !resp.status().is_success() {
            warn!("Trace.moe HTTP {}", resp.status());
            return None;
        }

        let data: Value = match resp.json().await {
            Ok(d) => d,
            Err(e) => {
                warn!("Trace.moe response was not JSON: {e}");
                return None;
            }
        };

        let top = match data.get("result").and_then(|r| r.as_array()).and_then(|a| a.first()) {
            Some(t) => t,
            None => {
                warn!("Trace.moe found no matching anime scene.");
                return None;
            }
        };

        let similarity = top.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if similarity < min_similarity {
            info!(
                "Trace.moe match ({:.1}%) is below the {:.0}% threshold.",
                similarity * 100.0,
                min_similarity * 100.0
            );
            return None;
        }

        let anilist = top.get("anilist");
        let media_id = anilist
            .and_then(|a| a.get("id"))
            .and_then(|v| v.as_i64());

        let series_name = anilist
            .and_then(|a| a.get("title"))
            .and_then(|t| {
                t.get("romaji")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .or_else(|| t.get("english").and_then(|v| v.as_str()).filter(|s| !s.is_empty()))
                    .or_else(|| t.get("native").and_then(|v| v.as_str()).filter(|s| !s.is_empty()))
            })
            .unwrap_or("Unknown Anime")
            .to_string();

        let episode = top.get("episode").and_then(|v| v.as_i64());
        let clean_series = RE_TV_SUFFIX.replace_all(&series_name, "").trim().to_string();

        info!(
            "Trace.moe menemukan adegan: '{clean_series}' (Episode: {episode:?}, Kemiripan: {:.1}%)",
            similarity * 100.0
        );

        // Resolve the character through AniList.
        let char_candidates: Vec<String> = match media_id {
            Some(id) => {
                info!("Looking up characters for '{clean_series}' (ID: {id})...");
                let (_, chars) = lookup_characters_by_media_id(id).await;
                if !chars.is_empty() {
                    info!("AniList returned {} characters: {:?}", chars.len(), &chars[..chars.len().min(3)]);
                }
                chars
            }
            None => Vec::new(),
        };

        if char_candidates.is_empty() {
            warn!(
                "Anime scene '{clean_series}' was found, but no specific character could be confirmed. \
                 Skipping the claim to avoid sending an anime title as a name."
            );
            return None;
        }

        let primary = char_candidates[0].clone();
        let alternates: Vec<String> = char_candidates[1..].to_vec();
        let (first_name, last_name) = split_name(&primary);

        info!("Trace.moe primary character: '{primary}'");
        if !alternates.is_empty() {
            info!("Alternate candidates: {alternates:?}");
        }

        Some(CharacterInfo {
            full_name: primary,
            first_name,
            last_name,
            series: Some(clean_series),
            confidence: similarity,
            source: "trace_moe".to_string(),
            alternate_names: alternates,
        })
    }
}
