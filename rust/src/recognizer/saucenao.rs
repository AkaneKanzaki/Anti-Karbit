//! SauceNAO recognizer.
//!
//! Strong on Pixiv, Twitter/X and Danbooru illustrations and anime/VTuber
//! fanart. Requires a free API key.

use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use reqwest::multipart::{Form, Part};
use serde_json::Value;
use tracing::{debug, info, warn};

use super::anilist::lookup_series_from_character;
use super::base::{clean_text, detect_mime, split_name, CharacterInfo};
use crate::http::CLIENT;

static RE_PAREN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(.*?\)").expect("regex parenthetical valid"));

/// SauceNAO recognizer.
///
/// The API key and similarity threshold are read from configuration at search
/// time, so dashboard changes take effect without a restart.
#[derive(Default)]
pub struct SauceNaoRecognizer;

impl SauceNaoRecognizer {
    pub async fn identify(&self, image_bytes: &[u8]) -> Option<CharacterInfo> {
        let cfg = crate::config::get();
        let endpoint = "https://saucenao.com/search.php";
        let api_key = cfg.saucenao_api_key;
        let min_similarity = cfg.saucenao_min_similarity;

        if api_key.is_empty() {
            debug!("SauceNAO API Key tidak diisi, melewati pencarian SauceNAO.");
            return None;
        }

        let mime = detect_mime(image_bytes);

        let file_part = Part::bytes(image_bytes.to_vec())
            .file_name("waifu.jpg")
            .mime_str(mime)
            .ok()?;

        let form = Form::new()
            .part("file", file_part)
            .text("output_type", "2")
            .text("numres", "5")
            .text("api_key", api_key);

        info!("Sending image to SauceNAO...");

        let resp = match CLIENT
            .post(endpoint)
            .timeout(Duration::from_secs(15))
            .multipart(form)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("SauceNAO request failed: {e}");
                return None;
            }
        };

        if !resp.status().is_success() {
            warn!("SauceNAO HTTP status {}", resp.status());
            return None;
        }

        let json: Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                warn!("SauceNAO response was not JSON: {e}");
                return None;
            }
        };

        let results = match json.get("results").and_then(|r| r.as_array()) {
            Some(r) if !r.is_empty() => r,
            _ => {
                warn!("SauceNAO found no matching images.");
                return None;
            }
        };

        let mut best_char: Option<String> = None;
        let mut best_series: Option<String> = None;
        let mut best_sim = 0.0_f64;

        for r in results {
            // SauceNAO sends `similarity` as a string, not a number.
            let sim = r
                .get("header")
                .and_then(|h| h.get("similarity"))
                .and_then(|v| match v {
                    Value::String(s) => s.trim().parse::<f64>().ok(),
                    Value::Number(n) => n.as_f64(),
                    _ => None,
                })
                .unwrap_or(0.0);

            if sim < min_similarity {
                continue;
            }

            let data_block = match r.get("data") {
                Some(d) => d,
                None => continue,
            };

            let raw_char = pick_str(data_block, &["characters", "character", "eng_name", "jp_name"]);
            let raw_series = pick_str(data_block, &["material", "source", "title"]);

            let mut char_name: Option<String> = None;
            if let Some(raw) = &raw_char {
                let first_c = raw.split(',').next().unwrap_or("").trim();
                let stripped = RE_PAREN.replace_all(first_c, "");
                let stripped = stripped.trim();
                if !stripped.is_empty() {
                    char_name = Some(clean_text(stripped));
                }
            }

            // Fallback: a short Pixiv title sometimes contains the character name.
            if char_name.is_none() && raw_series.is_some() && sim >= 80.0
                && let Some(title) = data_block.get("title").and_then(|v| v.as_str()) {
                    let lower = title.to_lowercase();
                    let is_chapterish = ["chapter", "ep", "vol"].iter().any(|w| lower.contains(w));
                    if !title.is_empty() && title.split_whitespace().count() <= 3 && !is_chapterish {
                        char_name = Some(clean_text(title));
                    }
                }

            if let Some(name) = char_name
                && sim > best_sim {
                    best_sim = sim;
                    best_char = Some(name);
                    if let Some(series) = &raw_series {
                        let first_s = series.split(',').next().unwrap_or("").trim();
                        best_series = Some(clean_text(first_s));
                    }
                }
        }

        let best_char = match best_char {
            Some(c) => c,
            None => {
                warn!("SauceNAO found a match but no usable character metadata.");
                return None;
            }
        };

        if best_series.is_none() {
            let (_, anilist_series) = lookup_series_from_character(&best_char).await;
            if anilist_series.is_some() {
                best_series = anilist_series;
            }
        }

        let (first_name, last_name) = split_name(&best_char);

        info!(
            "SauceNAO menemukan: {best_char} (Seri: {}, Sim: {best_sim}%)",
            best_series.as_deref().unwrap_or("Unknown")
        );

        Some(CharacterInfo {
            full_name: best_char,
            first_name,
            last_name,
            series: best_series,
            confidence: best_sim / 100.0,
            source: "saucenao".to_string(),
            alternate_names: Vec::new(),
        })
    }
}

/// Return the first present, non-empty string from a list of keys.
fn pick_str(obj: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(s) = obj.get(*key).and_then(|v| v.as_str())
            && !s.trim().is_empty() {
                return Some(s.to_string());
            }
    }
    None
}
