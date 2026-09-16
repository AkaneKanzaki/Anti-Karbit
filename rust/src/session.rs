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
use tracing::warn;

/// Header magic SQLite database.
const SQLITE_HEADER: &[u8] = b"SQLite format 3\0";

/// Membersihkan string base64 dari kemungkinan karakter liar saat disalin:
/// - Tanda kutip pembungkus (" atau ')
/// - Semua karakter whitespace (\r, \n, spasi, tab dari terminal line wrapping)
/// - Karakter URL-safe (- dan _) dikonversi ke standard (+ dan /)
/// - Penambahan padding '=' jika terpotong
pub fn clean_base64_session(raw: &str) -> String {
    let mut s = raw.trim();
    // Hapus tanda petik pembungkus
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        if s.len() >= 2 {
            s = &s[1..s.len() - 1];
        }
    }

    // Filter seluruh whitespace (mengatasi wrapping baris terminal dan multi-line env)
    let mut cleaned: String = s
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();

    // Hapus petik lagi jika sebelumnya berlapis (misal ' "..." ')
    if (cleaned.starts_with('"') && cleaned.ends_with('"'))
        || (cleaned.starts_with('\'') && cleaned.ends_with('\''))
    {
        if cleaned.len() >= 2 {
            cleaned = cleaned[1..cleaned.len() - 1].to_string();
        }
    }

    // Konversi URL-safe base64 ke standard
    let mut normalized = cleaned.replace('-', "+").replace('_', "/");

    // Auto-pad missing base64 padding
    let rem = normalized.len() % 4;
    if rem == 2 {
        normalized.push_str("==");
    } else if rem == 3 {
        normalized.push('=');
    }

    normalized
}

/// Write the session file from `TELEGRAM_STRING_SESSION` when it is set.
///
/// Returns `true` when the session file was restored from the env var.
pub fn materialize_from_env(session_path: &str) -> Result<bool, SessionError> {
    let encoded = crate::config::get().telegram_string_session;
    materialize_from_string(&encoded, session_path)
}

/// The core of [`materialize_from_env`], split out so it can be tested
/// without touching the process environment.
pub fn materialize_from_string(encoded: &str, session_path: &str) -> Result<bool, SessionError> {
    let cleaned = clean_base64_session(encoded);
    if cleaned.is_empty() {
        return Ok(false);
    }

    // Never overwrite a local session that already exists and is non-empty (>16 bytes).
    if let Ok(meta) = Path::new(session_path).metadata() {
        if meta.len() > 16 {
            return Ok(false);
        }
    }

    // Sisa modulo 4 == 1 tidak mungkin valid dalam base64 (1 karakter = 6 bit, < 1 byte)
    if cleaned.len() % 4 == 1 {
        return Err(SessionError::Decode(format!(
            "Panjang input tidak valid: {} (sisa 1 karakter). Pastikan seluruh string session tersalin lengkap tanpa karakter terpotong.",
            cleaned.len()
        )));
    }

    let bytes = B64
        .decode(&cleaned)
        .map_err(|e| SessionError::Decode(format!("{e} (panjang string: {})", cleaned.len())))?;

    if bytes.is_empty() {
        return Err(SessionError::Decode("Hasil dekode kosong.".to_string()));
    }

    // Peringatkan bila hasil dekode bukan file SQLite session Telegram
    if bytes.len() >= 16 && !bytes.starts_with(SQLITE_HEADER) {
        warn!(
            "Header file sesi tidak diawali dengan 'SQLite format 3'. Pastikan TELEGRAM_STRING_SESSION diekspor melalui 'antikarbit export-session' (bukan Telethon atau Pyrogram string session)."
        );
    }

    if let Some(parent) = Path::new(session_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| SessionError::Io(e.to_string()))?;
        }
    }

    std::fs::write(session_path, &bytes).map_err(|e| SessionError::Io(e.to_string()))?;

    Ok(true)
}

/// Read the session file and return it as a paste-ready base64 string.
pub fn export_to_string(session_path: &str) -> Result<String, SessionError> {
    let bytes = std::fs::read(session_path).map_err(|e| SessionError::Io(e.to_string()))?;
    Ok(B64.encode(bytes))
}

/// Memeriksa dan memperbaiki file sesi SQLite sebelum dibuka oleh grammers:
/// 1. Jika tabel `dc_home` sudah ada tetapi `PRAGMA user_version` masih 0 (misal dibuat oleh generator eksternal),
///    set `user_version = 1` agar grammers tidak mencoba `migrate_v0_to_v1` yang memicu error
///    "table dc_home already exists".
/// 2. Pastikan kolom `ipv6` pada tabel `dc_option` memiliki nilai yang valid (bukan string kosong),
///    karena grammers mem-parse kolom tersebut sebagai `SocketAddr`.
pub async fn sanitize_session_db(session_path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let p = Path::new(session_path);
    if !p.exists() || p.metadata().map(|m| m.len()).unwrap_or(0) < 16 {
        return Ok(());
    }

    let conn = match libsql::Builder::new_local(session_path).build().await {
        Ok(b) => match b.connect() {
            Ok(c) => c,
            Err(e) => {
                warn!("Gagal membuka koneksi SQLite untuk sanitasi: {e}");
                return Ok(());
            }
        },
        Err(e) => {
            warn!("Gagal membangun builder SQLite untuk sanitasi: {e}");
            return Ok(());
        }
    };

    // Periksa apakah tabel dc_home sudah ada di database
    let mut table_exists = false;
    if let Ok(mut rows) = conn
        .query(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='dc_home'",
            (),
        )
        .await
    {
        if rows.next().await.ok().flatten().is_some() {
            table_exists = true;
        }
    }

    if table_exists {
        // Set user_version = 1 agar grammers-session menganggap database sudah v1
        let _ = conn.execute("PRAGMA user_version = 1", ()).await;

        // Pastikan ipv6 diisi fallback jika kosong/null (grammers parse sebagai SocketAddr)
        let _ = conn
            .execute(
                "UPDATE dc_option SET ipv6 = '[::1]:443' WHERE ipv6 = '' OR ipv6 IS NULL",
                (),
            )
            .await;
    }

    Ok(())
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
    fn membersihkan_whitespace_newline_dan_petik() {
        let dir = std::env::temp_dir().join("antikarbit_session_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("cleaned.session");
        let _ = std::fs::remove_file(&path);

        let original = b"Hello, Telegram Session!";
        let encoded = B64.encode(original);

        // Simulasi terminal copy-paste: disisipi \r\n, spasi, dan tanda petik
        let dirty = format!("  \"{}\\r\\n  {}  \\n\"  ", &encoded[..10], &encoded[10..])
            .replace("\\r", "\r")
            .replace("\\n", "\n");

        let restored = materialize_from_string(&dirty, path.to_str().unwrap()).unwrap();
        assert!(restored);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn mendukung_base64_url_safe_dan_unpadded() {
        let dir = std::env::temp_dir().join("antikarbit_session_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("urlsafe.session");
        let _ = std::fs::remove_file(&path);

        // Data yang menghasilkan '+' dan '/' dalam standard base64
        let original = vec![251, 239]; // standard: ++8=, url-safe: --8=
        let raw_url_safe = "--8"; // unpadded url safe

        let restored = materialize_from_string(raw_url_safe, path.to_str().unwrap()).unwrap();
        assert!(restored);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn menimpa_file_sesi_jika_kosong_nol_byte() {
        let dir = std::env::temp_dir().join("antikarbit_session_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("empty.session");

        // Tulis file 0 byte
        std::fs::write(&path, b"").unwrap();
        let encoded = B64.encode(b"sesi-baru-dari-env");

        let restored = materialize_from_string(&encoded, path.to_str().unwrap()).unwrap();
        assert!(restored, "file 0-byte harus boleh ditimpa");
        assert_eq!(std::fs::read(&path).unwrap(), b"sesi-baru-dari-env");
    }

    #[test]
    fn tidak_menimpa_sesi_valid_yang_sudah_ada() {
        let dir = std::env::temp_dir().join("antikarbit_session_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("existing.session");

        let existing_data = vec![1u8; 32];
        std::fs::write(&path, &existing_data).unwrap();
        let encoded = B64.encode(b"sesi-baru");

        let restored = materialize_from_string(&encoded, path.to_str().unwrap()).unwrap();
        assert!(!restored, "sesi lama yang valid tidak boleh ditimpa");
        assert_eq!(std::fs::read(&path).unwrap(), existing_data);
    }
}
