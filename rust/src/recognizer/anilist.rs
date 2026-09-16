//! AniList GraphQL lookups.
//!
//! Free and requires no API key.

use std::time::Duration;

use serde_json::{json, Value};
use tracing::{debug, info};

use crate::http::CLIENT;

const ANILIST_GRAPHQL: &str = "https://graphql.anilist.co";

const CHAR_QUERY: &str = r#"
query ($search: String) {
  Character(search: $search) {
    name {
      full
      native
    }
    media(sort: POPULARITY_DESC, page: 1, perPage: 1) {
      nodes {
        title {
          romaji
          english
        }
        type
      }
    }
  }
}
"#;

const MEDIA_CHARACTERS_QUERY: &str = r#"
query ($id: Int) {
  Media(id: $id) {
    title {
      romaji
      english
    }
    characters(sort: [ROLE, RELEVANCE], perPage: 6) {
      nodes {
        name {
          full
          native
        }
      }
    }
  }
}
"#;

async fn post_graphql(payload: &Value) -> Option<Value> {
    let resp = CLIENT
        .post(ANILIST_GRAPHQL)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(8))
        .json(payload)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        debug!("AniList HTTP {}", resp.status());
        return None;
    }

    resp.json::<Value>().await.ok()
}

/// Look up a series name from a character name.
///
/// Mengembalikan `(nama_kanonik, nama_seri)`.
pub async fn lookup_series_from_character(char_name: &str) -> (Option<String>, Option<String>) {
    let payload = json!({
        "query": CHAR_QUERY,
        "variables": { "search": char_name },
    });

    let data = match post_graphql(&payload).await {
        Some(d) => d,
        None => return (None, None),
    };

    let char_data = match data.get("data").and_then(|d| d.get("Character")) {
        Some(c) if !c.is_null() => c,
        _ => {
            debug!("AniList tidak menemukan karakter: '{char_name}'");
            return (None, None);
        }
    };

    let canonical_name = char_data
        .get("name")
        .and_then(|n| n.get("full"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| char_name.to_string());

    let series_name = char_data
        .get("media")
        .and_then(|m| m.get("nodes"))
        .and_then(|n| n.as_array())
        .and_then(|arr| arr.first())
        .and_then(|node| node.get("title"))
        .and_then(|t| {
            t.get("romaji")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| t.get("english").and_then(|v| v.as_str()))
        })
        .map(|s| s.to_string());

    info!("AniList: '{char_name}' -> character='{canonical_name}', series={series_name:?}");
    (Some(canonical_name), series_name)
}

/// Look up the series title and main characters for an AniList `media_id`.
///
/// Mengembalikan `(judul_seri, [nama_karakter])`.
pub async fn lookup_characters_by_media_id(media_id: i64) -> (Option<String>, Vec<String>) {
    let payload = json!({
        "query": MEDIA_CHARACTERS_QUERY,
        "variables": { "id": media_id },
    });

    let data = match post_graphql(&payload).await {
        Some(d) => d,
        None => return (None, Vec::new()),
    };

    let media = match data.get("data").and_then(|d| d.get("Media")) {
        Some(m) if !m.is_null() => m,
        _ => return (None, Vec::new()),
    };

    let series_name = media
        .get("title")
        .and_then(|t| {
            t.get("romaji")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| t.get("english").and_then(|v| v.as_str()))
        })
        .unwrap_or("Anime")
        .to_string();

    let mut char_names: Vec<String> = Vec::new();
    if let Some(nodes) = media
        .get("characters")
        .and_then(|c| c.get("nodes"))
        .and_then(|n| n.as_array())
    {
        for node in nodes {
            if let Some(full) = node
                .get("name")
                .and_then(|n| n.get("full"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                && !char_names.iter().any(|existing| existing == full) {
                    char_names.push(full.to_string());
                }
        }
    }

    info!("AniList media {media_id} ({series_name}): characters {char_names:?}");
    (Some(series_name), char_names)
}
