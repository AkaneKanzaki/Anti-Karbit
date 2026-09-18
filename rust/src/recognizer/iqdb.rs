//! IQDB recognizer.
//!
//! IQDB scans anime booru databases (Danbooru, Gelbooru, Konachan, yande.re,
//! Anime-Pictures). Free and requires no API key.
//!
//! HTML is extracted with a DOM parser (`scraper`) rather than regex over the
//! whole document. The outcome is the same, but far more resilient to changes
//! in IQDB's markup.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use reqwest::multipart::{Form, Part};
use scraper::{Html, Selector};
use tracing::{info, warn};

use super::anilist::lookup_series_from_character;
use super::base::{clean_text, detect_mime, split_name, CharacterInfo};
use crate::http::CLIENT;

static RE_SIMILARITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\d+)%\s*similarity").expect("regex similarity valid")
});
static RE_PARENTHETICAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(([^)]+)\)").expect("regex parenthetical valid"));
static RE_STRIP_PARENTHETICAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\([^)]*\)").expect("regex strip valid"));
static RE_RATING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)Rating:\s*[a-z]\s*").expect("regex rating valid"));
static RE_SCORE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)Score:\s*\d+\s*").expect("regex score valid"));
static RE_TAGS_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*Tags:\s*").expect("regex tags valid"));

// ---------------------------------------------------------------------------
// Kamus tag booru
// ---------------------------------------------------------------------------

const GENERIC_TAGS: &[&str] = &[
    "1girl", "2girls", "3girls", "4girls", "5girls", "6+girls", "multiple_girls",
    "1boy", "2boys", "3boys", "multiple_boys", "solo", "highres", "absurdres", "incredibly_absurdres",
    "long_hair", "short_hair", "medium_hair", "very_long_hair",
    "black_hair", "brown_hair", "blonde_hair", "blue_hair", "pink_hair", "purple_hair",
    "red_hair", "white_hair", "green_hair", "silver_hair", "grey_hair", "orange_hair",
    "two-tone_hair", "streaked_hair", "multicolored_hair", "gradient_hair",
    "blue_eyes", "red_eyes", "brown_eyes", "green_eyes", "pink_eyes", "purple_eyes",
    "yellow_eyes", "amber_eyes", "grey_eyes", "heterochromia",
    "blush", "smile", "open_mouth", "closed_eyes", "looking_at_viewer", "looking_away",
    "simple_background", "white_background", "transparent_background", "grey_background",
    "black_background",
    "school_uniform", "sailor_uniform", "sailor_suit", "skirt", "dress", "thighhighs", "kneehighs",
    "gloves", "hair_ornament", "hair_ribbon", "hair_flower", "ribbon", "bow",
    "official_art", "fanart", "original", "rating", "score", "tags", "pixiv", "danbooru",
    "gelbooru", "konachan", "yande.re", "anime-pictures", "zerochan",
    "japanese_clothes", "kimono", "yukata", "maid", "pantyhose", "female", "male", "ecchi",
    "uniform", "twin_tails", "twintails", "ponytail", "braid", "side_ponytail", "french_braid",
    "ahoge", "animal_ears", "cat_ears", "dog_ears", "fox_ears", "bunny_ears", "wolf_ears",
    "barefoot", "cleavage", "swimsuit", "bikini", "monochrome", "comic", "parody",
    "translated", "western", "korean", "crying", "tears", "food", "drink", "weapon",
    "sword", "gun", "wings", "halo", "horns", "tail", "jewelry", "necklace", "earrings",
    "glasses", "sunglasses", "hat", "cap", "beret", "hood", "jacket", "coat", "sweater",
    "cardigan", "shirt", "collarbone", "navel", "bare_shoulders", "sleeveless",
    "no_character", "no_people", "flower", "water", "moon", "magic", "night", "sky",
    "clouds", "stars", "tree", "plant", "scenery", "instrument", "violin", "piano",
    "img", "image", "preview", "thumbnail", "photo", "picture", "safe", "questionable", "explicit",
    "signed", "single", "reflection", "feathers", "traditional_clothes", "full_moon",
    "alcd", "girl", "boy", "wink", "petals", "aliasing", "anthropomorphism",
    "headband", "headphones", "zettai_ryouiki", "hairpin",
    "profile", "upper_body", "lower_body", "full_body", "portrait", "close-up", "cowboy_shot",
    "sitting", "standing", "lying", "kneeling", "leaning_forward",
    "holding", "arms_behind_back", "arms_up", "hand_on_hip", "peace_sign", "v",
    "cleavage_cutout", "bare_arms", "bare_legs", "midriff", "sideboob", "underboob",
];

const THEMES_AND_COSTUMES: &[&str] = &[
    "maid_bikini", "maid_uniform", "bunny_girl", "bunny_suit", "reverse_bunny_suit",
    "school_swimsuit", "competition_swimsuit", "one-piece_swimsuit", "micro_bikini",
    "gothic_lolita", "sweet_lolita", "lolita_fashion", "cheerleader", "racing_queen",
    "miko", "nurse", "office_lady", "police", "soldier", "military_uniform",
    "wedding_dress", "bridal_gauntlets", "santa_costume", "christmas", "halloween_costume",
    "halloween", "cyberpunk", "steampunk", "mecha_musume", "monster_girl",
    "magical_girl", "armor", "power_armor", "tracksuit", "gym_uniform", "bloomers",
    "hoodie", "oversized_clothes", "catgirl", "foxgirl", "wolfgirl", "cowgirl",
    "original_character", "fan_character", "chibi", "alternate_costume",
    "alternate_hairstyle", "costume_switch", "clothes_lift", "skirt_lift",
    "pantyshot", "upskirt", "underbust", "crossover", "gender_bend", "gender_swap",
    "bad_id", "bad_link", "bad_pixiv_id", "copyright_request", "artist_request",
    "virtual_youtuber", "vtuber", "indie_vtuber", "envtuber", "jpvtuber", "idvtuber",
    // `voicevox` and `vocaloid` live in KNOWN_SERIES_TAGS instead: they name a
    // franchise, and classifying them as a costume threw the series away.
    "utaite",
];

const KNOWN_SERIES_TAGS: &[&str] = &[
    "hololive", "hololive_production", "hololive_english", "hololive_indonesia",
    "hololive_gamers", "hololive_fantasy", "hololive_dev_is", "holox",
    "hololive_0th_gen", "hololive_1st_gen", "hololive_2nd_gen", "hololive_3rd_gen",
    "hololive_4th_gen", "hololive_5th_gen", "holostars", "holostars_english",
    "nijisanji", "nijisanji_en", "nijisanji_id", "nijisanji_kr", "virtuareal",
    "vshojo", "vspo", "vspo!", "774inc", "brave_group", "phase_connect",
    "neo-porte", "idol_corporation", "idol_corp", "noripro", "kawaii_production",
    "production_kawaii", "wactor", "aogiri_high_school", "upd8",
    "genshin_impact", "honkai_impact", "honkai_impact_3rd", "houkai_gakuen_2",
    "honkai_star_rail", "zenless_zone_zero", "wuthering_waves",
    "blue_archive", "arknights", "azur_lane", "girls_frontline", "girls_frontline_2",
    "nikke", "goddess_of_victory:_nikke", "nikke:_goddess_of_victory",
    "fate/grand_order", "fate_grand_order", "fate/stay_night", "fate_stay_night",
    "fate/extra", "fate/apocrypha", "fate_series", "type-moon", "typemoon", "tsukihime",
    "touhou", "touhou_project", "kantai_collection", "kancolle",
    "the_idolm@ster", "idolmaster", "idolmaster_cinderella_girls", "idolmaster_million_live",
    "idolmaster_shiny_colors", "idolmaster_side_m", "gakuen_idolmaster",
    "love_live!", "love_live", "love_live!_sunshine!!", "love_live!_nijigasaki_high_school_idol_club",
    "love_live!_superstar!!", "love_live!_hasunosora_jogakuin_school_idol_club",
    "bang_dream!", "bang_dream", "bandori", "project_sekai", "project_sekai_colorful_stage!",
    "d4dj", "uma_musume", "uma_musume_pretty_derby", "granblue_fantasy",
    "princess_connect!", "princess_connect!_re:dive", "alchemy_stars",
    "re:zero_kara_hajimeru_isekai_seikatsu", "re:zero", "sword_art_online",
    "danganronpa", "tokyo_kushu", "tokyo_ghoul", "atelier_series", "atelier_live",
    "league_of_legends", "riot_games", "valorant", "overwatch", "pokemon",
    "fire_emblem", "honkai", "atelier", "dungeon_meshi", "frieren",
    "sousou_no_frieren", "bocchi_the_rock!", "bocchi_the_rock", "oshi_no_ko",
    "chainsaw_man", "jujutsu_kaisen", "spy_x_family", "lycoris_recoil",
    "neon_genesis_evangelion", "evangelion", "dragon_ball", "naruto",
    "one_piece", "bleach", "fairy_tail", "attack_on_titan",
    // Vocaloid/UTAU and friends are franchises, not costumes. They used to sit
    // in THEMES_AND_COSTUMES, which meant `hatsune_miku_(vocaloid)` had its
    // series silently thrown away by the `is_theme` check in the caller and
    // showed up as "Unknown" on the dashboard.
    //
    // Only the *franchise* names go here. A character name (for example
    // `hatsune_miku`) must NOT: `is_series_tag` runs before the character
    // branch, so listing it would swallow the character tag itself.
    "vocaloid", "vocaloid_2", "project_diva",
    "utau", "voicevox", "voiceroid", "cevio", "synthesizer_v",
    "shingeki_no_kyojin", "fullmetal_alchemist", "hunter_x_hunter", "demon_slayer",
    "kimetsu_no_yaiba", "my_hero_academia", "boku_no_hero_academia",
];

const KNOWN_SERIES_WORDS: &[&str] = &[
    "hololive", "nijisanji", "holostars", "vshojo", "vspo", "noripro",
    "genshin", "honkai", "arknights", "kancolle", "touhou",
    "idolmaster", "lovelive", "bandori", "sekai", "granblue",
    "pokemon", "vtuber", "phase", "connect",
];

const SERIES_ALIASES: &[(&str, &str)] = &[
    ("hololive", "Hololive"),
    ("hololive_production", "Hololive Production"),
    ("hololive_english", "Hololive English"),
    ("hololive_indonesia", "Hololive Indonesia"),
    ("hololive_gamers", "Hololive Gamers"),
    ("hololive_fantasy", "Hololive Fantasy"),
    ("hololive_dev_is", "Hololive DEV_IS"),
    ("holox", "holoX"),
    ("holostars", "HOLOSTARS"),
    ("nijisanji", "Nijisanji"),
    ("nijisanji_en", "Nijisanji EN"),
    ("vshojo", "VShojo"),
    ("vspo", "VSPO!"),
    ("vspo!", "VSPO!"),
    ("phase_connect", "Phase Connect"),
    ("neo-porte", "Neo-Porte"),
    ("idol_corporation", "Idol Corp"),
    ("idol_corp", "Idol Corp"),
    ("noripro", "NoriPro"),
    ("touhou", "Touhou Project"),
    ("touhou_project", "Touhou Project"),
    ("vocaloid", "Vocaloid"),
    ("vocaloid_2", "Vocaloid 2"),
    ("project_diva", "Project DIVA"),
    ("voicevox", "VOICEVOX"),
    ("utau", "UTAU"),
    ("voiceroid", "Voiceroid"),
    ("cevio", "CeVIO"),
    ("synthesizer_v", "Synthesizer V"),
    ("kancolle", "Kantai Collection"),
    ("kantai_collection", "Kantai Collection"),
    ("re:zero_kara_hajimeru_isekai_seikatsu", "Re:Zero"),
    ("re:zero", "Re:Zero"),
    ("idolmaster", "The iDOLM@STER"),
    ("the_idolm@ster", "The iDOLM@STER"),
    ("idolmaster_cinderella_girls", "The iDOLM@STER Cinderella Girls"),
    ("idolmaster_shiny_colors", "The iDOLM@STER Shiny Colors"),
    ("gakuen_idolmaster", "Gakuen Idolmaster"),
    ("genshin_impact", "Genshin Impact"),
    ("honkai_impact", "Honkai Impact 3rd"),
    ("honkai_impact_3rd", "Honkai Impact 3rd"),
    ("houkai_gakuen_2", "Honkai Impact 3rd"),
    ("honkai_star_rail", "Honkai: Star Rail"),
    ("zenless_zone_zero", "Zenless Zone Zero"),
    ("wuthering_waves", "Wuthering Waves"),
    ("azur_lane", "Azur Lane"),
    ("blue_archive", "Blue Archive"),
    ("arknights", "Arknights"),
    ("nikke", "Goddess of Victory: Nikke"),
    ("nikke:_goddess_of_victory", "Goddess of Victory: Nikke"),
    ("goddess_of_victory:_nikke", "Goddess of Victory: Nikke"),
    ("league_of_legends", "League of Legends"),
    ("riot_games", "League of Legends"),
    ("tokyo_kushu", "Tokyo Ghoul"),
    ("tokyo_ghoul", "Tokyo Ghoul"),
    ("sword_art_online", "Sword Art Online"),
    ("danganronpa", "Danganronpa"),
    ("love_live!", "Love Live!"),
    ("love_live", "Love Live!"),
    ("fate/grand_order", "Fate/Grand Order"),
    ("fate_grand_order", "Fate/Grand Order"),
    ("fate/stay_night", "Fate/Stay Night"),
    ("fate_stay_night", "Fate/Stay Night"),
    ("fate_series", "Fate Series"),
    ("type-moon", "TYPE-MOON"),
    ("typemoon", "TYPE-MOON"),
    ("bang_dream!", "BanG Dream!"),
    ("bang_dream", "BanG Dream!"),
    ("bandori", "BanG Dream!"),
    ("project_sekai", "Project SEKAI"),
    ("project_sekai_colorful_stage!", "Project SEKAI"),
    ("uma_musume", "Uma Musume"),
    ("uma_musume_pretty_derby", "Uma Musume Pretty Derby"),
    ("granblue_fantasy", "Granblue Fantasy"),
    ("princess_connect!", "Princess Connect!"),
    ("princess_connect!_re:dive", "Princess Connect! Re:Dive"),
    ("sousou_no_frieren", "Sousou no Frieren"),
    ("frieren", "Sousou no Frieren"),
    ("bocchi_the_rock!", "Bocchi the Rock!"),
    ("bocchi_the_rock", "Bocchi the Rock!"),
    ("oshi_no_ko", "Oshi no Ko"),
    ("lycoris_recoil", "Lycoris Recoil"),
    ("dungeon_meshi", "Dungeon Meshi"),
];

const ARTIST_BLACKLIST: &[&str] = &[
    "pixiv", "circle", "artist", "doujin", "cosplay", "vtuber", "twitter",
    "fanart", "official_art", "cover", "album", "sketch",
    "kantoku", "hiten", "morikura_en", "fukahire", "mika_pikazo", "anmi",
    "tony_taka", "tiv", "redrop", "wada_arco", "takeuchi_takashi",
    "asanagi", "shindol", "canno", "namie", "lack", "ryota-h", "kuroboshi_kouhaku",
];

static SET_GENERIC: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| GENERIC_TAGS.iter().copied().collect());
static SET_THEMES: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| THEMES_AND_COSTUMES.iter().copied().collect());
static SET_SERIES_TAGS: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| KNOWN_SERIES_TAGS.iter().copied().collect());
static SET_SERIES_WORDS: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| KNOWN_SERIES_WORDS.iter().copied().collect());
static SET_ARTISTS: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| ARTIST_BLACKLIST.iter().copied().collect());
static MAP_ALIASES: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| SERIES_ALIASES.iter().copied().collect());

fn is_generic(norm: &str) -> bool {
    SET_GENERIC.contains(norm)
}
fn is_theme(norm: &str) -> bool {
    SET_THEMES.contains(norm)
}
fn is_artist(norm: &str) -> bool {
    SET_ARTISTS.contains(norm)
}

/// Whether this tag refers to a series or group rather than a character.
fn is_series_tag(norm: &str) -> bool {
    if SET_SERIES_TAGS.contains(norm) {
        return true;
    }
    for word in norm.replace('-', "_").split('_') {
        if word.chars().count() > 4 && SET_SERIES_WORDS.contains(word) {
            return true;
        }
    }
    false
}

fn alias_or_clean(norm: &str, original: &str) -> String {
    MAP_ALIASES
        .get(norm)
        .map(|s| s.to_string())
        .unwrap_or_else(|| clean_text(original))
}

/// Parse a tag shaped like `name_(qualifier)_(series)` or `name_(series)`.
fn parse_parentheticals(tag: &str) -> (Option<String>, Option<String>) {
    let parentheticals: Vec<String> = RE_PARENTHETICAL
        .captures_iter(tag)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect();

    if parentheticals.is_empty() {
        return (None, None);
    }

    let char_part = RE_STRIP_PARENTHETICAL.replace_all(tag, "").trim().to_string();
    let char_norm = char_part.to_lowercase().replace(' ', "_");

    if char_part.is_empty() || is_generic(&char_norm) || is_theme(&char_norm) {
        return (None, None);
    }

    let mut series_part: Option<String> = None;
    for paren in parentheticals.iter().rev() {
        let paren_norm = paren.to_lowercase().replace(' ', "_");

        // A recognised series tag or a known alias is always the series.
        if SET_SERIES_TAGS.contains(paren_norm.as_str())
            || MAP_ALIASES.contains_key(paren_norm.as_str())
        {
            series_part = Some(paren.clone());
            break;
        }

        // Otherwise a single word is a physical qualifier, not a series.
        //
        // `inugami_korone_(dog)` and `hatsune_miku_(plants)` are common booru
        // shapes where the parenthetical describes the *picture* (the animal,
        // the scenery), not a franchise. Treating it as a series produced a
        // nonsense series like "Dog" or "Plants" on the dashboard. A series is
        // overwhelmingly multi-word or already in the known-tag list, so
        // requiring >1 word loses almost nothing and stops the false positives.
        //
        // The `(dog)_(hololive)` case still resolves: the loop runs right to
        // left, so `hololive` is seen first and breaks out.
        if !paren.contains(' ')
            && !paren.contains('_')
            && !SET_SERIES_WORDS.contains(paren_norm.as_str())
        {
            continue;
        }

        if paren.split_whitespace().count() <= 3
            && !is_artist(&paren_norm)
            && !is_generic(&paren_norm)
        {
            series_part = Some(paren.clone());
        }
    }

    (Some(char_part), series_part)
}

/// Analyse a booru tag string and extract `(character, series)`.
pub fn extract_character_from_tags(raw_tags_str: &str) -> (Option<String>, Option<String>) {
    let trimmed = raw_tags_str.trim();
    if trimmed.is_empty() || matches!(trimmed, "[IMG]" | "[IMAGE]" | "IMG" | "icon") {
        return (None, None);
    }

    let clean = RE_RATING.replace_all(trimmed, "");
    let clean = RE_SCORE.replace_all(&clean, "");
    let clean = RE_TAGS_PREFIX.replace(&clean, "").to_string();

    // Tags are comma separated (Anime-Pictures/Zerochan) or space separated
    // (Konachan/yande.re).
    let tags: Vec<String> = if clean.contains(',') {
        clean.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
    } else {
        clean.split_whitespace().map(|t| t.to_string()).collect()
    };

    // (name, series, priority)
    let mut char_candidates: Vec<(String, Option<String>, i32)> = Vec::new();
    let mut detected_series: Option<String> = None;

    for raw_tag in tags {
        let t = RE_TAGS_PREFIX.replace(raw_tag.trim(), "").trim().to_string();
        if t.starts_with('[') && t.ends_with(']') {
            continue;
        }

        let norm = t
            .to_lowercase()
            .replace(' ', "_")
            .trim_matches(|c| "[]().".contains(c))
            .to_string();

        if norm.chars().count() <= 2 || is_generic(&norm) || is_theme(&norm) || is_artist(&norm) {
            continue;
        }

        // --- Standalone series or VTuber group name ---
        if is_series_tag(&norm) {
            let series_cand = alias_or_clean(&norm, &t);
            if detected_series.as_ref().is_none_or(|d| series_cand.len() > d.len()) {
                detected_series = Some(series_cand);
            }
            continue;
        }

        // --- Format paling akurat: char_name_(series) ---
        if t.contains('(') {
            let (char_part, series_part) = parse_parentheticals(&t);
            if let Some(char_part) = char_part {
                let char_norm = char_part
                    .to_lowercase()
                    .replace(' ', "_")
                    .trim_matches('_')
                    .to_string();

                if !is_generic(&char_norm)
                    && !is_theme(&char_norm)
                    && !is_series_tag(&char_norm)
                    && !is_artist(&char_norm)
                    && char_norm.chars().count() > 2
                {
                    let series_display = match &series_part {
                        Some(sp) => {
                            let series_norm = sp.to_lowercase().replace(' ', "_");
                            if is_artist(&series_norm) || is_theme(&series_norm) {
                                detected_series.clone()
                            } else {
                                Some(alias_or_clean(&series_norm, sp))
                            }
                        }
                        None => detected_series.clone(),
                    };
                    char_candidates.push((clean_text(&char_part), series_display, 10));
                }
            }
            continue;
        }

        // --- Two-or-more word tag that is not generic, a theme, or a group ---
        let parts: Vec<String> = t.replace('_', " ").split_whitespace().map(|s| s.to_string()).collect();
        if parts.len() >= 2 {
            let part_norms: Vec<String> = parts
                .iter()
                .map(|p| p.to_lowercase().replace(' ', "_"))
                .collect();

            if part_norms.iter().any(|p| is_generic(p)) {
                continue;
            }
            if part_norms.iter().any(|p| is_artist(p)) {
                continue;
            }
            if is_series_tag(&norm) || is_theme(&norm) {
                continue;
            }
            // A tag containing a series word is recorded as a series, not a character.
            if parts
                .iter()
                .any(|p| p.chars().count() > 4 && SET_SERIES_WORDS.contains(p.to_lowercase().as_str()))
            {
                let series_cand = alias_or_clean(&norm, &t);
                if detected_series.as_ref().is_none_or(|d| series_cand.len() > d.len()) {
                    detected_series = Some(series_cand);
                }
                continue;
            }

            char_candidates.push((clean_text(&t), None, 5));
            continue;
        }

        // --- Kata tunggal spesifik (prioritas rendah) ---
        if !is_generic(&norm)
            && !is_theme(&norm)
            && !is_series_tag(&norm)
            && !is_artist(&norm)
            && norm.chars().count() > 3
        {
            char_candidates.push((clean_text(&t), None, 1));
        }
    }

    if char_candidates.is_empty() {
        return (None, detected_series);
    }

    // Sort stabil menurun berdasarkan prioritas.
    // Stable descending sort, so equal priorities keep insertion order.
    char_candidates.sort_by_key(|a| std::cmp::Reverse(a.2));
    let (best_char, mut best_series, _) = char_candidates.remove(0);

    if best_series.is_none() && detected_series.is_some() {
        best_series = detected_series;
    }

    (Some(best_char), best_series)
}

// ---------------------------------------------------------------------------
// Recognizer
// ---------------------------------------------------------------------------

/// Identifies anime characters through the IQDB image search engine
/// (<https://iqdb.org>), which scans anime booru databases (Danbooru,
/// Gelbooru, Konachan, yande.re, Anime-Pictures).
///
/// The similarity threshold is read from configuration **at scan time**, not
/// copied at startup, so dashboard changes take effect immediately.
#[derive(Default)]
pub struct IqdbRecognizer;

impl IqdbRecognizer {
    pub async fn identify(&self, image_bytes: &[u8]) -> Option<CharacterInfo> {
        let min_similarity = crate::config::get().iqdb_min_similarity;
        let endpoint = "https://iqdb.org/";
        let mime = detect_mime(image_bytes);

        let file_part = Part::bytes(image_bytes.to_vec())
            .file_name("waifu.jpg")
            .mime_str(mime)
            .ok()?;
        let form = Form::new().part("file", file_part);

        info!("Sending image to IQDB...");

        let resp = match CLIENT
            .post(endpoint)
            .timeout(Duration::from_secs(15))
            .multipart(form)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("IQDB request failed: {e}");
                return None;
            }
        };

        if !resp.status().is_success() {
            warn!("IQDB returned HTTP {}", resp.status());
            return None;
        }

        let html = match resp.text().await {
            Ok(h) => h,
            Err(e) => {
                warn!("IQDB response could not be read: {e}");
                return None;
            }
        };

        // The DOM parser is confined to its own block: `scraper::Html` is not
        // Send, and holding it across an `await` would make this future
        // non-Send, which matters because it runs under tokio::spawn.
        let (best_char, mut best_series, best_sim, found_table) = {
            let doc = Html::parse_document(&html);
            let table_sel = Selector::parse("table").expect("selector table valid");
            let img_sel = Selector::parse("img[alt]").expect("selector img valid");

            let mut best_char: Option<String> = None;
            let mut best_series: Option<String> = None;
            let mut best_sim = 0.0_f64;
            let mut found_table = false;

        for table in doc.select(&table_sel) {
            found_table = true;
            let table_html = table.html();
            let lower = table_html.to_lowercase();

            if lower.contains("no relevant matches") || lower.contains("your image") {
                continue;
            }

            // Without a numeric similarity percentage, never assume a match.
            let similarity = match RE_SIMILARITY.captures(&table_html) {
                Some(caps) => caps
                    .get(1)
                    .and_then(|m| m.as_str().parse::<f64>().ok())
                    .unwrap_or(0.0),
                None => continue,
            };

            if similarity < min_similarity {
                continue;
            }

            let mut char_found: Option<String> = None;
            let mut series_found: Option<String> = None;

            for img in table.select(&img_sel) {
                if let Some(alt) = img.value().attr("alt") {
                    let (c, s) = extract_character_from_tags(alt);
                    if let Some(c) = c {
                        char_found = Some(c);
                        series_found = s;
                        break;
                    }
                }
            }

            if let Some(c) = char_found
                && similarity > best_sim {
                    best_sim = similarity;
                    best_char = Some(c);
                    best_series = series_found;
                }
            }

            (best_char, best_series, best_sim, found_table)
        };

        if !found_table {
            warn!("IQDB returned no result table.");
            return None;
        }

        let mut best_char = match best_char {
            Some(c) => c,
            None => {
                warn!(
                    "IQDB found no character match at or above {min_similarity}% similarity."
                );
                return None;
            }
        };

        // Correct the name and series via AniList when the series is unknown.
        if best_series.is_none() {
            info!("No series found on IQDB; trying AniList for '{best_char}'...");
            let (anilist_name, anilist_series) = lookup_series_from_character(&best_char).await;
            if anilist_series.is_some() {
                best_series = anilist_series;
            }
            if let Some(name) = anilist_name
                && name != best_char {
                    info!("AniList corrected the name: '{best_char}' -> '{name}'");
                    best_char = name;
                }
        }

        let (first_name, last_name) = split_name(&best_char);

        info!(
            "IQDB menemukan: {best_char} (Seri: {}, Kemiripan: {best_sim}%)",
            best_series.as_deref().unwrap_or("Unknown")
        );

        Some(CharacterInfo {
            full_name: best_char,
            first_name,
            last_name,
            series: best_series,
            confidence: best_sim / 100.0,
            source: "iqdb_search".to_string(),
            alternate_names: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mengekstrak_karakter_dari_tag_dengan_seri() {
        let (c, s) = extract_character_from_tags("houshou_marine_(hololive)");
        assert_eq!(c.as_deref(), Some("Houshou Marine"));
        assert_eq!(s.as_deref(), Some("Hololive"));
    }

    #[test]
    fn tag_generic_diabaikan() {
        let (c, _) = extract_character_from_tags("1girl, long_hair, blue_eyes");
        assert_eq!(c, None);
    }

    #[test]
    fn tag_seri_tidak_dianggap_karakter() {
        let (c, s) = extract_character_from_tags("hololive_production");
        assert_eq!(c, None);
        assert_eq!(s.as_deref(), Some("Hololive Production"));
    }

    #[test]
    fn nama_karakter_dua_kata_dikenali() {
        let (c, _) = extract_character_from_tags("houraisan_kaguya");
        assert_eq!(c.as_deref(), Some("Houraisan Kaguya"));
    }

    #[test]
    fn is_series_tag_mengenali_kata_seri() {
        assert!(is_series_tag("hololive"));
        assert!(is_series_tag("genshin_impact"));
        assert!(!is_series_tag("houraisan_kaguya"));
    }

    #[test]
    fn parse_parentheticals_mengambil_seri_paling_kanan() {
        let (c, s) = parse_parentheticals("inugami_korone_(dog)_(hololive)");
        // Sisa underscore di sini memang dipertahankan: `parse_parentheticals`
        // only strips the parentheses. The caller's `clean_text` does the
        // `clean_text` di fungsi pemanggil.
        assert_eq!(c.as_deref(), Some("inugami_korone__"));
        assert_eq!(s.as_deref(), Some("hololive"));
    }

    #[test]
    fn pipeline_lengkap_membersihkan_sisa_underscore() {
        // End-to-end: the physical qualifier in the middle is dropped, the
        // series comes from the rightmost parentheses, and the name ends up tidy.
        let (c, s) = extract_character_from_tags("inugami_korone_(dog)_(hololive)");
        assert_eq!(c.as_deref(), Some("Inugami Korone"));
        assert_eq!(s.as_deref(), Some("Hololive"));
    }

    #[test]
    fn qualifier_fisik_tidak_dianggap_seri() {
        // Regression: a lone single-word qualifier used to become the series,
        // so the dashboard showed "Dog" or "Plants" as the franchise.
        assert_eq!(
            extract_character_from_tags("inugami_korone_(dog)").1,
            None,
            "kualifikasi fisik tidak boleh jadi seri"
        );
        assert_eq!(
            extract_character_from_tags("hatsune_miku_(plants)").1,
            None,
            "tag pemandangan tidak boleh jadi seri"
        );
        // The character is still identified -- only the series is withheld.
        assert_eq!(
            extract_character_from_tags("inugami_korone_(dog)").0.as_deref(),
            Some("Inugami Korone")
        );
    }

    #[test]
    fn seri_satu_kata_multi_kata_tetap_dikenali() {
        // The single-word guard must not swallow a real multi-word series, nor
        // a known one-word series tag.
        assert_eq!(
            extract_character_from_tags("ai_hoshino_(oshi_no_ko)").1.as_deref(),
            Some("Oshi no Ko")
        );
        assert_eq!(
            extract_character_from_tags("komeiji_satori_(touhou)").1.as_deref(),
            Some("Touhou Project")
        );
    }

    #[test]
    fn vocaloid_adalah_seri_bukan_kostum() {
        // Regression: `vocaloid` sat in THEMES_AND_COSTUMES, so the caller's
        // `is_theme` check discarded the series and the dashboard said
        // "Unknown" for a perfectly good Hatsune Miku tag.
        let (c, s) = extract_character_from_tags("hatsune_miku_(vocaloid)");
        assert_eq!(c.as_deref(), Some("Hatsune Miku"));
        assert_eq!(s.as_deref(), Some("Vocaloid"));
    }
}
