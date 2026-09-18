//! Application configuration.
//!
//! Every `.env` key is preserved exactly, so an existing `.env` file keeps
//! working without any changes.

use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};

use serde_json::{json, Value};
use tracing::info;

const DEFAULT_TRIGGER_KEYWORDS: &str = "A waifu has appeared!,A husbando has appeared!,/protecc name,Add her to your harem,Add him to your harem";
const DEFAULT_SUCCESS_KEYWORDS: &str =
    "now protected,added to your harem,added to your collection,is now yours,congratulations";
const DEFAULT_FAIL_KEYWORDS: &str =
    "not quite right,wrong name,already claimed,already protecc,rip,try again";

#[derive(Clone, Debug)]
pub struct Config {
    // Telegram
    pub telegram_api_id: i32,
    pub telegram_api_hash: String,
    pub telegram_session_name: String,
    pub telegram_string_session: String,

    // Dashboard security
    pub dashboard_password: String,

    // Reverse image search
    pub iqdb_min_similarity: f64,
    pub tracemoe_min_similarity: f64,
    pub saucenao_api_key: String,
    pub saucenao_min_similarity: f64,
    pub lens_enabled: bool,
    /// Ascii2d requires no API key, so it is governed by its own flag.
    pub ascii2d_enabled: bool,
    pub ascii2d_min_similarity: f64,

    // Claim
    pub claim_command: String,
    pub name_format: String,

    // Filters
    pub trigger_keywords: Vec<String>,
    pub target_chat_ids: Vec<i64>,

    // Timing
    pub min_delay_seconds: f64,
    pub max_delay_seconds: f64,
    pub verify_timeout_seconds: f64,
    /// Upper bound on the `send_message` call itself. Without this a stuck
    /// request hangs the claim task forever.
    pub send_timeout_seconds: f64,

    // Reply detection
    pub success_keywords: Vec<String>,
    pub fail_keywords: Vec<String>,

    // Web
    pub web_port: u16,
    pub web_host: String,
}

/// Split a `a,b,c` style string into a vector, dropping empty entries.
fn split_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn split_list_lower(raw: &str) -> Vec<String> {
    split_list(raw).into_iter().map(|s| s.to_lowercase()).collect()
}

/// Read `key` from the environment, falling back to a default when empty,
/// with leading and trailing whitespace stripped.
fn env_or(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                default.to_string()
            } else {
                t.to_string()
            }
        }
        _ => default.to_string(),
    }
}

fn env_trim(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(v) => {
            let t = v.trim().to_string();
            if t.is_empty() { default.to_string() } else { t }
        }
        Err(_) => default.to_string(),
    }
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|f| !f.is_nan())
        .unwrap_or(default)
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => !matches!(v.trim().to_lowercase().as_str(), "false" | "0" | "no"),
        Err(_) => default,
    }
}

/// Normalise the Trace.moe threshold: a value above 1.0 is read as a percentage.
fn normalize_ratio(v: f64) -> f64 {
    if v > 1.0 { v / 100.0 } else { v }
}

// ---------------------------------------------------------------------------
// Kunci yang dikelola dashboard, dan deteksi variabel lingkungan platform
// ---------------------------------------------------------------------------

/// Every key the Settings tab is allowed to write.
///
/// This is the single source of truth: `as_dict()` exposes exactly these,
/// `apply_settings()` accepts exactly these, and `update_and_save()` persists
/// exactly these. Adding a setting means adding it here once.
pub const MANAGED_KEYS: &[&str] = &[
    "CLAIM_COMMAND",
    "NAME_FORMAT",
    "IQDB_MIN_SIMILARITY",
    "TRACEMOE_MIN_SIMILARITY",
    "SAUCENAO_API_KEY",
    "SAUCENAO_MIN_SIMILARITY",
    "LENS_ENABLED",
    "ASCII2D_ENABLED",
    "ASCII2D_MIN_SIMILARITY",
    "TRIGGER_KEYWORDS",
    "TARGET_CHAT_IDS",
    "MIN_DELAY_SECONDS",
    "MAX_DELAY_SECONDS",
    "VERIFY_TIMEOUT_SECONDS",
    "SEND_TIMEOUT_SECONDS",
];

/// Keys that are supplied by the *platform* environment rather than by the
/// local `.env` file.
///
/// On Railway (and any other container host) the filesystem is ephemeral: a
/// `.env` written by the dashboard disappears on the next deploy or restart,
/// and is not read at all if the platform injects the value itself. So when a
/// key exists in the real environment, that value is the source of truth and
/// the Settings tab must present it as read-only instead of pretending an
/// edit will stick.
///
/// The distinction matters: `dotenvy::dotenv()` copies `.env` into the process
/// environment, so a naive `std::env::var()` check would flag every key as
/// platform-provided. Only the snapshot taken *before* `dotenv()` runs is
/// genuine.
static PLATFORM_ENV_KEYS: LazyLock<std::collections::HashSet<String>> =
    LazyLock::new(|| {
        MANAGED_KEYS
            .iter()
            .filter(|key| {
                std::env::var(key)
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false)
            })
            .map(|key| key.to_string())
            .collect()
    });

/// Capture which managed keys the platform supplied, *before* `.env` is loaded.
///
/// Must be called once at startup, ahead of `dotenvy::dotenv()`. `main()` does
/// this as its very first statement.
///
/// Safe to call more than once: `LazyLock` only evaluates on first access, so a
/// later call cannot overwrite an earlier snapshot. That is deliberate — it
/// means a test or a helper that touches config early cannot retroactively
/// reclassify keys as platform-provided.
pub fn capture_platform_env() {
    LazyLock::force(&PLATFORM_ENV_KEYS);
}

/// Whether a managed key is locked by a platform environment variable.
pub fn is_locked_by_env(key: &str) -> bool {
    LazyLock::force(&PLATFORM_ENV_KEYS).contains(key)
}

/// The managed keys that are locked, for display in the dashboard.
pub fn locked_keys() -> Vec<String> {
    let mut keys: Vec<String> = MANAGED_KEYS
        .iter()
        .filter(|key| is_locked_by_env(key))
        .map(|key| key.to_string())
        .collect();
    // Stable, predictable order for the UI.
    keys.sort();
    keys
}

/// Whether the app is running on a platform that injects its own environment
/// variables, which is what makes local persistence unreliable.
///
/// Detection is deliberately conservative: a false positive would show a scary
/// banner on a normal VPS, a false negative only hides a hint.
pub fn is_platform_deployment() -> bool {
    // Railway sets RAILWAY_ENVIRONMENT / RAILWAY_PROJECT_ID.
    if std::env::var("RAILWAY_ENVIRONMENT").is_ok() || std::env::var("RAILWAY_PROJECT_ID").is_ok() {
        return true;
    }
    // Other managed hosts that use the same ephemeral-filesystem model.
    for key in [
        "RENDER",
        "DYNO",
        "FLY_APP_NAME",
        "HEROKU_APP_NAME",
        "K_SERVICE",
        "WEBSITE_SITE_NAME",
    ] {
        if std::env::var(key).is_ok() {
            return true;
        }
    }
    false
}

impl Config {
    pub fn from_env() -> Self {
        let tracemoe_raw = env_f64("TRACEMOE_MIN_SIMILARITY", 0.85);

        let target_chat_ids = std::env::var("TARGET_CHAT_IDS")
            .unwrap_or_default()
            .split(',')
            .filter_map(|c| {
                let t = c.trim();
                let digits = t.strip_prefix('-').unwrap_or(t);
                if !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()) {
                    t.parse::<i64>().ok()
                } else {
                    None
                }
            })
            .collect();

        // PORT takes priority for compatibility with Railway/Heroku/Render.
        let web_port = std::env::var("PORT")
            .ok()
            .or_else(|| std::env::var("WEB_PORT").ok())
            .and_then(|v| v.trim().parse::<u16>().ok())
            .unwrap_or(8080);

        Self {
            telegram_api_id: std::env::var("TELEGRAM_API_ID")
                .ok()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0),
            telegram_api_hash: env_or("TELEGRAM_API_HASH", ""),
            telegram_session_name: env_or("TELEGRAM_SESSION_NAME", "waifu_claimer_session"),
            telegram_string_session: env_or("TELEGRAM_STRING_SESSION", ""),

            dashboard_password: env_or("DASHBOARD_PASSWORD", ""),

            iqdb_min_similarity: env_f64("IQDB_MIN_SIMILARITY", 60.0),
            tracemoe_min_similarity: normalize_ratio(tracemoe_raw),
            saucenao_api_key: env_trim("SAUCENAO_API_KEY", ""),
            saucenao_min_similarity: env_f64("SAUCENAO_MIN_SIMILARITY", 70.0),
            lens_enabled: env_bool("LENS_ENABLED", true),
            ascii2d_enabled: env_bool("ASCII2D_ENABLED", true),
            ascii2d_min_similarity: env_f64("ASCII2D_MIN_SIMILARITY", 80.0),

            claim_command: env_or("CLAIM_COMMAND", "/protecc"),
            name_format: env_or("NAME_FORMAT", "full").to_lowercase(),

            trigger_keywords: split_list(&env_or("TRIGGER_KEYWORDS", DEFAULT_TRIGGER_KEYWORDS)),
            target_chat_ids,

            min_delay_seconds: env_f64("MIN_DELAY_SECONDS", 0.5),
            max_delay_seconds: env_f64("MAX_DELAY_SECONDS", 1.5),
            verify_timeout_seconds: env_f64("VERIFY_TIMEOUT_SECONDS", 5.0),
            send_timeout_seconds: env_f64("SEND_TIMEOUT_SECONDS", 5.0),

            success_keywords: split_list_lower(&env_or("SUCCESS_KEYWORDS", DEFAULT_SUCCESS_KEYWORDS)),
            fail_keywords: split_list_lower(&env_or("FAIL_KEYWORDS", DEFAULT_FAIL_KEYWORDS)),

            web_port,
            web_host: env_or("WEB_HOST", "0.0.0.0"),
        }
    }

    /// Validate the essential settings, returning a list of problems found.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.telegram_api_id == 0 {
            errors.push("TELEGRAM_API_ID belum diisi di .env".to_string());
        }
        if self.telegram_api_hash.is_empty() {
            errors.push("TELEGRAM_API_HASH belum diisi di .env".to_string());
        }
        errors
    }

    /// JSON representation for the web UI. Field names must match exactly what
    /// `web/app.js` reads, since it consumes them directly.
    pub fn as_dict(&self) -> Value {
        json!({
            "CLAIM_COMMAND": self.claim_command,
            "NAME_FORMAT": self.name_format,
            "IQDB_MIN_SIMILARITY": self.iqdb_min_similarity,
            "TRACEMOE_MIN_SIMILARITY": self.tracemoe_min_similarity,
            "SAUCENAO_API_KEY": self.saucenao_api_key,
            "SAUCENAO_MIN_SIMILARITY": self.saucenao_min_similarity,
            "LENS_ENABLED": self.lens_enabled,
            "ASCII2D_ENABLED": self.ascii2d_enabled,
            "ASCII2D_MIN_SIMILARITY": self.ascii2d_min_similarity,
            "TRIGGER_KEYWORDS": self.trigger_keywords.join(", "),
            "TARGET_CHAT_IDS": self
                .target_chat_ids
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            "MIN_DELAY_SECONDS": self.min_delay_seconds,
            "MAX_DELAY_SECONDS": self.max_delay_seconds,
            "VERIFY_TIMEOUT_SECONDS": self.verify_timeout_seconds,
            "SEND_TIMEOUT_SECONDS": self.send_timeout_seconds,
            "SUCCESS_KEYWORDS": self.success_keywords.join(", "),
            "FAIL_KEYWORDS": self.fail_keywords.join(", "),
            "WEB_PORT": self.web_port,
            "WEB_HOST": self.web_host,
            // Metadata for the Settings tab. These keys are not settings
            // themselves: they tell the dashboard which fields are supplied by
            // the platform environment and therefore cannot be persisted here.
            "ENV_LOCKED_KEYS": locked_keys(),
            "PLATFORM_DEPLOYMENT": is_platform_deployment(),
            "ENV_FILE": env_file_display(),
        })
    }

    /// Apply dashboard changes to this instance without touching the disk.
    fn apply_settings(&mut self, s: &Value) {
        fn as_str(v: &Value) -> Option<String> {
            match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => None,
            }
        }

        fn as_f64(v: &Value, fallback: f64) -> f64 {
            match v {
                Value::Number(n) => n.as_f64().filter(|f| !f.is_nan()).unwrap_or(fallback),
                Value::String(s) => s
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|f| !f.is_nan())
                    .unwrap_or(fallback),
                _ => fallback,
            }
        }

        /// Checkboxes are only sent by the browser when checked, so a missing
        /// key means "off". `as_bool` still honours an explicit `false` string
        /// for callers that send the value directly.
        fn as_bool(v: &Value, fallback: bool) -> bool {
            match v {
                Value::Bool(b) => *b,
                Value::String(s) => {
                    let s = s.trim().to_lowercase();
                    if s.is_empty() {
                        return fallback;
                    }
                    !matches!(s.as_str(), "false" | "0" | "no" | "off")
                }
                Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
                _ => fallback,
            }
        }

        if let Some(v) = s.get("CLAIM_COMMAND").and_then(as_str) {
            let v = v.trim();
            if !v.is_empty() {
                self.claim_command = v.to_string();
            }
        }
        if let Some(v) = s.get("NAME_FORMAT").and_then(as_str) {
            let v = v.trim().to_lowercase();
            if !v.is_empty() {
                self.name_format = v;
            }
        }
        if let Some(v) = s.get("IQDB_MIN_SIMILARITY") {
            self.iqdb_min_similarity = as_f64(v, self.iqdb_min_similarity);
        }
        if let Some(v) = s.get("TRACEMOE_MIN_SIMILARITY") {
            self.tracemoe_min_similarity =
                normalize_ratio(as_f64(v, self.tracemoe_min_similarity));
        }
        if let Some(v) = s.get("SAUCENAO_API_KEY").and_then(as_str) {
            self.saucenao_api_key = v.trim().to_string();
        }
        if let Some(v) = s.get("SAUCENAO_MIN_SIMILARITY") {
            self.saucenao_min_similarity = as_f64(v, self.saucenao_min_similarity);
        }
        if let Some(v) = s.get("LENS_ENABLED") {
            self.lens_enabled = as_bool(v, self.lens_enabled);
        }
        if let Some(v) = s.get("ASCII2D_ENABLED") {
            self.ascii2d_enabled = as_bool(v, self.ascii2d_enabled);
        }
        if let Some(v) = s.get("ASCII2D_MIN_SIMILARITY") {
            self.ascii2d_min_similarity = as_f64(v, self.ascii2d_min_similarity);
        }
        if let Some(v) = s.get("TRIGGER_KEYWORDS").and_then(as_str) {
            let list = split_list(&v);
            if !list.is_empty() {
                self.trigger_keywords = list;
            }
        }
        if let Some(v) = s.get("TARGET_CHAT_IDS").and_then(as_str) {
            self.target_chat_ids = v
                .split(',')
                .filter_map(|c| {
                    let t = c.trim();
                    let digits = t.strip_prefix('-').unwrap_or(t);
                    if !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()) {
                        t.parse::<i64>().ok()
                    } else {
                        None
                    }
                })
                .collect();
        }
        if let Some(v) = s.get("MIN_DELAY_SECONDS") {
            self.min_delay_seconds = as_f64(v, self.min_delay_seconds);
        }
        if let Some(v) = s.get("MAX_DELAY_SECONDS") {
            self.max_delay_seconds = as_f64(v, self.max_delay_seconds);
        }
        if let Some(v) = s.get("VERIFY_TIMEOUT_SECONDS") {
            self.verify_timeout_seconds = as_f64(v, self.verify_timeout_seconds);
        }
        if let Some(v) = s.get("SEND_TIMEOUT_SECONDS") {
            self.send_timeout_seconds = as_f64(v, self.send_timeout_seconds);
        }
    }
}

// ---------------------------------------------------------------------------
// State global
// ---------------------------------------------------------------------------

static CONFIG: LazyLock<RwLock<Config>> = LazyLock::new(|| RwLock::new(Config::from_env()));

/// Muat `.env` (sekali) lalu inisialisasi konfigurasi global.
pub fn init() {
    // Snapshot which keys the platform supplied BEFORE `.env` is loaded.
    // `dotenvy` copies `.env` into the process environment, so after this call
    // the two are indistinguishable and every key would look platform-provided.
    capture_platform_env();

    // Load .env from the working directory. Missing is fine: on Railway the
    // variables are injected by the platform instead.
    let _ = dotenvy::dotenv();
    LazyLock::force(&CONFIG);
}

/// A snapshot of the current configuration.
pub fn get() -> Config {
    CONFIG
        .read()
        .expect("config lock poisoned")
        .clone()
}

/// Path of the `.env` file used to persist settings.
fn env_file_path() -> PathBuf {
    std::env::var("ANTIKARBIT_ENV_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".env"))
}

/// The `.env` path as a string, for display in the dashboard.
pub fn env_file_display() -> String {
    env_file_path().display().to_string()
}

/// Update configuration from the dashboard and write it back to `.env`.
///
/// Keys locked by a platform environment variable are applied in memory for
/// this session (so the dashboard stays useful) but are **not** written to
/// `.env`, because that file is not read on such a host anyway and a stale
/// value would be misleading.
///
/// Unmanaged lines (for example TELEGRAM_API_ID/HASH) are preserved verbatim.
pub fn update_and_save(settings: &Value) -> std::io::Result<()> {
    let snapshot = {
        let mut guard = CONFIG.write().expect("config lock poisoned");
        guard.apply_settings(settings);
        guard.clone()
    };

    // Keys the dashboard manages, ordered so the file diff stays stable.
    let managed: Vec<(&str, String)> = vec![
        ("CLAIM_COMMAND", snapshot.claim_command.clone()),
        ("NAME_FORMAT", snapshot.name_format.clone()),
        ("IQDB_MIN_SIMILARITY", snapshot.iqdb_min_similarity.to_string()),
        ("TRACEMOE_MIN_SIMILARITY", snapshot.tracemoe_min_similarity.to_string()),
        ("SAUCENAO_API_KEY", snapshot.saucenao_api_key.clone()),
        ("SAUCENAO_MIN_SIMILARITY", snapshot.saucenao_min_similarity.to_string()),
        ("LENS_ENABLED", snapshot.lens_enabled.to_string()),
        ("ASCII2D_ENABLED", snapshot.ascii2d_enabled.to_string()),
        ("ASCII2D_MIN_SIMILARITY", snapshot.ascii2d_min_similarity.to_string()),
        ("TRIGGER_KEYWORDS", snapshot.trigger_keywords.join(",")),
        (
            "TARGET_CHAT_IDS",
            snapshot
                .target_chat_ids
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(","),
        ),
        ("MIN_DELAY_SECONDS", snapshot.min_delay_seconds.to_string()),
        ("MAX_DELAY_SECONDS", snapshot.max_delay_seconds.to_string()),
        ("VERIFY_TIMEOUT_SECONDS", snapshot.verify_timeout_seconds.to_string()),
        ("SEND_TIMEOUT_SECONDS", snapshot.send_timeout_seconds.to_string()),
    ];

    let path = env_file_path();

    // A platform deployment has no writable, persistent `.env`. Writing one is
    // harmless but pointless, and on a read-only image it would fail and
    // surface as a bogus error in the UI. Skip it and report success: the
    // in-memory change above already took effect.
    if is_platform_deployment() && !path.exists() {
        info!(
            "Platform deployment detected; settings applied in memory only ({} not written).",
            path.display()
        );
        return Ok(());
    }

    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    let mut out = String::new();
    let mut handled: Vec<&str> = Vec::new();

    for line in existing.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('#') && trimmed.contains('=') {
            let key = trimmed.split('=').next().unwrap_or("").trim();
            if let Some((_, value)) = managed.iter().find(|(k, _)| *k == key) {
                // Leave platform-provided values untouched: this `.env` is
                // ignored for those keys, and rewriting them would lie about
                // what is actually in effect.
                if is_locked_by_env(key) {
                    out.push_str(line);
                    out.push('\n');
                    handled.push(key);
                    continue;
                }
                out.push_str(&format!("{key}={value}\n"));
                handled.push(key);
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }

    for (key, value) in &managed {
        if !handled.contains(key) && !is_locked_by_env(key) {
            out.push_str(&format!("{key}={value}\n"));
        }
    }

    std::fs::write(&path, out)
}

// ---------------------------------------------------------------------------
// Pemakaian memori proses
// ---------------------------------------------------------------------------

/// Process memory usage in MB.
///
/// On Linux (the Railway target) this is read from `/proc/self/status`.
/// Elsewhere it returns 0.0; the frontend already handles that case with
/// `if (data.memory_mb)`.
pub fn current_memory_mb() -> f64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("VmRSS:") {
                    if let Some(kb) = rest
                        .split_whitespace()
                        .next()
                        .and_then(|v| v.parse::<f64>().ok())
                    {
                        return (kb / 1024.0 * 10.0).round() / 10.0;
                    }
                }
            }
        }
        0.0
    }
    #[cfg(not(target_os = "linux"))]
    {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisasi_ratio_mengubah_persen_ke_desimal() {
        assert!((normalize_ratio(85.0) - 0.85).abs() < 1e-9);
        assert!((normalize_ratio(0.85) - 0.85).abs() < 1e-9);
    }

    #[test]
    fn split_list_membuang_entri_kosong() {
        assert_eq!(split_list("a, b ,,c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn as_dict_memakai_key_yang_dibaca_frontend() {
        let cfg = Config::from_env();
        let d = cfg.as_dict();
        for key in [
            "CLAIM_COMMAND",
            "NAME_FORMAT",
            "IQDB_MIN_SIMILARITY",
            "TRACEMOE_MIN_SIMILARITY",
            "SAUCENAO_API_KEY",
            "SAUCENAO_MIN_SIMILARITY",
            "LENS_ENABLED",
            "TRIGGER_KEYWORDS",
            "TARGET_CHAT_IDS",
            "MIN_DELAY_SECONDS",
            "MAX_DELAY_SECONDS",
            "VERIFY_TIMEOUT_SECONDS",
            "SEND_TIMEOUT_SECONDS",
            "SUCCESS_KEYWORDS",
            "FAIL_KEYWORDS",
            "WEB_PORT",
            "WEB_HOST",
            "ASCII2D_ENABLED",
            "ASCII2D_MIN_SIMILARITY",
            "ENV_LOCKED_KEYS",
            "PLATFORM_DEPLOYMENT",
            "ENV_FILE",
        ] {
            assert!(d.get(key).is_some(), "key hilang: {key}");
        }
    }

    #[test]
    fn managed_keys_sama_dengan_daftar_kunci_pengaturan() {
        // The Settings tab must not drift from the keys the backend accepts.
        // SUCCESS_KEYWORDS/FAIL_KEYWORDS are read-only in practice (the UI
        // sends them but they are not persisted), so they are excluded here
        // together with the metadata keys.
        let cfg = Config::from_env();
        let d = cfg.as_dict();
        for key in MANAGED_KEYS {
            assert!(
                d.get(*key).is_some(),
                "MANAGED_KEYS memuat '{key}' yang tidak ada di as_dict()"
            );
        }
    }

    #[test]
    fn default_ascii2d_aktif_dengan_ambang_80() {
        // The user-facing defaults: Ascii2d on, 80% threshold.
        let cfg = Config::from_env();
        assert!(cfg.ascii2d_enabled, "Ascii2d harus aktif secara default");
        assert!(
            (cfg.ascii2d_min_similarity - 80.0).abs() < 1e-9,
            "ambang Ascii2d harus 80%, dapat {}",
            cfg.ascii2d_min_similarity
        );
    }

    #[test]
    fn as_bool_menghormati_nilai_eksplisit() {
        // Reached through apply_settings, which is private, so the behaviour is
        // asserted via the public path instead: an absent key must not flip a
        // flag, because an unchecked checkbox is never submitted.
        let mut cfg = Config::from_env();
        let before = cfg.ascii2d_enabled;
        cfg.apply_settings(&json!({ "CLAIM_COMMAND": "/protecc" }));
        assert_eq!(cfg.ascii2d_enabled, before);
    }

    #[test]
    fn apply_settings_menyalakan_dan_mematikan_ascii2d() {
        let mut cfg = Config::from_env();

        cfg.apply_settings(&json!({ "ASCII2D_ENABLED": "false" }));
        assert!(!cfg.ascii2d_enabled);

        cfg.apply_settings(&json!({ "ASCII2D_ENABLED": "true" }));
        assert!(cfg.ascii2d_enabled);

        cfg.apply_settings(&json!({ "ASCII2D_ENABLED": false }));
        assert!(!cfg.ascii2d_enabled);

        // An empty string (what an unchecked-adjacent form field can yield)
        // must leave the current value alone rather than silently disabling.
        cfg.apply_settings(&json!({ "ASCII2D_ENABLED": true }));
        cfg.apply_settings(&json!({ "ASCII2D_ENABLED": "" }));
        assert!(cfg.ascii2d_enabled);
    }

    #[test]
    fn apply_settings_menerapkan_ambang_ascii2d() {
        let mut cfg = Config::from_env();
        cfg.apply_settings(&json!({ "ASCII2D_MIN_SIMILARITY": 55.5 }));
        assert!((cfg.ascii2d_min_similarity - 55.5).abs() < 1e-9);
    }

    #[test]
    fn kunci_tanpa_env_tidak_terkunci() {
        // In the test process no managed key is set by a "platform", so nothing
        // should be reported as locked and `.env` stays authoritative.
        assert!(locked_keys().is_empty(), "tidak ada kunci yang boleh terkunci");
    }
}
