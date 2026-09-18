//! Web dashboard built on axum.
//!
//! The HTTP contract here is **locked down by integration tests**, because
//! `web/app.js` reads it directly: route names, JSON field names, and SSE event
//! names. Changing any of them without updating the frontend breaks the
//! dashboard silently.

pub mod logs;

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{ConnectInfo, Form, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use futures_util::stream::Stream;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use tower_http::services::ServeDir;
use tracing::{info, warn};

use askama::Template;
use crate::config;
use crate::core::WaifuListener;
use logs::LogBuffer;

// --- Batas keamanan ---
const BRUTE_FORCE_WINDOW_SEC: f64 = 60.0;
const BRUTE_FORCE_MAX_ATTEMPTS: usize = 5;
const API_RATE_WINDOW_SEC: f64 = 10.0;
const API_RATE_MAX_REQUESTS: usize = 60;
const COOKIE_NAME: &str = "ak_session";

#[derive(Clone)]
pub struct AppState {
    /// `None` until Telegram is connected. The dashboard keeps serving requests
    /// so a connection error can be inspected in the browser instead of the
    /// container dying with no trace.
    listener: Arc<std::sync::RwLock<Option<Arc<WaifuListener>>>>,
    /// The claim event bus. Owned by the application so SSE clients can subscribe
    /// from the start, before the Telegram bot has connected.
    events_tx: broadcast::Sender<Value>,
    pub me_info: Arc<Mutex<Value>>,
    pub logs: Arc<LogBuffer>,
    sessions: Arc<Mutex<HashSet<String>>>,
    login_attempts: Arc<Mutex<HashMap<String, Vec<f64>>>>,
    api_requests: Arc<Mutex<HashMap<String, Vec<f64>>>>,
}

impl AppState {
    pub fn new(logs: Arc<LogBuffer>) -> Self {
        let (events_tx, _) = broadcast::channel(256);
        Self {
            listener: Arc::new(std::sync::RwLock::new(None)),
            events_tx,
            me_info: Arc::new(Mutex::new(json!({}))),
            logs,
            sessions: Arc::new(Mutex::new(HashSet::new())),
            login_attempts: Arc::new(Mutex::new(HashMap::new())),
            api_requests: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The event sender to hand to [`WaifuListener`].
    pub fn events_sender(&self) -> broadcast::Sender<Value> {
        self.events_tx.clone()
    }

    /// Install the listener once Telegram has connected.
    pub fn set_listener(&self, listener: Arc<WaifuListener>) {
        if let Ok(mut slot) = self.listener.write() {
            *slot = Some(listener);
        }
    }

    /// The current listener, if one has been installed yet.
    pub fn listener(&self) -> Option<Arc<WaifuListener>> {
        self.listener.read().ok().and_then(|slot| slot.clone())
    }

    pub fn set_me(&self, name: &str, username: Option<&str>, id: i64) {
        if let Ok(mut me) = self.me_info.lock() {
            *me = json!({ "name": name, "username": username, "id": id });
        }
    }
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// The client's real IP, honouring reverse proxy headers.
fn client_ip(headers: &HeaderMap, connect: Option<&ConnectInfo<SocketAddr>>) -> String {
    if let Some(v) = headers.get("CF-Connecting-IP").and_then(|v| v.to_str().ok()) {
        return v.trim().to_string();
    }
    if let Some(v) = headers.get("X-Forwarded-For").and_then(|v| v.to_str().ok())
        && let Some(first) = v.split(',').next() {
            return first.trim().to_string();
        }
    if let Some(v) = headers.get("X-Real-IP").and_then(|v| v.to_str().ok()) {
        return v.trim().to_string();
    }
    connect
        .map(|c| c.0.ip().to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

fn sha256_hex(input: &str) -> String {
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Constant-time comparison.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn random_token() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    let bytes: [u8; 32] = rng.random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn is_protected() -> bool {
    !config::get().dashboard_password.trim().is_empty()
}

fn session_token(jar: &CookieJar) -> Option<String> {
    jar.get(COOKIE_NAME).map(|c| c.value().to_string())
}

fn is_authenticated(state: &AppState, jar: &CookieJar) -> bool {
    if !is_protected() {
        return true;
    }
    let Some(token) = session_token(jar) else {
        return false;
    };
    state
        .sessions
        .lock()
        .map(|s| s.contains(&token))
        .unwrap_or(false)
}

/// Validate Origin/Referer on state-changing requests.
fn is_valid_origin(headers: &HeaderMap, port: u16) -> bool {
    let origin = headers
        .get("Origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim()
        .to_string();
    let referer = headers
        .get("Referer")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim()
        .to_string();

    // Non-browser clients (curl, internal scripts) send neither.
    if origin.is_empty() && referer.is_empty() {
        return true;
    }

    let mut valid_hosts: HashSet<String> = HashSet::new();
    valid_hosts.insert("localhost".into());
    valid_hosts.insert("127.0.0.1".into());

    if let Some(host) = headers.get("Host").and_then(|v| v.to_str().ok()) {
        let host = host.trim().to_lowercase();
        if !host.is_empty() {
            valid_hosts.insert(host.clone());
            valid_hosts.insert(host.split(':').next().unwrap_or("").to_string());
        }
    }
    if let Some(host) = headers.get("X-Forwarded-Host").and_then(|v| v.to_str().ok()) {
        let host = host.trim().to_lowercase();
        if !host.is_empty() {
            valid_hosts.insert(host.clone());
            valid_hosts.insert(host.split(':').next().unwrap_or("").to_string());
        }
    }

    for candidate in [origin, referer] {
        if candidate.is_empty() {
            continue;
        }
        // Simple parsing: take the authority section after the scheme.
        let Some(rest) = candidate.split("://").nth(1) else {
            continue;
        };
        let authority = rest.split('/').next().unwrap_or("").to_lowercase();
        if authority.is_empty() {
            continue;
        }
        if valid_hosts.contains(&authority) {
            return true;
        }
        let hostname = authority.split(':').next().unwrap_or("");
        if valid_hosts.contains(hostname) {
            return true;
        }
        // Port default di belakang reverse proxy.
        if (authority == format!("localhost:{port}") || authority == format!("127.0.0.1:{port}"))
            && !authority.is_empty()
        {
            return true;
        }
    }

    false
}

fn is_api_rate_limited(state: &AppState, ip: &str) -> bool {
    let now = unix_now();
    let Ok(mut log) = state.api_requests.lock() else {
        return false;
    };

    let mut entries = log.get(ip).cloned().unwrap_or_default();
    entries.retain(|t| now - t < API_RATE_WINDOW_SEC);

    // Drop stale entries so the map does not grow unbounded.
    if log.len() > 500 {
        log.retain(|_, v| v.last().map(|t| now - t < API_RATE_WINDOW_SEC).unwrap_or(false));
    }

    if entries.len() >= API_RATE_MAX_REQUESTS {
        log.insert(ip.to_string(), entries);
        return true;
    }

    entries.push(now);
    log.insert(ip.to_string(), entries);
    false
}

fn is_brute_forced(state: &AppState, ip: &str) -> bool {
    let now = unix_now();
    let Ok(mut map) = state.login_attempts.lock() else {
        return false;
    };
    let mut attempts = map.get(ip).cloned().unwrap_or_default();
    attempts.retain(|t| now - t < BRUTE_FORCE_WINDOW_SEC);
    let blocked = attempts.len() >= BRUTE_FORCE_MAX_ATTEMPTS;
    map.insert(ip.to_string(), attempts);
    blocked
}

fn record_failed_attempt(state: &AppState, ip: &str) -> usize {
    let Ok(mut map) = state.login_attempts.lock() else {
        return 0;
    };
    let entry = map.entry(ip.to_string()).or_default();
    entry.push(unix_now());
    entry.len()
}

/// Middleware: access control, CSRF, rate limiting, security headers.
async fn guard(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    let method = request.method().clone();
    let headers = request.headers().clone();
    let connect = ConnectInfo(addr);

    let is_static = path == "/"
        || path == "/api/auth/login"
        || path == "/htmx/login"
        || path.ends_with(".css")
        || path.ends_with(".js")
        || path.ends_with(".png")
        || path.ends_with(".ico")
        || path.ends_with(".woff2")
        || path.ends_with(".woff")
        || path.ends_with(".ttf");

    let ip = client_ip(&headers, Some(&connect));

    if !is_static && path.starts_with("/api/") && is_api_rate_limited(&state, &ip) {
        warn!("API rate limit exceeded from {ip} on {path}");
        let mut response = Json(json!({
            "error": "Too many requests. Please wait a moment.",
            "retry_after": API_RATE_WINDOW_SEC,
        }))
        .into_response();
        *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
        response.headers_mut().insert(
            "Retry-After",
            API_RATE_WINDOW_SEC.to_string().parse().unwrap(),
        );
        return with_security_headers(response);
    }

    if matches!(method, Method::POST | Method::PUT | Method::DELETE | Method::PATCH)
        && path != "/api/auth/login"
        && path != "/htmx/login"
        && !is_valid_origin(&headers, config::get().web_port)
    {
        warn!("CSRF check blocked a request from {ip} on {path}");
        let mut response = Json(json!({
            "error": "Invalid request (CSRF origin check failed)."
        }))
        .into_response();
        *response.status_mut() = StatusCode::FORBIDDEN;
        return with_security_headers(response);
    }

    let jar = CookieJar::from_headers(&headers);
    let authenticated = is_authenticated(&state, &jar);

    let response = if is_static || authenticated {
        next.run(request).await
    } else {
        let mut response = Json(json!({
            "authenticated": false,
            "message": "Authentication required",
        }))
        .into_response();
        *response.status_mut() = StatusCode::UNAUTHORIZED;
        response
    };

    with_security_headers(response)
}

fn with_security_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    headers.insert("X-Frame-Options", "DENY".parse().unwrap());
    headers.insert("X-XSS-Protection", "1; mode=block".parse().unwrap());
    headers.insert(
        "Referrer-Policy",
        "strict-origin-when-cross-origin".parse().unwrap(),
    );
    headers.insert("Cache-Control", "no-store".parse().unwrap());
    response
}

// --- Handler autentikasi ---

async fn auth_login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Option<Json<Value>>,
) -> Response {
    if !is_protected() {
        return Json(json!({
            "authenticated": true,
            "message": "No password configured",
        }))
        .into_response();
    }

    let ip = client_ip(&headers, Some(&ConnectInfo(addr)));
    if is_brute_forced(&state, &ip) {
        warn!("Login blocked (brute force) from {ip}");
        let mut response = Json(json!({
            "authenticated": false,
            "message": "Too many failed attempts. Wait 60 seconds.",
        }))
        .into_response();
        *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
        return response;
    }

    let password = body
        .and_then(|Json(v)| v.get("password").and_then(|p| p.as_str()).map(String::from))
        .unwrap_or_default();

    let expected = sha256_hex(&config::get().dashboard_password);
    let given = sha256_hex(&password);

    if constant_time_eq(&expected, &given) {
        if let Ok(mut attempts) = state.login_attempts.lock() {
            attempts.remove(&ip);
        }
        let token = random_token();
        if let Ok(mut sessions) = state.sessions.lock() {
            sessions.insert(token.clone());
        }
        info!("Dashboard login succeeded from {ip}");

        let cookie = Cookie::build((COOKIE_NAME, token))
            .http_only(true)
            .same_site(SameSite::Strict)
            .path("/")
            .max_age(time::Duration::seconds(86_400 * 7))
            .build();

        (jar.add(cookie), Json(json!({ "authenticated": true, "message": "Login berhasil" })))
            .into_response()
    } else {
        let count = record_failed_attempt(&state, &ip);
        let remaining = BRUTE_FORCE_MAX_ATTEMPTS.saturating_sub(count);
        warn!("Failed dashboard login from {ip} ({remaining} attempts remaining)");
        let mut response = Json(json!({
            "authenticated": false,
            "message": format!("Wrong password. Attempts remaining: {remaining}"),
        }))
        .into_response();
        *response.status_mut() = StatusCode::UNAUTHORIZED;
        response
    }
}

async fn auth_logout(State(state): State<AppState>, jar: CookieJar) -> Response {
    if let Some(token) = session_token(&jar)
        && let Ok(mut sessions) = state.sessions.lock() {
            sessions.remove(&token);
        }
    (
        jar.remove(Cookie::from(COOKIE_NAME)),
        Json(json!({ "success": true, "message": "Logout berhasil" })),
    )
        .into_response()
}

async fn auth_check(State(state): State<AppState>, jar: CookieJar) -> Response {
    Json(json!({
        "authenticated": is_authenticated(&state, &jar),
        "protected": is_protected(),
    }))
    .into_response()
}

// --- Handler dashboard ---

async fn status(State(state): State<AppState>) -> Response {
    let cfg = config::get();
    let user = state
        .me_info
        .lock()
        .map(|m| m.clone())
        .unwrap_or_else(|_| json!({}));

    let listener = state.listener();

    let is_active = listener
        .as_ref()
        .map(|l| l.is_active.load(std::sync::atomic::Ordering::Relaxed))
        .unwrap_or(false);

    let stats = listener
        .as_ref()
        .map(|l| l.stats.to_json())
        .unwrap_or_else(|| json!({ "detected": 0, "claimed": 0, "failed": 0, "start_time": 0.0 }));

    let (cache_entries, cache_hits, cache_misses) = listener
        .as_ref()
        .map(|l| l.cache_stats())
        .unwrap_or((0, 0, 0));

    let lookups = cache_hits + cache_misses;
    let hit_rate = if lookups > 0 {
        (cache_hits as f64 / lookups as f64 * 1000.0).round() / 10.0
    } else {
        0.0
    };

    let engines = listener
        .as_ref()
        .map(|l| l.active_engines())
        .unwrap_or_default();

    Json(json!({
        "status": "online",
        "bot_ready": listener.is_some(),
        "is_active": is_active,
        "memory_mb": config::current_memory_mb(),
        "user": user,
        "stats": stats,
        "cache": {
            "entries": cache_entries,
            "hits": cache_hits,
            "misses": cache_misses,
            "hit_rate": hit_rate,
        },
        "engines": engines,
        "config": cfg.as_dict(),
    }))
    .into_response()
}

/// A uniform response for when the Telegram bot is not ready.
fn bot_not_ready() -> Response {
    let mut response = Json(json!({
        "success": false,
        "bot_ready": false,
        "message": "Telegram bot is not connected. Check the Live Console for details.",
    }))
    .into_response();
    *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    response
}

async fn toggle_bot(State(state): State<AppState>) -> Response {
    let Some(listener) = state.listener() else {
        return bot_not_ready();
    };

    let now_active = listener.toggle();
    let state_str = if now_active { "active" } else { "paused" };
    info!("Bot listener is now {state_str}");
    Json(json!({
        "success": true,
        "is_active": now_active,
        "message": format!("Bot {state_str}"),
    }))
    .into_response()
}

async fn settings_get() -> Response {
    Json(config::get().as_dict()).into_response()
}

async fn settings_post(Json(body): Json<Value>) -> Response {
    match config::update_and_save(&body) {
        Ok(()) => {
            info!("Settings updated from the web dashboard.");
            Json(json!({ "success": true, "config": config::get().as_dict() })).into_response()
        }
        Err(e) => {
            warn!("Could not write .env: {e}");
            let mut response =
                Json(json!({ "success": false, "message": "Failed to write the .env file" }))
                    .into_response();
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            response
        }
    }
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    // History first, so the live console is not empty when it opens.
    let mut pending: Vec<(String, String)> = Vec::new();
    for entry in state.logs.history() {
        pending.push(("log".to_string(), entry.to_string()));
    }
    if let Some(listener) = state.listener() {
        for event in listener.recent_events() {
            pending.push(("claim_event".to_string(), event.to_string()));
        }
    }

    let log_rx = state.logs.subscribe();
    let claim_rx = state.events_tx.subscribe();

    let stream = futures_util::stream::unfold(
        (pending, log_rx, claim_rx),
        |(mut pending, mut log_rx, mut claim_rx)| async move {
            if !pending.is_empty() {
                let (name, data) = pending.remove(0);
                return Some((
                    Ok(Event::default().event(name).data(data)),
                    (pending, log_rx, claim_rx),
                ));
            }

            loop {
                tokio::select! {
                    result = log_rx.recv() => match result {
                        Ok(value) => {
                            return Some((
                                Ok(Event::default().event("log").data(value.to_string())),
                                (pending, log_rx, claim_rx),
                            ));
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return None,
                    },
                    result = claim_rx.recv() => match result {
                        Ok(value) => {
                            return Some((
                                Ok(Event::default().event("claim_event").data(value.to_string())),
                                (pending, log_rx, claim_rx),
                            ));
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return None,
                    },
                }
            }
        },
    );

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)),
    )
}

// ===========================================================================
// Askama + HTMX Templates & Handlers
// ===========================================================================

pub struct HtmlTemplate<T>(pub T);

impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => axum::response::Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {err}"),
            )
                .into_response(),
        }
    }
}

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub authenticated: bool,
    pub is_protected: bool,
    /// Login error to display, empty when there is none.
    pub login_error: String,
    pub is_active: bool,
    pub bot_ready: bool,
    pub user_name: String,
    pub user_tag: String,
    pub memory_mb: f64,
    pub detected: u64,
    pub claimed: u64,
    pub failed: u64,
    pub success_rate: f64,
    pub claim_command: String,
    pub name_format: String,
    pub iqdb_similarity: f64,
    pub tracemoe_similarity: f64,
    pub saucenao_similarity: f64,
    pub saucenao_key: String,
    pub lens_enabled: bool,
    pub triggers: String,
    pub target_chats: String,
    pub min_delay: f64,
    pub max_delay: f64,
    pub verify_timeout: f64,
    pub send_timeout: f64,
    pub success_keywords: String,
    pub fail_keywords: String,
}

#[derive(Template)]
#[template(path = "partials/bot_toggle.html")]
pub struct BotToggleTemplate {
    pub is_active: bool,
    pub bot_ready: bool,
}

#[derive(Template)]
#[template(path = "partials/stats_cards.html")]
pub struct StatsCardsTemplate {
    pub detected: u64,
    pub claimed: u64,
    pub failed: u64,
    pub success_rate: f64,
    pub memory_mb: f64,
}

#[derive(Template)]
#[template(path = "partials/settings_toast.html")]
pub struct SettingsToastTemplate {
    pub message: String,
}

async fn index_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let cfg = config::get();
    let auth = is_authenticated(&state, &jar);
    let protected = is_protected();

    // Login feedback travels as a query param to keep this stateless.
    let login_error = match params.get("login").map(|s| s.as_str()) {
        Some("failed") => "Wrong password. Please try again.".to_string(),
        Some("blocked") => {
            "Too many failed attempts. Wait 60 seconds and try again.".to_string()
        }
        _ => String::new(),
    };

    let user = state
        .me_info
        .lock()
        .map(|m| m.clone())
        .unwrap_or_else(|_| json!({}));
    let user_name = user
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("Telegram User")
        .to_string();
    let user_tag = user
        .get("username")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();

    let listener = state.listener();
    let is_active = listener
        .as_ref()
        .map(|l| l.is_active.load(std::sync::atomic::Ordering::Relaxed))
        .unwrap_or(false);
    let bot_ready = listener.is_some();

    let (detected, claimed, failed, success_rate) = if let Some(l) = listener.as_ref() {
        let det = l.stats.detected.load(std::sync::atomic::Ordering::Relaxed);
        let cla = l.stats.claimed.load(std::sync::atomic::Ordering::Relaxed);
        let fai = l.stats.failed.load(std::sync::atomic::Ordering::Relaxed);
        let rate = if det > 0 {
            (cla as f64 / det as f64 * 1000.0).round() / 10.0
        } else {
            100.0
        };
        (det, cla, fai, rate)
    } else {
        (0, 0, 0, 100.0)
    };

    HtmlTemplate(IndexTemplate {
        authenticated: auth,
        is_protected: protected,
        login_error,
        is_active,
        bot_ready,
        user_name,
        user_tag,
        memory_mb: config::current_memory_mb(),
        detected,
        claimed,
        failed,
        success_rate,
        claim_command: cfg.claim_command,
        name_format: cfg.name_format,
        iqdb_similarity: cfg.iqdb_min_similarity,
        tracemoe_similarity: (cfg.tracemoe_min_similarity * 100.0).round(),
        saucenao_similarity: cfg.saucenao_min_similarity,
        saucenao_key: cfg.saucenao_api_key.clone(),
        lens_enabled: cfg.lens_enabled,
        triggers: cfg.trigger_keywords.join(", "),
        target_chats: cfg
            .target_chat_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", "),
        min_delay: cfg.min_delay_seconds,
        max_delay: cfg.max_delay_seconds,
        verify_timeout: cfg.verify_timeout_seconds,
        send_timeout: cfg.send_timeout_seconds,
        success_keywords: cfg.success_keywords.join(", "),
        fail_keywords: cfg.fail_keywords.join(", "),
    })
    .into_response()
}

async fn htmx_toggle(State(state): State<AppState>) -> Response {
    let Some(listener) = state.listener() else {
        return HtmlTemplate(BotToggleTemplate {
            is_active: false,
            bot_ready: false,
        })
        .into_response();
    };

    let now_active = listener.toggle();
    let state_str = if now_active { "AKTIF" } else { "DIJEDA (PAUSED)" };
    info!("Bot listener diubah menjadi: {state_str} (via HTMX)");
    HtmlTemplate(BotToggleTemplate {
        is_active: now_active,
        bot_ready: true,
    })
    .into_response()
}

async fn htmx_stats(State(state): State<AppState>) -> Response {
    let listener = state.listener();
    let (detected, claimed, failed, success_rate) = if let Some(l) = listener.as_ref() {
        let det = l.stats.detected.load(std::sync::atomic::Ordering::Relaxed);
        let cla = l.stats.claimed.load(std::sync::atomic::Ordering::Relaxed);
        let fai = l.stats.failed.load(std::sync::atomic::Ordering::Relaxed);
        let rate = if det > 0 {
            (cla as f64 / det as f64 * 1000.0).round() / 10.0
        } else {
            100.0
        };
        (det, cla, fai, rate)
    } else {
        (0, 0, 0, 100.0)
    };

    HtmlTemplate(StatsCardsTemplate {
        detected,
        claimed,
        failed,
        success_rate,
        memory_mb: config::current_memory_mb(),
    })
    .into_response()
}

async fn htmx_settings(Form(form): Form<HashMap<String, String>>) -> Response {
    let mut map = serde_json::Map::new();
    for (k, v) in form {
        map.insert(k, Value::String(v));
    }
    let body = Value::Object(map);

    match config::update_and_save(&body) {
        Ok(()) => {
            info!("Settings updated from the settings form.");
            HtmlTemplate(SettingsToastTemplate {
                message: "Settings saved to .env and applied immediately.".to_string(),
            })
            .into_response()
        }
        Err(e) => {
            warn!("Could not write .env: {e}");
            HtmlTemplate(SettingsToastTemplate {
                message: format!("Could not save settings: {e}"),
            })
            .into_response()
        }
    }
}

async fn htmx_login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let ip = client_ip(&headers, Some(&ConnectInfo(addr)));

    // Brute-force protection equivalent to the API endpoint. Without it,
    // /htmx/login could be guessed without limit, because the rate limiter
    // only covers /api/ paths.
    if is_brute_forced(&state, &ip) {
        warn!("Dashboard login blocked (brute force) from {ip}");
        return Redirect::to("/?login=blocked").into_response();
    }

    let password = form.get("password").map(|s| s.as_str()).unwrap_or_default();
    let expected = sha256_hex(&config::get().dashboard_password);
    let given = sha256_hex(password);

    if constant_time_eq(&expected, &given) {
        if let Ok(mut attempts) = state.login_attempts.lock() {
            attempts.remove(&ip);
        }
        let token = random_token();
        if let Ok(mut sessions) = state.sessions.lock() {
            sessions.insert(token.clone());
        }
        info!("Dashboard login succeeded from {ip}");
        let cookie = Cookie::build((COOKIE_NAME, token))
            .http_only(true)
            .same_site(SameSite::Strict)
            .path("/")
            .max_age(time::Duration::seconds(86_400 * 7))
            .build();
        (jar.add(cookie), Redirect::to("/")).into_response()
    } else {
        record_failed_attempt(&state, &ip);
        warn!("Failed dashboard login from {ip}");
        Redirect::to("/?login=failed").into_response()
    }
}

async fn htmx_logout(State(state): State<AppState>, jar: CookieJar) -> Response {
    if let Some(token) = session_token(&jar)
        && let Ok(mut sessions) = state.sessions.lock() {
            sessions.remove(&token);
        }
    (jar.remove(Cookie::from(COOKIE_NAME)), Redirect::to("/")).into_response()
}

/// Bangun router lengkap.
pub fn build_router(state: AppState, web_dir: &str) -> Router {
    Router::new()
        .route("/", get(index_page))
        .route("/htmx/toggle", post(htmx_toggle))
        .route("/htmx/stats", get(htmx_stats))
        .route("/htmx/settings", post(htmx_settings))
        .route("/htmx/login", post(htmx_login))
        .route("/htmx/logout", post(htmx_logout))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/auth/check", get(auth_check))
        .route("/api/status", get(status))
        .route("/api/bot/toggle", post(toggle_bot))
        .route("/api/settings", get(settings_get).post(settings_post))
        .route("/api/events", get(events))
        .fallback_service(ServeDir::new(web_dir))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

/// Jalankan server dashboard.
pub async fn serve(state: AppState, web_dir: &str) -> std::io::Result<()> {
    let cfg = config::get();

    let bind_host = {
        let h = cfg.web_host.trim();
        // Domain atau URL publik (misalnya *.railway.app atau https://...)
        // tidak bisa di-bind sebagai local network interface IP di dalam container.
        if h.contains("://") || h.contains(".railway.app") || h.contains(".up.railway.app") {
            warn!("WEB_HOST '{h}' adalah URL/domain publik, bukan network interface IP lokal. Menggunakan fallback '0.0.0.0'.");
            "0.0.0.0"
        } else {
            h
        }
    };

    let addr = format!("{}:{}", bind_host, cfg.web_port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            if bind_host != "0.0.0.0" {
                warn!("Gagal bind ke {addr}: {e}. Mencoba fallback ke 0.0.0.0:{}...", cfg.web_port);
                let fallback_addr = format!("0.0.0.0:{}", cfg.web_port);
                tokio::net::TcpListener::bind(&fallback_addr).await?
            } else {
                return Err(e);
            }
        }
    };

    let actual_local_addr = listener.local_addr()?;
    info!("============================================================");
    info!("Dashboard listening on http://{actual_local_addr}");
    info!("============================================================");

    let app = build_router(state, web_dir);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use grammers_client::{Client, SenderPool};
    use grammers_session::storages::MemorySession;
    use tower::ServiceExt;

    use crate::core::Claimer;
    use crate::recognizer::MultiEngine;

    /// Build the router with a Telegram client that is **not connected**.
    /// `SenderPool::new` only creates channels; connections are made on demand.
    /// Every HTTP route can therefore be tested without credentials or an OTP.
    fn test_app() -> Router {
        let session = Arc::new(MemorySession::default());
        let SenderPool { handle, .. } = SenderPool::new(session, 1);
        let client = Client::new(handle);

        let state = AppState::new(Arc::new(LogBuffer::new()));
        let listener = Arc::new(WaifuListener::new(
            client,
            MultiEngine::new(),
            Claimer::new(),
            state.events_sender(),
        ));
        state.set_listener(listener);

        // Direktori `web/` ada di root proyek, sedangkan pengujian berjalan
        // directory (`rust/`).
        build_router(state, "../web")
    }

    /// Router without a listener, mirroring the state before Telegram connects.
    fn test_app_without_listener() -> Router {
        build_router(AppState::new(Arc::new(LogBuffer::new())), "../web")
    }

    fn request(method: Method, uri: &str) -> Request<Body> {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .expect("request valid");
        // The middleware needs ConnectInfo; axum::serve fills this in production.
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 45_678))));
        req
    }

    async fn body_json(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body terbaca");
        serde_json::from_slice(&bytes).expect("body adalah JSON valid")
    }

    #[tokio::test]
    async fn auth_check_mengembalikan_kontrak_frontend() {
        let response = test_app()
            .oneshot(request(Method::GET, "/api/auth/check"))
            .await
            .expect("handler berjalan");

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert!(json.get("authenticated").is_some());
        assert!(json.get("protected").is_some());
    }

    #[tokio::test]
    async fn status_memuat_semua_field_yang_dibaca_app_js() {
        let response = test_app()
            .oneshot(request(Method::GET, "/api/status"))
            .await
            .expect("handler berjalan");

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;

        // Top-level fields that web/app.js reads.
        for key in ["status", "is_active", "memory_mb", "user", "stats", "config"] {
            assert!(json.get(key).is_some(), "field hilang: {key}");
        }

        // stats.detected / claimed / failed feed the statistic cards.
        let stats = json.get("stats").expect("stats ada");
        for key in ["detected", "claimed", "failed"] {
            assert!(stats.get(key).is_some(), "stats.{key} hilang");
        }

        // config.* feeds the configuration summary panel.
        let cfg = json.get("config").expect("config ada");
        for key in [
            "CLAIM_COMMAND",
            "NAME_FORMAT",
            "IQDB_MIN_SIMILARITY",
            "TRACEMOE_MIN_SIMILARITY",
            "TRIGGER_KEYWORDS",
        ] {
            assert!(cfg.get(key).is_some(), "config.{key} hilang");
        }
    }

    #[tokio::test]
    async fn settings_mengembalikan_seluruh_key_yang_dipakai_frontend() {
        let response = test_app()
            .oneshot(request(Method::GET, "/api/settings"))
            .await
            .expect("handler berjalan");

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;

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
            "SUCCESS_KEYWORDS",
            "FAIL_KEYWORDS",
            "WEB_PORT",
            "WEB_HOST",
        ] {
            assert!(json.get(key).is_some(), "key pengaturan hilang: {key}");
        }
    }

    #[tokio::test]
    async fn security_headers_terpasang_di_setiap_respons() {
        let response = test_app()
            .oneshot(request(Method::GET, "/api/status"))
            .await
            .expect("handler berjalan");

        let headers = response.headers();
        assert_eq!(headers.get("X-Content-Type-Options").unwrap(), "nosniff");
        assert_eq!(headers.get("X-Frame-Options").unwrap(), "DENY");
        assert_eq!(headers.get("X-XSS-Protection").unwrap(), "1; mode=block");
        assert_eq!(headers.get("Cache-Control").unwrap(), "no-store");
        assert!(headers.get("Referrer-Policy").is_some());
    }

    #[tokio::test]
    async fn csrf_menolak_origin_asing() {
        let mut req = request(Method::POST, "/api/bot/toggle");
        req.headers_mut()
            .insert("Origin", "https://penyerang.example".parse().unwrap());

        let response = test_app().oneshot(req).await.expect("handler berjalan");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn csrf_mengizinkan_origin_lokal() {
        let mut req = request(Method::POST, "/api/bot/toggle");
        req.headers_mut()
            .insert("Host", "localhost:8080".parse().unwrap());
        req.headers_mut()
            .insert("Origin", "http://localhost:8080".parse().unwrap());

        let response = test_app().oneshot(req).await.expect("handler berjalan");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn toggle_membalik_status_aktif() {
        let app = test_app();

        let first = app
            .clone()
            .oneshot(request(Method::POST, "/api/bot/toggle"))
            .await
            .expect("handler berjalan");
        assert_eq!(first.status(), StatusCode::OK);
        let json = body_json(first).await;
        assert_eq!(json.get("success").unwrap(), &json!(true));
        let after_first = json.get("is_active").unwrap().as_bool().unwrap();

        let second = app
            .oneshot(request(Method::POST, "/api/bot/toggle"))
            .await
            .expect("handler berjalan");
        let json = body_json(second).await;
        let after_second = json.get("is_active").unwrap().as_bool().unwrap();

        assert_ne!(after_first, after_second, "toggle harus membalik status");
    }

    #[tokio::test]
    async fn file_statis_disajikan() {
        let response = test_app()
            .oneshot(request(Method::GET, "/style.css"))
            .await
            .expect("handler berjalan");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn sse_mengembalikan_event_stream() {
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            test_app().oneshot(request(Method::GET, "/api/events")),
        )
        .await
        .expect("SSE tidak boleh menggantung saat membuka koneksi")
        .expect("handler berjalan");

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get("content-type")
            .expect("content-type ada")
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            content_type.starts_with("text/event-stream"),
            "content-type tak terduga: {content_type}"
        );
    }

    #[tokio::test]
    async fn rate_limit_menolak_setelah_batas_terlampaui() {
        let app = test_app();

        for _ in 0..API_RATE_MAX_REQUESTS {
            let response = app
                .clone()
                .oneshot(request(Method::GET, "/api/status"))
                .await
                .expect("handler berjalan");
            assert_eq!(response.status(), StatusCode::OK);
        }

        let blocked = app
            .oneshot(request(Method::GET, "/api/status"))
            .await
            .expect("handler berjalan");
        assert_eq!(
            blocked.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "permintaan ke-{} harus diblokir",
            API_RATE_MAX_REQUESTS + 1
        );
    }

    #[tokio::test]
    async fn status_tanpa_listener_tetap_melayani() {
        // Scenario: the Telegram credentials are wrong. The dashboard must stay
        // up so the cause is visible in the live console.
        let response = test_app_without_listener()
            .oneshot(request(Method::GET, "/api/status"))
            .await
            .expect("handler berjalan");

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert_eq!(json.get("bot_ready").unwrap(), &json!(false));
        assert_eq!(json.get("is_active").unwrap(), &json!(false));
        assert_eq!(json.get("cache").unwrap().get("entries").unwrap(), &json!(0));
        assert_eq!(json.get("engines").unwrap(), &json!([]));
        assert_eq!(json.get("stats").unwrap().get("detected").unwrap(), &json!(0));
    }

    #[tokio::test]
    async fn toggle_tanpa_listener_mengembalikan_503() {
        let response = test_app_without_listener()
            .oneshot(request(Method::POST, "/api/bot/toggle"))
            .await
            .expect("handler berjalan");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let json = body_json(response).await;
        assert_eq!(json.get("success").unwrap(), &json!(false));
        assert_eq!(json.get("bot_ready").unwrap(), &json!(false));
    }

    #[tokio::test]
    async fn sse_tetap_terbuka_tanpa_listener() {
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            test_app_without_listener().oneshot(request(Method::GET, "/api/events")),
        )
        .await
        .expect("SSE tidak boleh menggantung")
        .expect("handler berjalan");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn status_melaporkan_engine_dan_cache_saat_siap() {
        let response = test_app()
            .oneshot(request(Method::GET, "/api/status"))
            .await
            .expect("handler berjalan");

        let json = body_json(response).await;
        assert_eq!(json.get("bot_ready").unwrap(), &json!(true));

        // IQDB and Trace.moe are always active; SauceNAO/Lens depend on config.
        let engines = json.get("engines").unwrap().as_array().unwrap();
        assert!(engines.iter().any(|e| e == "IQDB"));
        assert!(engines.iter().any(|e| e == "Trace.moe"));

        let cache = json.get("cache").unwrap();
        assert!(cache.get("hit_rate").is_some());
        assert!(cache.get("hits").is_some());
    }

    // --- Rute HTMX (Server-Driven UI) ---

    async fn body_text(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body terbaca");
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[tokio::test]
    async fn halaman_utama_dirender_dari_template_askama() {
        let response = test_app()
            .oneshot(request(Method::GET, "/"))
            .await
            .expect("handler berjalan");

        assert_eq!(response.status(), StatusCode::OK);
        let html = body_text(response).await;

        // Local HTMX bundle, not a CDN.
        assert!(html.contains("/htmx.min.js"), "local htmx is not referenced");
        assert!(html.contains("/style.css"));

        for tab in ["dashboard", "logs", "settings"] {
            assert!(
                html.contains(&format!("data-tab=\"{tab}\"")),
                "tab '{tab}' missing from page"
            );
        }

        // The tester tab was removed, and the SSE extension with it.
        assert!(
            !html.contains("data-tab=\"tester\""),
            "the tester tab should no longer exist"
        );
        assert!(
            !html.contains("htmx-ext-sse"),
            "the HTMX SSE extension is no longer needed"
        );

        // The live stream is consumed by app.js instead.
        assert!(html.contains("/app.js"), "app.js is not referenced");
    }

    #[tokio::test]
    async fn htmx_stats_mengembalikan_kartu_statistik() {
        let response = test_app()
            .oneshot(request(Method::GET, "/htmx/stats"))
            .await
            .expect("handler berjalan");

        assert_eq!(response.status(), StatusCode::OK);
        let html = body_text(response).await;
        assert!(html.contains("stat-card"), "kartu statistik tidak dirender");
        assert!(html.contains("stat-detected"));
    }

    #[tokio::test]
    async fn htmx_toggle_mengembalikan_partial_tombol() {
        let app = test_app();

        let first = app
            .clone()
            .oneshot(request(Method::POST, "/htmx/toggle"))
            .await
            .expect("handler berjalan");
        assert_eq!(first.status(), StatusCode::OK);
        let html = body_text(first).await;
        assert!(
            html.contains("bot-toggle-wrapper"),
            "partial toggle tidak dirender"
        );

        // The second request must also succeed, flipping the state back.
        let second = app
            .oneshot(request(Method::POST, "/htmx/toggle"))
            .await
            .expect("handler berjalan");
        assert_eq!(second.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn htmx_stats_tetap_melayani_tanpa_listener() {
        // Wrong Telegram credentials: the partial must still render, with zeros,
        // rather than failing.
        let response = test_app_without_listener()
            .oneshot(request(Method::GET, "/htmx/stats"))
            .await
            .expect("handler berjalan");

        assert_eq!(response.status(), StatusCode::OK);
        let html = body_text(response).await;
        assert!(html.contains("stat-card"));
    }

    /// Build an `IndexTemplate` with neutral values, so a test can vary one field
    /// without spelling out all 26 of them.
    fn sample_index_template() -> IndexTemplate {
        IndexTemplate {
            authenticated: false,
            is_protected: true,
            login_error: String::new(),
            is_active: false,
            bot_ready: false,
            user_name: "Telegram User".into(),
            user_tag: String::new(),
            memory_mb: 0.0,
            detected: 0,
            claimed: 0,
            failed: 0,
            success_rate: 100.0,
            claim_command: "/protecc".into(),
            name_format: "full".into(),
            iqdb_similarity: 60.0,
            tracemoe_similarity: 85.0,
            saucenao_similarity: 70.0,
            saucenao_key: String::new(),
            lens_enabled: true,
            triggers: "A waifu has appeared!".into(),
            target_chats: String::new(),
            min_delay: 0.0,
            max_delay: 0.0,
            verify_timeout: 5.0,
            send_timeout: 5.0,
            success_keywords: "now protected".into(),
            fail_keywords: "not quite right".into(),
        }
    }

    #[test]
    fn template_menampilkan_galat_login_saat_belum_terautentikasi() {
        use askama::Template as _;

        let mut tpl = sample_index_template();
        tpl.login_error = "Wrong password. Please try again.".to_string();

        let html = tpl.render().expect("template terender");

        assert!(html.contains("login-overlay"), "overlay login tidak muncul");
        assert!(
            html.contains("Wrong password"),
            "pesan galat tidak dirender di dalam overlay"
        );
    }

    #[test]
    fn template_menyembunyikan_overlay_login_saat_sudah_terautentikasi() {
        use askama::Template as _;

        let mut tpl = sample_index_template();
        tpl.authenticated = true;

        let html = tpl.render().expect("template terender");

        assert!(
            !html.contains("login-overlay"),
            "the login overlay must not render once signed in"
        );
        // The main application renders instead.
        assert!(html.contains("data-tab=\"dashboard\""));
        assert!(html.contains("id=\"log-terminal\""));
        assert!(html.contains("id=\"feed-list\""));
    }

    #[test]
    fn template_tanpa_password_tidak_menampilkan_overlay_login() {
        use askama::Template as _;

        let mut tpl = sample_index_template();
        tpl.is_protected = false;

        let html = tpl.render().expect("template terender");
        assert!(!html.contains("login-overlay"));
    }
}
