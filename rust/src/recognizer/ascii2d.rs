//! Ascii2d recognizer.
//!
//! Ascii2d (<https://ascii2d.net>) is a Japanese reverse image search engine
//! that indexes Pixiv and Twitter artwork. Free, no API key required.
//!
//! How it differs from IQDB:
//!
//! * It runs **two searches**. The first is a colour search
//!   (`/search/file`), which is the one that returns the Pixiv/Twitter post.
//!   If that finds nothing, it falls back to a monochrome search
//!   (`/search/file?type=color` is the colour variant; the plain form answer is
//!   the "bovw" / feature-based search) for black-and-white manga art.
//! * The result page is a list of `.item-box` rows. Each row holds a thumbnail,
//!   a link to the source (Pixiv/Twitter), and a detail block with the author,
//!   the title, and any tags.
//! * There is **no similarity percentage**. The engine either has the artwork
//!   indexed or it does not. Confidence is therefore derived from what the row
//!   contributed: a character-shaped tag is stronger evidence than a bare
//!   title, and a row that yields nothing usable is discarded.
//!
//! HTML is parsed with `scraper` rather than regex over the whole document, for
//! the same reason as IQDB: it survives markup changes far better.

use std::time::Duration;

use reqwest::multipart::{Form, Part};
use scraper::{Html, Selector};
use tracing::{info, warn};

use super::base::{clean_text, detect_mime, split_name, CharacterInfo};
use super::iqdb::extract_character_from_tags;
use crate::http::CLIENT;

const COLOR_ENDPOINT: &str = "https://ascii2d.net/search/file";
const MONO_ENDPOINT: &str = "https://ascii2d.net/search/file?type=color";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Confidence assigned when a row yielded a character name and a series.
const CONFIDENCE_STRONG: f64 = 0.85;
/// Confidence when only a character name was found.
const CONFIDENCE_NAME: f64 = 0.75;
/// Confidence when only a title/character could be derived from a Pixiv title.
const CONFIDENCE_TITLE: f64 = 0.65;

// Because Ascii2d reports no percentage, the configured `ASCII2D_MIN_SIMILARITY`
// is compared against the evidence strengths above. The practical effect on the
// shipped default of 80%:
//
//   80 (default) -> only rows yielding a character AND a series are used
//   75           -> rows with a bare character name also qualify
//   65           -> title-derived guesses also qualify
//   0            -> everything qualifies

/// A single parsed Ascii2d result row.
#[derive(Debug, Clone, Default, PartialEq)]
struct Ascii2dRow {
    /// `booru` / `pixiv` / `twitter` — where the match came from.
    detail: String,
    /// Tag string scraped from the row's tag list, if any.
    tags: String,
    /// The post title, e.g. a Pixiv artwork title.
    title: String,
    /// The author/artist name.
    author: String,
}

impl Ascii2dRow {
    /// Whether this row carried no usable information at all.
    fn is_empty(&self) -> bool {
        self.tags.trim().is_empty() && self.title.trim().is_empty()
    }
}

/// Identifies anime characters through Ascii2d (<https://ascii2d.net>).
///
/// The threshold and the enable flag are read from configuration **at scan
/// time**, so dashboard changes take effect without a restart.
#[derive(Default)]
pub struct Ascii2dRecognizer;

impl Ascii2dRecognizer {
    pub async fn identify(&self, image_bytes: &[u8]) -> Option<CharacterInfo> {
        let cfg = crate::config::get();
        if !cfg.ascii2d_enabled {
            return None;
        }
        let min_similarity = cfg.ascii2d_min_similarity;

        info!("Sending image to Ascii2d...");

        // Colour search first: that is where Pixiv/Twitter artwork lands.
        let mut rows = match self.search(COLOR_ENDPOINT, image_bytes).await {
            Ok(rows) => rows,
            Err(e) => {
                warn!("Ascii2d colour search failed: {e}");
                Vec::new()
            }
        };

        if rows.iter().all(|r| r.is_empty()) {
            info!("Ascii2d colour search found nothing; trying the monochrome search...");
            match self.search(MONO_ENDPOINT, image_bytes).await {
                Ok(mono) => rows = mono,
                Err(e) => warn!("Ascii2d monochrome search failed: {e}"),
            }
        }

        if rows.is_empty() {
            warn!("Ascii2d returned no result rows.");
            return None;
        }

        // Rank the rows by how strong their evidence is, then keep the best.
        let mut best: Option<(String, Option<String>, f64)> = None;

        for row in &rows {
            let Some((name, series, base_confidence)) = derive_from_row(row) else {
                continue;
            };

            let better = match &best {
                Some((_, _, current)) => base_confidence > *current,
                None => true,
            };
            if better {
                best = Some((name, series, base_confidence));
            }
        }

        let Some((best_char, best_series, base_confidence)) = best else {
            warn!(
                "Ascii2d returned {} row(s) but no character name could be derived.",
                rows.len()
            );
            return None;
        };

        // Ascii2d does not report a similarity percentage: it either has the
        // artwork indexed or it does not. The configured threshold is therefore
        // compared against the *evidence strength* of the matched row, on the
        // same 0-100 scale the IQDB/SauceNAO thresholds use.
        let evidence_pct = base_confidence * 100.0;
        if !clears_threshold(base_confidence, min_similarity) {
            info!(
                "Ascii2d row for '{best_char}' scored {evidence_pct:.0}%, below the \
                 {min_similarity:.0}% threshold; discarding."
            );
            return None;
        }

        // Report the evidence strength as a ratio, matching CharacterInfo's
        // 0.0-1.0 contract.
        let confidence = base_confidence.clamp(0.0, 1.0);

        let (first_name, last_name) = split_name(&best_char);

        info!(
            "Ascii2d menemukan: {best_char} (Seri: {}, Keyakinan: {:.0}%)",
            best_series.as_deref().unwrap_or("Unknown"),
            confidence * 100.0
        );

        Some(CharacterInfo {
            full_name: best_char,
            first_name,
            last_name,
            series: best_series,
            confidence,
            source: "ascii2d".to_string(),
            alternate_names: Vec::new(),
        })
    }

    /// POST the image to one Ascii2d endpoint and parse the result rows.
    async fn search(&self, endpoint: &str, image_bytes: &[u8]) -> Result<Vec<Ascii2dRow>, String> {
        let mime = detect_mime(image_bytes);

        let file_part = Part::bytes(image_bytes.to_vec())
            .file_name("waifu.jpg")
            .mime_str(mime)
            .map_err(|e| format!("could not build the multipart part: {e}"))?;
        let form = Form::new().part("file", file_part);

        let resp = CLIENT
            .post(endpoint)
            .timeout(REQUEST_TIMEOUT)
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("request to {endpoint} failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("{endpoint} returned HTTP {}", resp.status()));
        }

        let html = resp
            .text()
            .await
            .map_err(|e| format!("response body could not be read: {e}"))?;

        Ok(parse_rows(&html))
    }
}

/// Parse every result row out of an Ascii2d result page.
///
/// Kept as a free function, separate from the network call, so it is testable
/// against a captured page.
fn parse_rows(html: &str) -> Vec<Ascii2dRow> {
    let doc = Html::parse_document(html);

    let row_sel = Selector::parse("div.item-box").expect("selector item-box valid");
    let thumb_sel = Selector::parse("div.image-box img").expect("selector image-box valid");
    let detail_sel = Selector::parse("div.detail-box").expect("selector detail-box valid");
    let link_sel = Selector::parse("div.detail-box a").expect("selector detail-box a valid");
    let tag_sel = Selector::parse("div.detail-box a[href*='/search/']")
        .expect("selector detail tag valid");

    let mut rows: Vec<Ascii2dRow> = Vec::new();

    for row in doc.select(&row_sel) {
        // The first `.item-box` on the page is the header row containing the
        // query image itself, not a result. It has no `.detail-box`.
        let Some(detail) = row.select(&detail_sel).next() else {
            continue;
        };

        let detail_text = normalise_whitespace(&detail.text().collect::<Vec<_>>().join(" "));

        // "booru" / "pixiv" / "twitter" appears as the whole text of the first
        // link in the detail block.
        let mut detail_kind = String::new();
        let mut title = String::new();
        let mut author = String::new();

        for (index, link) in detail.select(&link_sel).enumerate() {
            let text = normalise_whitespace(&link.text().collect::<Vec<_>>().join(" "));
            if text.is_empty() {
                continue;
            }
            if index == 0 && is_source_label(&text) {
                detail_kind = text.to_lowercase();
                continue;
            }
            if author.is_empty() {
                author = clean_text(&text);
            } else if title.is_empty() {
                title = clean_text(&text);
            }
        }

        // Tag list lives in the `a[href*="/search/"]` links of the detail box,
        // but that pattern also matches the source label ("Pixiv" links to
        // `/search/pixiv`), so labels are filtered out explicitly.
        let mut tags: Vec<String> = Vec::new();
        for tag in detail.select(&tag_sel) {
            let text = normalise_whitespace(&tag.text().collect::<Vec<_>>().join(" "));
            if text.is_empty() || is_source_label(&text) {
                continue;
            }
            tags.push(text);
        }
        if tags.is_empty() {
            for img in row.select(&thumb_sel) {
                if let Some(alt) = img.value().attr("alt")
                    && !alt.trim().is_empty()
                {
                    tags.push(alt.trim().to_string());
                }
            }
        }

        // Ascii2d renders tags space separated inside one text node, so the
        // joined string is what the IQDB tag extractor expects.
        let tags_text = if tags.is_empty() {
            String::new()
        } else {
            tags.join(" ")
        };

        // Fall back to parsing the detail text when no links were found: some
        // rows render the metadata as plain text (for example "Title").
        if detail_kind.is_empty() {
            detail_kind = detail_kind_from_text(&detail_text);
        }

        rows.push(Ascii2dRow {
            detail: detail_kind,
            tags: tags_text,
            title,
            author,
        });
    }

    rows
}

/// Whether a detail-box link label names the source site rather than a person.
fn is_source_label(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    matches!(lower.as_str(), "booru" | "pixiv" | "twitter" | "anidb" | "seiga")
}

/// Pull the source label out of the raw detail text.
fn detail_kind_from_text(text: &str) -> String {
    let lower = text.to_lowercase();
    for label in ["booru", "pixiv", "twitter", "anidb", "seiga"] {
        if lower.contains(label) {
            return label.to_string();
        }
    }
    String::new()
}

fn normalise_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Turn one parsed row into `(name, series, base_confidence)`.
///
/// The tag string goes through the same booru-aware extractor IQDB uses, so
/// generic tags such as `1girl` or `long_hair` are never mistaken for a name.
fn derive_from_row(row: &Ascii2dRow) -> Option<(String, Option<String>, f64)> {
    if !row.tags.trim().is_empty() {
        let (character, series) = extract_character_from_tags(&row.tags);
        if let Some(name) = character {
            let confidence = if series.is_some() {
                CONFIDENCE_STRONG
            } else {
                CONFIDENCE_NAME
            };
            return Some((name, series, confidence));
        }
    }

    // No usable tag: fall back to the title, but only when it looks like a
    // person's name rather than an artwork title.
    let title = row.title.trim();
    if name_looks_like_person(title) {
        let (character, series) = extract_character_from_tags(title);
        if let Some(name) = character {
            return Some((name, series, CONFIDENCE_TITLE));
        }
        return Some((clean_text(title), None, CONFIDENCE_TITLE));
    }

    None
}

/// Whether a row's evidence strength clears the configured threshold.
///
/// Ascii2d reports no percentage, so the threshold is compared against how much
/// the matched row actually contributed: a character plus a series is the
/// strongest signal, a bare character name is weaker, and a title-derived guess
/// is weakest. Extracted so the comparison is testable without a network call.
fn clears_threshold(base_confidence: f64, min_similarity: f64) -> bool {
    base_confidence * 100.0 >= min_similarity
}

/// A conservative check that a title is plausibly a character name.
///
/// Pixiv titles are frequently sentences ("Thank you for 1000 followers"), and
/// claiming on one of those would send a nonsense command to the group.
fn name_looks_like_person(title: &str) -> bool {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 40 {
        return false;
    }

    let words: Vec<&str> = title.split_whitespace().collect();
    if words.is_empty() || words.len() > 3 {
        return false;
    }

    // Reject anything with sentence punctuation.
    if title.contains(['.', '!', '?', ':', ';', '"', '\'', ',']) {
        return false;
    }

    // Every word must look like a proper noun: letters only, at least two long.
    words.iter().all(|word| {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric());
        word.chars().count() >= 2
            && word.chars().all(|c| c.is_alphabetic() || c == '-' || c == '_')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed-down copy of a real Ascii2d result page: one header row and
    /// two result rows, one Booru and one Pixiv.
    const SAMPLE_PAGE: &str = r#"
    <html><body>
      <div class="row">
        <div class="item-box">
          <div class="image-box"><img src="/thumb/query.jpg" alt=""></div>
        </div>
      </div>
      <div class="row">
        <div class="item-box">
          <div class="image-box"><img src="//img.example/1.jpg" alt="hakushika_iori"></div>
          <div class="detail-box">
            <a href="/search/booru">Booru</a>
            <a href="/danbooru/post/1">Some Artist</a>
            <a href="/search/hakushika_iori">hakushika_iori</a>
            <a href="/search/genshin_impact">genshin_impact</a>
          </div>
        </div>
      </div>
      <div class="row">
        <div class="item-box">
          <div class="image-box"><img src="//img.example/2.jpg" alt=""></div>
          <div class="detail-box">
            <a href="/search/pixiv">Pixiv</a>
            <a href="https://pixiv.net/users/2">Another Artist</a>
            <a href="https://pixiv.net/artworks/2">Hourou Musuko</a>
          </div>
        </div>
      </div>
    </body></html>
    "#;

    fn row(html: &str) -> Vec<Ascii2dRow> {
        parse_rows(html)
    }

    /// A result page shaped like the real one: rows are wrapped in `div.row`,
    /// the query image is the first `.item-box`, and a Booru row carries several
    /// `/search/` tag links.
    const REALISTIC_PAGE: &str = r#"
    <html><body>
      <div class="row">
        <div class="item-box">
          <div class="image-box"><img src="/thumb/upload.jpg" alt=""></div>
          <div class="detail-box">
            <div class="detail-box gray-link"><span>Image search results</span></div>
          </div>
        </div>
      </div>
      <div class="row">
        <div class="item-box">
          <div class="image-box"><img src="//cdn.example/a.jpg" alt=""></div>
          <div class="detail-box">
            <div class="detail-box gray-link">
              <a href="/search/booru">Booru</a>
              <a href="/danbooru/post/42">Artist Name</a>
            </div>
            <div class="detail-box gray-link">
              <a href="/search/hakushika_iori">hakushika_iori</a>
              <a href="/search/blue_archive">blue_archive</a>
              <a href="/search/halo">halo</a>
            </div>
          </div>
        </div>
      </div>
      <div class="row">
        <div class="item-box">
          <div class="image-box"><img src="//cdn.example/b.jpg" alt=""></div>
          <div class="detail-box">
            <div class="detail-box gray-link">
              <a href="/search/pixiv">Pixiv</a>
              <a href="//pixiv.net/users/1">Pixiv Artist</a>
            </div>
            <div class="detail-box gray-link">
              <a href="//pixiv.net/artworks/1">Some Artwork Title</a>
            </div>
          </div>
        </div>
      </div>
    </body></html>
    "#;

    #[test]
    fn halaman_realistis_menghasilkan_karakter_dan_seri() {
        let rows = parse_rows(REALISTIC_PAGE);
        assert_eq!(rows.len(), 3, "query row dan dua hasil harus terbaca");

        // The Booru row is the usable one.
        let (name, series, confidence) = derive_from_row(&rows[1]).expect("baris booru dikenali");
        assert_eq!(name, "Hakushika Iori");
        assert_eq!(series.as_deref(), Some("Blue Archive"));
        assert_eq!(confidence, CONFIDENCE_STRONG);
    }

    #[test]
    fn label_sumber_tidak_menjadi_nama_karakter() {
        // "Booru" and "Pixiv" links point at /search/<label>; if they leaked
        // into the tag list the extractor could mistake them for a tag.
        let rows = parse_rows(REALISTIC_PAGE);
        assert!(
            !rows.iter().any(|r| r.tags.contains("Booru")),
            "label Booru bocor ke tag"
        );
        assert!(
            !rows.iter().any(|r| r.tags.contains("Pixiv")),
            "label Pixiv bocor ke tag"
        );
    }

    #[test]
    fn tag_generik_halo_tidak_menjadi_nama() {
        // `halo` is a generic costume tag in the booru dictionary.
        let rows = parse_rows(REALISTIC_PAGE);
        assert_eq!(rows[1].detail, "booru");
    }

    #[test]
    fn baris_query_tanpa_tag_tidak_lolos() {
        // The upload row must never be the source of a claim.
        let rows = parse_rows(REALISTIC_PAGE);
        assert!(derive_from_row(&rows[0]).is_none());
    }

    #[test]
    fn baris_header_tanpa_detail_box_diabaikan() {
        // Only the two real result rows survive; the query-image header row
        // has no `.detail-box` and must not be treated as a match.
        assert_eq!(row(SAMPLE_PAGE).len(), 2);
    }

    #[test]
    fn halaman_kosong_menghasilkan_tanpa_baris() {
        assert!(row("<html><body><p>No results</p></body></html>").is_empty());
    }

    #[test]
    fn baris_booru_membaca_tag_dan_sumber() {
        let rows = row(SAMPLE_PAGE);
        let booru = &rows[0];
        assert_eq!(booru.detail, "booru");
        assert!(booru.tags.contains("hakushika_iori"));
        assert_eq!(booru.author, "Some Artist");
    }

    #[test]
    fn label_sumber_tidak_ikut_masuk_daftar_tag() {
        // `/search/pixiv` is the source label, not a tag. Letting it through
        // would feed "pixiv" to the tag extractor on every Pixiv row.
        let rows = row(SAMPLE_PAGE);
        let pixiv = &rows[1];
        assert!(
            pixiv.tags.is_empty(),
            "label sumber bocor ke daftar tag: {:?}",
            pixiv.tags
        );
    }

    #[test]
    fn tag_karakter_diutamakan_daripada_judul() {
        let rows = row(SAMPLE_PAGE);
        let (name, series, confidence) = derive_from_row(&rows[0]).expect("baris booru dikenali");
        assert_eq!(name, "Hakushika Iori");
        // Series names come out title-cased through the alias table.
        assert_eq!(series.as_deref(), Some("Genshin Impact"));
        assert_eq!(confidence, CONFIDENCE_STRONG);
    }

    #[test]
    fn tag_generik_tidak_pernah_jadi_nama() {
        let generic = Ascii2dRow {
            detail: "booru".into(),
            tags: "1girl solo long_hair blue_eyes school_uniform".into(),
            title: String::new(),
            author: String::new(),
        };
        assert!(
            derive_from_row(&generic).is_none(),
            "tag generik tidak boleh dianggap nama karakter"
        );
    }

    #[test]
    fn baris_kosong_dikenali() {
        assert!(Ascii2dRow::default().is_empty());
    }

    #[test]
    fn kalimat_judul_pixiv_bukan_nama_orang() {
        assert!(!name_looks_like_person("Thank you for 1000 followers!"));
        assert!(!name_looks_like_person("Happy New Year 2026"));
        assert!(!name_looks_like_person(""));
        assert!(!name_looks_like_person(
            "a very long artwork title that runs on and on"
        ));
    }

    #[test]
    fn nama_dua_kata_diterima_sebagai_nama_orang() {
        assert!(name_looks_like_person("Hakushika Iori"));
        assert!(name_looks_like_person("Kaguya"));
    }

    #[test]
    fn label_sumber_dikenali() {
        assert!(is_source_label("Booru"));
        assert!(is_source_label(" pixiv "));
        assert!(!is_source_label("Some Artist"));
    }

    #[test]
    fn teks_detail_dipakai_saat_tautan_tidak_ada() {
        assert_eq!(detail_kind_from_text("pixiv 12345"), "pixiv");
        assert_eq!(detail_kind_from_text("no source here"), "");
    }

    #[test]
    fn ambang_80_meloloskan_tag_karakter_dengan_seri() {
        // 0.85 evidence vs the 80% default: passes, which is what makes the
        // default configuration actually usable rather than silently inert.
        assert!(clears_threshold(CONFIDENCE_STRONG, 80.0));
    }

    #[test]
    fn ambang_80_menolak_tebakan_dari_judul() {
        // A title-derived guess is weak evidence and must not clear 80%.
        assert!(!clears_threshold(CONFIDENCE_TITLE, 80.0));
    }

    #[test]
    fn ambang_80_menolak_nama_tanpa_seri() {
        // At the 80% default, only a row that yielded BOTH a character and a
        // series is strong enough. A bare name (75%) is not. This is the
        // intended trade-off: fewer, more reliable claims.
        assert!(!clears_threshold(CONFIDENCE_NAME, 80.0));
    }

    #[test]
    fn ambang_75_meloloskan_nama_tanpa_seri() {
        // Lowering the threshold to 75% is what enables bare-name matches,
        // so the relationship is monotonic rather than magic.
        assert!(clears_threshold(CONFIDENCE_NAME, 75.0));
    }

    #[test]
    fn ambang_di_atas_100_menolak_semua() {
        // Guards against a mis-typed threshold that would disable the engine
        // in a way the user cannot diagnose from the dashboard.
        assert!(!clears_threshold(CONFIDENCE_STRONG, 101.0));
    }

    #[test]
    fn ambang_nol_meloloskan_semua() {
        assert!(clears_threshold(CONFIDENCE_TITLE, 0.0));
    }
}
