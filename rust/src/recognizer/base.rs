//! Shared recognizer types.

/// Informasi karakter hasil pengenalan.
#[derive(Clone, Debug)]
pub struct CharacterInfo {
    pub full_name: String,
    pub first_name: String,
    pub last_name: Option<String>,
    pub series: Option<String>,
    pub confidence: f64,
    pub source: String,
    /// Alternate names reported by an engine, used as claim fallbacks.
    pub alternate_names: Vec<String>,
}

impl Default for CharacterInfo {
    fn default() -> Self {
        Self {
            full_name: String::new(),
            first_name: String::new(),
            last_name: None,
            series: None,
            confidence: 1.0,
            source: "unknown".to_string(),
            alternate_names: Vec::new(),
        }
    }
}

impl CharacterInfo {
    /// Names to send for a claim, according to `config.NAME_FORMAT`.
    ///
    /// - `first`: given name only
    /// - `both`: given name, then full name
    /// - anything else (`full`): full name only
    pub fn claim_names(&self, mode: &str) -> Vec<String> {
        let mut names = Vec::new();
        match mode {
            "first" => {
                if !self.first_name.is_empty() {
                    names.push(self.first_name.clone());
                } else {
                    names.push(self.full_name.clone());
                }
            }
            "both" => {
                if !self.first_name.is_empty() {
                    names.push(self.first_name.clone());
                }
                if !self.full_name.is_empty() && self.full_name != self.first_name {
                    names.push(self.full_name.clone());
                }
            }
            _ => names.push(self.full_name.clone()),
        }
        names
    }
}

/// Split a full name into (given name, family name).
///
/// Booru and AniList use "Family Given" order, so the **last** word is taken
/// as the given name.
pub fn split_name(full: &str) -> (String, Option<String>) {
    let parts: Vec<&str> = full.split_whitespace().collect();
    if parts.len() >= 2 {
        let first = parts[parts.len() - 1].to_string();
        let last = parts[..parts.len() - 1].join(" ");
        (first, Some(last))
    } else {
        (full.to_string(), None)
    }
}

/// Turn `some_underscored_name` into `Some Underscored Name`.
pub fn clean_text(s: &str) -> String {
    s.replace('_', " ")
        .split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = String::new();
                    out.extend(first.to_uppercase());
                    out.push_str(&chars.as_str().to_lowercase());
                    out
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Detect the MIME type from the image's magic bytes.
pub fn detect_mime(image_bytes: &[u8]) -> &'static str {
    if image_bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if image_bytes.starts_with(b"RIFF") && image_bytes.len() >= 16 && &image_bytes[8..12] == b"WEBP"
    {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_name_mengambil_kata_terakhir_sebagai_nama_depan() {
        let (first, last) = split_name("Houraisan Kaguya");
        assert_eq!(first, "Kaguya");
        assert_eq!(last.as_deref(), Some("Houraisan"));
    }

    #[test]
    fn split_name_tanpa_spasi() {
        let (first, last) = split_name("Kaguya");
        assert_eq!(first, "Kaguya");
        assert_eq!(last, None);
    }

    #[test]
    fn clean_text_mengubah_underscore() {
        assert_eq!(clean_text("houshou_marine"), "Houshou Marine");
    }

    #[test]
    fn detect_mime_mengenali_png_dan_webp() {
        assert_eq!(detect_mime(b"\x89PNG\r\n\x1a\n"), "image/png");
        assert_eq!(detect_mime(b"RIFF\0\0\0\0WEBPVP8 "), "image/webp");
        assert_eq!(detect_mime(b"\xff\xd8\xff"), "image/jpeg");
    }

    #[test]
    fn claim_names_mode_first() {
        let c = CharacterInfo {
            full_name: "Houraisan Kaguya".into(),
            first_name: "Kaguya".into(),
            ..Default::default()
        };
        assert_eq!(c.claim_names("first"), vec!["Kaguya"]);
        assert_eq!(c.claim_names("full"), vec!["Houraisan Kaguya"]);
        assert_eq!(c.claim_names("both"), vec!["Kaguya", "Houraisan Kaguya"]);
    }
}
