//! Recognition cache keyed by image hash.
//!
//! The same character frequently spawns again in a group. Without a cache,
//! every appearance costs a full reverse search (~2.8 s for IQDB alone).
//! With it, a repeat is effectively instant.

use std::collections::HashMap;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::recognizer::CharacterInfo;

/// Maximum number of entries. Kept small because each one only holds
/// character metadata, never the image bytes.
const MAX_ENTRIES: usize = 512;

pub struct RecognitionCache {
    entries: Mutex<HashMap<String, CachedEntry>>,
    hits: Mutex<u64>,
    misses: Mutex<u64>,
}

struct CachedEntry {
    info: CharacterInfo,
    /// Insertion order, used for simple FIFO eviction.
    seq: u64,
}

impl RecognitionCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            hits: Mutex::new(0),
            misses: Mutex::new(0),
        }
    }

    /// SHA-256 of the image bytes. Cheap enough (hundreds of microseconds) and
    /// covers the common case: the game bot reposting the exact same file.
    pub fn key(image_bytes: &[u8]) -> String {
        use std::fmt::Write as _;

        let mut hasher = Sha256::new();
        hasher.update(image_bytes);
        let digest = hasher.finalize();

        let mut out = String::with_capacity(digest.len() * 2);
        for byte in digest.iter() {
            let _ = write!(out, "{byte:02x}");
        }
        out
    }

    pub fn get(&self, key: &str) -> Option<CharacterInfo> {
        let entries = self.entries.lock().ok()?;
        match entries.get(key) {
            Some(entry) => {
                let info = entry.info.clone();
                if let Ok(mut hits) = self.hits.lock() {
                    *hits += 1;
                }
                Some(info)
            }
            None => {
                if let Ok(mut misses) = self.misses.lock() {
                    *misses += 1;
                }
                None
            }
        }
    }

    pub fn put(&self, key: String, info: CharacterInfo) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };

        if entries.len() >= MAX_ENTRIES {
            // Evict the entry with the smallest sequence, i.e. the oldest.
            if let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, e)| e.seq)
                .map(|(k, _)| k.clone())
            {
                entries.remove(&oldest);
            }
        }

        let seq = entries
            .values()
            .map(|e| e.seq)
            .max()
            .unwrap_or(0)
            .wrapping_add(1);

        entries.insert(key, CachedEntry { info, seq });
    }

    /// Statistik `(jumlah_entri, hit, miss)`.
    pub fn stats(&self) -> (usize, u64, u64) {
        let len = self.entries.lock().map(|e| e.len()).unwrap_or(0);
        let hits = self.hits.lock().map(|h| *h).unwrap_or(0);
        let misses = self.misses.lock().map(|m| *m).unwrap_or(0);
        (len, hits, misses)
    }
}

impl Default for RecognitionCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_stabil_untuk_bytes_sama() {
        assert_eq!(
            RecognitionCache::key(b"gambar-uji"),
            RecognitionCache::key(b"gambar-uji")
        );
        assert_ne!(
            RecognitionCache::key(b"gambar-uji"),
            RecognitionCache::key(b"gambar-lain")
        );
    }

    #[test]
    fn simpan_dan_ambil() {
        let cache = RecognitionCache::new();
        let key = RecognitionCache::key(b"x");
        assert!(cache.get(&key).is_none());

        cache.put(
            key.clone(),
            CharacterInfo {
                full_name: "Houraisan Kaguya".into(),
                ..Default::default()
            },
        );

        let got = cache.get(&key).expect("entri harus ada");
        assert_eq!(got.full_name, "Houraisan Kaguya");
        assert_eq!(cache.stats().1, 1);
    }
}
