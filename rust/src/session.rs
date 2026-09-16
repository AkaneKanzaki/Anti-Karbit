//! Telegram session portability.
//!
//! The problem: grammers stores its `SqliteSession` as a file, and a platform
//! like Railway without a persistent volume deletes that file on every deploy,
//! so the bot would ask for an OTP again each time.
//!
//! The fix: encode the session file as base64 into an environment variable and
//! write it back to disk at startup. Session files are only tens of kilobytes,
//! so they fit comfortably in an env var.
//!
//! A zero-code alternative is to attach a Railway Volume and point
//! `TELEGRAM_SESSION_NAME` at a path inside it.

use std::path::Path;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;

/// Write the session file from `TELEGRAM_STRING_SESSION` when it is set.
///
/// Returns `true` when the session file was restored from the env var.
pub fn materialize_from_env(session_path: &str) -> Result<bool, SessionError> {
    let encoded = crate::config::get().telegram_string_session;
    materialize_from_string(encoded.trim(), session_path)
}

/// The core of [`materialize_from_env`], split out so it can be tested
/// without touching the process environment.
pub fn materialize_from_string(encoded: &str, session_path: &str) -> Result<bool, SessionError> {
    if encoded.is_empty() {
        return Ok(false);
    }

    // Never overwrite a local session that already exists and is valid.
    if Path::new(session_path).is_file() {
        return Ok(false);
    }

    let bytes = B64
        .decode(encoded)
        .map_err(|e| SessionError::Decode(e.to_string()))?;

    std::fs::write(session_path, &bytes).map_err(|e| SessionError::Io(e.to_string()))?;

    Ok(true)
}

/// Read the session file and return it as a paste-ready base64 string.
pub fn export_to_string(session_path: &str) -> Result<String, SessionError> {
    let bytes = std::fs::read(session_path).map_err(|e| SessionError::Io(e.to_string()))?;
    Ok(B64.encode(bytes))
}

#[derive(Debug)]
pub enum SessionError {
    Decode(String),
    Io(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Decode(e) => write!(f, "Gagal mendekode TELEGRAM_STRING_SESSION: {e}"),
            SessionError::Io(e) => write!(f, "Gagal mengakses file sesi: {e}"),
        }
    }
}

impl std::error::Error for SessionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bolak_balik_base64_mempertahankan_bytes() {
        let dir = std::env::temp_dir().join("antikarbit_session_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("roundtrip.session");

        let original: Vec<u8> = (0u8..=255).collect();
        std::fs::write(&path, &original).unwrap();

        let encoded = export_to_string(path.to_str().unwrap()).unwrap();
        let decoded = B64.decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn string_kosong_tidak_menulis_apa_pun() {
        let path = std::env::temp_dir()
            .join("antikarbit_session_test")
            .join("kosong.session");
        let _ = std::fs::remove_file(&path);

        let result = materialize_from_string("", path.to_str().unwrap()).unwrap();
        assert!(!result);
        assert!(!path.exists());
    }

    #[test]
    fn memulihkan_sesi_dari_string_base64() {
        let dir = std::env::temp_dir().join("antikarbit_session_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("restore.session");
        let _ = std::fs::remove_file(&path);

        let original: Vec<u8> = (0u8..=255).collect();
        let encoded = B64.encode(&original);

        let restored = materialize_from_string(&encoded, path.to_str().unwrap()).unwrap();
        assert!(restored);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn tidak_menimpa_sesi_yang_sudah_ada() {
        let dir = std::env::temp_dir().join("antikarbit_session_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("existing.session");

        std::fs::write(&path, b"sesi-asli").unwrap();
        let encoded = B64.encode(b"sesi-baru");

        let restored = materialize_from_string(&encoded, path.to_str().unwrap()).unwrap();
        assert!(!restored, "sesi lama tidak boleh ditimpa");
        assert_eq!(std::fs::read(&path).unwrap(), b"sesi-asli");
    }
}
