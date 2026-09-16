//! Google Lens recognizer.
//!
//! The last fallback, after IQDB, SauceNAO and Trace.moe. Because Google's
//! crawl results are unstructured, every candidate **must** pass AniList
//! verification before the bot is allowed to claim it.

use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use reqwest::multipart::{Form, Part};
use tracing::{debug, info, warn};

use super::anilist::lookup_series_from_character;
use super::base::{detect_mime, split_name, CharacterInfo};
use crate::http::COOKIE_CLIENT;

static RE_CALLBACK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)AF_initDataCallback\((\{.*?\})\)").expect("regex callback valid")
});
static RE_DATA_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)"data":\s*(\[.*?\])\s*[,}]"#).expect("regex data valid"));
static RE_QUOTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""([^"]{3,80})""#).expect("regex quoted valid"));
static RE_H3: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<h3[^>]*>([^<]+)</h3>").expect("regex h3 valid"));
static RE_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<title>([^<]+)</title>").expect("regex title valid"));
static RE_GOOGLE_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*-\s*Google(?:\s+Search)?\s*$").expect("regex suffix valid")
});
static RE_CAPITALIZED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""([A-Z][a-zA-Z\s]{3,40})""#).expect("regex capitalized valid")
});
static RE_FROM_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(.+?)\s+(?:from|in|of)\s+(.+)$").expect("regex from valid")
});
static RE_PAREN_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(.+?)\s*\(([^)]+)\)\s*$").expect("regex paren valid")
});

const LENS_BLACKLIST: &[&str] = &[
    "google", "google search", "google lens", "google images", "search",
    "bing", "yahoo", "yandex", "search result", "image result",
    "sign in", "login", "signup", "register", "submit", "button", "input",
    "captcha", "enablejs", "invalid component state", "invalid class name",
    "privacy", "terms", "settings", "feedback", "help", "overview",
    "loading", "error", "404", "menu", "navigate", "home", "about",
    "domcontentloaded", "customevent", "promise", "symbol", "constructor",
    "component", "renderer", "decorator", "event type", "function",
    "illustration", "artwork", "wallpaper", "fanart", "official art",
    "download", "pinterest", "twitter", "instagram", "deviantart",
    "pixiv", "zerochan", "danbooru", "gelbooru", "yande.re", "konachan",
    "resolution", "pixels", "image", "photo", "picture", "screenshot",
    "figure", "merchandise", "poster", "print", "acrylic", "standee",
    "cosplay", "costume", "wig", "outfit", "dress",
];

fn has_blacklisted(text_lower: &str) -> bool {
    LENS_BLACKLIST.iter().any(|bl| text_lower.contains(bl))
}

/// Detect HTML tag name artefacts.
///
/// Google embeds tag names as uppercase strings in the page data
/// (`"APPLET"`, `"AREA"`, `"EMBED"`, ...). These slipped through as character
/// candidates and were then "confirmed" by AniList as unrelated characters,
/// which produced wrong claims. Real character names are never all uppercase.
fn is_tag_artifact(candidate: &str) -> bool {
    let letters: Vec<char> = candidate.chars().filter(|c| c.is_alphabetic()).collect();
    !letters.is_empty()
        && letters.len() > 3
        && letters.iter().all(|c| c.is_uppercase())
}

/// Reject HTML tag artefacts and blacklisted terms.
fn is_rejected(candidate: &str) -> bool {
    has_blacklisted(&candidate.to_lowercase()) || is_tag_artifact(candidate)
}

/// Analyse Lens text snippets looking for a character name.
fn extract_character_from_lens_text(snippets: &[String]) -> (Option<String>, Option<String>) {
    for snippet in snippets {
        if snippet.chars().count() < 3 {
            continue;
        }

        let snippet_clean = snippet.trim();

        if is_rejected(snippet_clean) {
            continue;
        }

        // Pola: "X from Y"
        if let Some(caps) = RE_FROM_PATTERN.captures(snippet_clean) {
            let char_part = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            let series_part = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
            let word_count = char_part.split_whitespace().count();
            if (2..=4).contains(&word_count)
                && char_part.chars().count() <= 40
                && !is_rejected(char_part)
            {
                return (Some(char_part.to_string()), Some(series_part.to_string()));
            }
        }

        // Pola: "X (Y)"
        if let Some(caps) = RE_PAREN_PATTERN.captures(snippet_clean) {
            let char_part = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            let series_part = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
            let word_count = char_part.split_whitespace().count();
            if (1..=4).contains(&word_count)
                && char_part.chars().count() <= 40
                && !is_rejected(char_part)
            {
                return (Some(char_part.to_string()), Some(series_part.to_string()));
            }
        }

        // Standalone name candidate: 1-4 words, each starting with a capital.
        let words: Vec<&str> = snippet_clean.split_whitespace().collect();
        if (1..=4).contains(&words.len())
            && snippet_clean.chars().count() <= 40
            && words.iter().all(|w| {
                w.chars()
                    .next()
                    .map(|c| c.is_uppercase() || !c.is_alphabetic())
                    .unwrap_or(false)
            })
        {
            return (Some(snippet_clean.to_string()), None);
        }
    }

    (None, None)
}

/// Extract candidate text from a Google Lens HTML response.
fn parse_lens_response(html: &str) -> Vec<String> {
    let mut text_results: Vec<String> = Vec::new();

    // Jalur utama: blok JSON `AF_initDataCallback`.
    for caps in RE_CALLBACK.captures_iter(html) {
        let Some(cb) = caps.get(1) else { continue };
        let Some(data_caps) = RE_DATA_FIELD.captures(cb.as_str()) else {
            continue;
        };
        let Some(data) = data_caps.get(1) else { continue };
        for quoted in RE_QUOTED.captures_iter(data.as_str()) {
            if let Some(m) = quoted.get(1) {
                text_results.push(m.as_str().to_string());
            }
        }
    }

    for caps in RE_H3.captures_iter(html) {
        if let Some(m) = caps.get(1) {
            text_results.push(m.as_str().to_string());
        }
    }

    if let Some(caps) = RE_TITLE.captures(html)
        && let Some(m) = caps.get(1) {
            let title = RE_GOOGLE_SUFFIX.replace(m.as_str().trim(), "").trim().to_string();
            if !title.is_empty()
                && !matches!(
                    title.to_lowercase().as_str(),
                    "google" | "google search" | "google lens" | "search"
                )
            {
                text_results.push(title);
            }
        }

    for caps in RE_CAPITALIZED.captures_iter(html) {
        if let Some(m) = caps.get(1) {
            text_results.push(m.as_str().to_string());
        }
    }

    // Deduplikasi sambil menyaring blacklist.
    let mut seen: Vec<String> = Vec::new();
    let mut unique: Vec<String> = Vec::new();
    for t in text_results {
        let t_clean = t.trim().to_string();
        if t_clean.is_empty() || is_rejected(&t_clean) {
            continue;
        }
        if !seen.iter().any(|s| s == &t_clean) {
            seen.push(t_clean.clone());
            unique.push(t_clean);
        }
    }

    unique
}

/// Google Lens recognizer.
///
/// The enabled flag is read from configuration at search time, so toggling it
/// in the dashboard takes effect without a restart.
#[derive(Default)]
pub struct GoogleLensRecognizer;

impl GoogleLensRecognizer {
    pub async fn identify(&self, image_bytes: &[u8]) -> Option<CharacterInfo> {
        if !crate::config::get().lens_enabled {
            debug!("Google Lens recognizer dinonaktifkan, melewati...");
            return None;
        }

        let upload_endpoint = "https://lens.google.com/v3/upload";
        let mime = detect_mime(image_bytes);

        info!("Sending image to Google Lens for a visual search...");

        // Warm-up: pick up a session cookie from google.com before uploading.
        let _ = COOKIE_CLIENT
            .get("https://www.google.com/")
            .timeout(Duration::from_secs(8))
            .send()
            .await;

        let file_part = match Part::bytes(image_bytes.to_vec())
            .file_name("image.jpg")
            .mime_str(mime)
        {
            Ok(p) => p,
            Err(e) => {
                warn!("Google Lens multipart setup failed: {e}");
                return None;
            }
        };

        let form = Form::new()
            .part("encoded_image", file_part)
            .text("image_content", "")
            .text("re", "df")
            .text("s", "4")
            .text("st", "")
            .text("lp", "1");

        let resp = match COOKIE_CLIENT
            .post(upload_endpoint)
            .header("Referer", "https://lens.google.com/")
            .header("Origin", "https://lens.google.com")
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
            )
            .header("Accept-Language", "en-US,en;q=0.9")
            .timeout(Duration::from_secs(20))
            .multipart(form)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Google Lens request failed: {e}");
                return None;
            }
        };

        let status = resp.status();
        if !status.is_success() && status.as_u16() != 302 {
            warn!("Google Lens HTTP {}", status.as_u16());
            return None;
        }

        let html = match resp.text().await {
            Ok(h) => h,
            Err(e) => {
                warn!("Google Lens response could not be read: {e}");
                return None;
            }
        };

        if html.len() < 500 {
            warn!("Google Lens returned an empty or very short response.");
            return None;
        }

        let snippets = parse_lens_response(&html);
        if snippets.is_empty() {
            warn!("Google Lens found no relevant text in the response.");
            return None;
        }

        info!(
            "Google Lens menghasilkan {} kandidat teks: {:?}",
            snippets.len(),
            &snippets[..snippets.len().min(5)]
        );

        let (char_name, series_name) = extract_character_from_lens_text(&snippets);
        let char_name = match char_name {
            Some(c) => c,
            None => {
                warn!("Google Lens could not extract a candidate name from the visual text.");
                return None;
            }
        };

        // Verifikasi WAJIB lewat AniList.
        let (anilist_name, anilist_series) = lookup_series_from_character(&char_name).await;

        let confirmed = match anilist_name {
            Some(n) => n,
            None => {
                warn!(
                    "Google Lens menemukan teks '{char_name}', tetapi TIDAK terverifikasi di AniList. \
                     Menolak klaim untuk mencegah klaim istilah acak."
                );
                return None;
            }
        };

        info!("AniList confirmed '{char_name}' -> '{confirmed}' from {anilist_series:?}");

        let final_name = confirmed;
        let final_series = anilist_series.or(series_name);

        let (first_name, last_name) = split_name(&final_name);

        info!(
            "Google Lens berhasil: {final_name} (Seri: {})",
            final_series.as_deref().unwrap_or("Unknown")
        );

        Some(CharacterInfo {
            full_name: final_name,
            first_name,
            last_name,
            series: final_series,
            confidence: 0.60,
            source: "google_lens".to_string(),
            alternate_names: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blacklist_menyaring_istilah_umum() {
        assert!(has_blacklisted("google search"));
        assert!(has_blacklisted("wallpaper hd"));
        assert!(!has_blacklisted("houraisan kaguya"));
    }

    #[test]
    fn mengekstrak_pola_from() {
        let snippets = vec!["Kaguya Houraisan from Touhou Project".to_string()];
        let (c, s) = extract_character_from_lens_text(&snippets);
        assert_eq!(c.as_deref(), Some("Kaguya Houraisan"));
        assert_eq!(s.as_deref(), Some("Touhou Project"));
    }

    #[test]
    fn menolak_snippet_blacklist() {
        let snippets = vec!["Google Search".to_string()];
        let (c, _) = extract_character_from_lens_text(&snippets);
        assert_eq!(c, None);
    }

    #[test]
    fn menolak_artefak_nama_tag_html() {
        // A real case from `test_waifu.png`: these slipped through, were
        // "confirmed" by AniList, and produced a claim for 'Silfy Appleton'.
        for tag in ["APPLET", "AREA", "BASE", "COMMAND", "EMBED"] {
            assert!(is_tag_artifact(tag), "'{tag}' seharusnya disaring");
        }
    }

    #[test]
    fn nama_karakter_biasa_tidak_dianggap_artefak() {
        for name in ["Houraisan Kaguya", "Silfy Appleton", "Kaguya", "Rem"] {
            assert!(!is_tag_artifact(name), "'{name}' tidak boleh disaring");
        }
    }

    #[test]
    fn artefak_tag_tidak_lolos_sebagai_kandidat() {
        let snippets: Vec<String> = ["APPLET", "EMBED", "AREA"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (c, _) = extract_character_from_lens_text(&snippets);
        assert_eq!(c, None, "artefak tag HTML tidak boleh jadi kandidat");
    }
}
