//! AntiKarbit — Telegram waifu claimer bot with a web dashboard.
//!
//! Four modes:
//!   `antikarbit web`            — web dashboard (default)
//!   `antikarbit cli`            — headless, for a VPS or background service
//!   `antikarbit export-session` — print the session as base64 for an env var
//!   `antikarbit test-image`     — test recognition on a single image

mod cache;
mod config;
mod core;
mod http;
mod recognizer;
mod session;
mod web;

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use grammers_client::client::{UpdateStream, UpdatesConfiguration};
use grammers_client::{Client, SenderPool, SignInError};
use grammers_session::storages::SqliteSession;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::core::{Claimer, WaifuListener};
use crate::recognizer::MultiEngine;
use crate::web::logs::LogBuffer;

fn init_tracing(logs: Arc<LogBuffer>) {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(fmt::layer().with_target(true))
        .with(web::logs::LogLayer::new(logs))
        .with(filter)
        .init();
}

/// Read a single line from stdin.
///
/// Runs under `spawn_blocking` so the blocking read does not tie up a runtime
/// worker thread. That matters because the dashboard is already serving
/// requests while sign-in is still waiting for input.
async fn prompt(message: &str) -> String {
    let message = message.to_string();
    tokio::task::spawn_blocking(move || {
        print!("{message}");
        let _ = std::io::stdout().flush();

        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        line.trim().to_string()
    })
    .await
    .unwrap_or_default()
}

fn session_path() -> String {
    format!("{}.session", config::get().telegram_session_name)
}

struct TelegramSession {
    client: Client,
    stream: UpdateStream,
    pool_task: JoinHandle<()>,
    name: String,
    username: Option<String>,
    id: i64,
}

/// Build the Telegram client, signing in when the session is not authorised.
///
/// The `SenderPool` runner is started **before** any API call. Without that,
/// `is_authorized()` hangs waiting on a connection that nothing is driving.
async fn connect_telegram() -> Result<TelegramSession, Box<dyn std::error::Error + Send + Sync>> {
    let cfg = config::get();
    let path = session_path();

    // Restore the session from the env var when present (Railway, no volume).
    match session::materialize_from_env(&path) {
        Ok(true) => info!("Session restored from TELEGRAM_STRING_SESSION."),
        Ok(false) => {}
        Err(e) => {
            error!("{e}");
            return Err(format!("Gagal memulihkan sesi dari TELEGRAM_STRING_SESSION: {e}").into());
        }
    }

    // Pastikan schema session SQLite valid (user_version = 1 & ipv6 valid) agar grammers tidak gagal
    if let Err(e) = session::sanitize_session_db(&path).await {
        warn!("Peringatan sanitasi database sesi: {e}");
    }

    let session = Arc::new(SqliteSession::open(&path).await?);

    let SenderPool {
        runner,
        handle,
        updates,
    } = SenderPool::new(Arc::clone(&session), cfg.telegram_api_id);

    let client = Client::new(handle);
    let pool_task = tokio::spawn(runner.run());

    if !client.is_authorized().await? {
        info!("Session is not authorised. Sign-in required.");
        let phone = prompt("Phone number (international format, e.g. +62812...): ").await;
        if phone.is_empty() {
            return Err(
                "Nomor telepon kosong atau stdin tidak interaktif (headless/cloud). Jika menjalankan di cloud seperti Railway, login terlebih dahulu di lokal dengan 'antikarbit export-session' lalu isi variabel TELEGRAM_STRING_SESSION, atau gunakan Railway Volume."
                    .into(),
            );
        }
        let token = client
            .request_login_code(&phone, &cfg.telegram_api_hash)
            .await?;
        let code = prompt("Login code (check your Telegram app): ").await;
        if code.is_empty() {
            return Err("Login code kosong. Pastikan terminal interaktif saat login.".into());
        }

        match client.sign_in(&token, &code).await {
            Ok(_) => info!("Signed in."),
            Err(SignInError::PasswordRequired(password_token)) => {
                // An account with two-step verification asks for a separate password
                // after the login code is accepted.
                let hint = password_token.hint().unwrap_or("no hint provided");
                info!("Two-step verification is enabled (hint: {hint}).");
                let password = prompt("Telegram 2FA password: ").await;
                client
                    .check_password(password_token, password.trim())
                    .await?;
                info!("Signed in with two-step verification.");
            }
            Err(e) => return Err(format!("Sign-in failed: {e}").into()),
        }
    }

    let me = client.get_me().await?;
    let name = me.first_name().unwrap_or("Telegram User").to_string();
    let username = me.username().map(|u| u.to_string());
    let id = me.id().bot_api_dialog_id().unwrap_or(0);

    info!(
        "Connected as {} (@{})",
        name,
        username.as_deref().unwrap_or("-")
    );

    // The stream is created here so the private `UpdatesLike` type never has
    // to appear in a signature.
    let stream = client
        .stream_updates(updates, UpdatesConfiguration::default())
        .await?;

    Ok(TelegramSession {
        client,
        stream,
        pool_task,
        name,
        username,
        id,
    })
}

fn build_components(client: Client, events_tx: broadcast::Sender<Value>) -> WaifuListener {
    WaifuListener::new(client, MultiEngine::new(), Claimer::new(), events_tx)
}

/// Locate the `web/` directory holding the dashboard assets.
fn find_web_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("WEB_DIR") {
        return PathBuf::from(dir);
    }
    for candidate in ["web", "../web", "/app/web"] {
        let path = PathBuf::from(candidate);
        if path.join("index.html").is_file() {
            return path;
        }
    }
    PathBuf::from("web")
}

fn print_banner(cfg: &config::Config) {
    println!(
        "\n============================================================\n\
         \x20  ANTI-KARBIT: WAIFU CLAIMER BOT (RUST)\n\
         ============================================================\n\
         * Engine Vision : IQDB + SauceNAO + Ascii2d + Trace.moe + Google Lens (paralel)\n\
         * Claim Command : {}\n\
         * Name Format   : {}\n\
         * Trigger Words : {}\n\
         * Chat Target   : {}\n\
         * Web Dashboard : http://localhost:{}\n\
         ============================================================\n",
        cfg.claim_command,
        cfg.name_format,
        cfg.trigger_keywords.join(", "),
        if cfg.target_chat_ids.is_empty() {
            "All groups".to_string()
        } else {
            cfg.target_chat_ids
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        },
        cfg.web_port
    );
}

async fn run_web(logs: Arc<LogBuffer>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cfg = config::get();
    print_banner(&cfg);

    let state = web::AppState::new(logs);
    let web_dir = find_web_dir();

    // 1. Start the dashboard FIRST.
    //
    // If the Telegram credentials are wrong or missing, the dashboard stays up
    // and the error streams into the live console. Without this the container
    // would die leaving nothing visible in the browser.
    let serve_state = state.clone();
    let web_task = tokio::spawn(async move {
        web::serve(serve_state, &web_dir.to_string_lossy()).await
    });

    // 2. Then connect to Telegram.
    let errors = cfg.validate();
    if !errors.is_empty() {
        for err in &errors {
            error!("Configuration: {err}");
        }
        error!("The dashboard stays up so this can be inspected. Fix .env and restart.");
        report_web_exit(web_task.await);
        return Ok(());
    }

    match connect_telegram().await {
        Ok(session) => {
            let TelegramSession {
                client,
                stream,
                pool_task,
                name,
                username,
                id,
            } = session;

            let listener = Arc::new(build_components(client.clone(), state.events_sender()));
            state.set_listener(listener.clone());
            state.set_me(&name, username.as_deref(), id);

            let listener_task = tokio::spawn({
                let listener = listener.clone();
                async move { listener.run(stream).await }
            });

            report_web_exit(web_task.await);

            listener_task.abort();
            drop(client);
            let _ = pool_task.await;
        }
        Err(e) => {
            error!("Could not connect to Telegram: {e}");
            error!("The dashboard stays up so this error can be inspected in the browser.");
            report_web_exit(web_task.await);
        }
    }

    Ok(())
}

/// Report a dashboard server that stopped early. `web::serve` is designed to
/// run forever, so returning at all means something went wrong.
fn report_web_exit(result: Result<std::io::Result<()>, tokio::task::JoinError>) {
    match result {
        Ok(Ok(())) => error!("Server dashboard berhenti tanpa error."),
        Ok(Err(e)) => error!("Server dashboard gagal: {e}"),
        Err(e) => error!("Task server dashboard panik: {e}"),
    }
}

async fn run_cli(logs: Arc<LogBuffer>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = logs;
    let cfg = config::get();

    let errors = cfg.validate();
    if !errors.is_empty() {
        for err in &errors {
            error!("Configuration: {err}");
        }
        return Ok(());
    }

    print_banner(&cfg);

    let session = connect_telegram().await?;
    let TelegramSession {
        client,
        stream,
        pool_task,
        ..
    } = session;

    let listener = {
        // CLI mode serves no dashboard, so nothing consumes the event bus.
        let (events_tx, _) = broadcast::channel(64);
        build_components(client.clone(), events_tx)
    };
    listener.run(stream).await;

    drop(client);
    let _ = pool_task.await;
    Ok(())
}

/// Test recognition on a single image from the command line.
///
/// Runs every engine in parallel and reports each one's result, which is useful
/// for working out which engine is misbehaving.
async fn run_test_image(path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cfg = config::get();

    let bytes = std::fs::read(path)?;
    println!("\nImage: {path} ({} bytes)", bytes.len());

    let engine = MultiEngine::new();
    println!("Active engines: {}", engine.active_labels().join(" -> "));
    println!("{}", "-".repeat(58));

    let started = std::time::Instant::now();
    let results = engine.identify_all(&bytes).await;
    let elapsed = started.elapsed();

    if results.is_empty() {
        println!("\n[NO MATCH] No engine could identify this image.");
        return Ok(());
    }

    for (kind, character) in &results {
        println!("\n=== {} ===", kind.label());
        println!("Full name  : {}", character.full_name);
        println!("First name : {}", character.first_name);
        println!(
            "Last name  : {}",
            character.last_name.as_deref().unwrap_or("-")
        );
        println!(
            "Series     : {}",
            character.series.as_deref().unwrap_or("Unknown")
        );
        println!("Confidence : {:.1}%", character.confidence * 100.0);
        if !character.alternate_names.is_empty() {
            println!("Also known as: {}", character.alternate_names.join(", "));
        }

        let names = character.claim_names(&cfg.name_format);
        let commands: Vec<String> = names
            .iter()
            .map(|n| format!("{} {}", cfg.claim_command, n))
            .collect();
        println!("Claim command : {}", commands.join("  |  "));
    }

    println!("\n{}", "-".repeat(58));
    println!(
        "{} engine(s) matched, {:.2} s total (parallel)",
        results.len(),
        elapsed.as_secs_f64()
    );

    Ok(())
}

/// Print the session as base64, ready to paste into `TELEGRAM_STRING_SESSION`.
async fn run_export_session() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cfg = config::get();

    let errors = cfg.validate();
    if !errors.is_empty() {
        for err in &errors {
            error!("Configuration: {err}");
        }
        return Ok(());
    }

    let path = session_path();
    info!("Menyiapkan sesi di: {path}");

    let session = connect_telegram().await?;

    // Close the connection so the session is flushed to disk before reading.
    session.pool_task.abort();
    drop(session.client);
    drop(session.stream);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let encoded = match session::export_to_string(&path) {
        Ok(e) => e,
        Err(e) => {
            error!("{e}");
            return Ok(());
        }
    };

    let out_file = "session_string.txt";
    if let Err(e) = std::fs::write(out_file, &encoded) {
        warn!("Gagal menulis ke berkas {out_file}: {e}");
    } else {
        info!("String sesi juga disimpan ke berkas '{out_file}' (bebas risiko line-wrapping).");
    }

    println!("\n================ TELEGRAM_STRING_SESSION ================");
    println!("Salin string di bawah ini ke variabel TELEGRAM_STRING_SESSION di Railway");
    println!("(atau buka dan salin langsung dari berkas 'session_string.txt'):\n");
    println!("{encoded}");
    println!("\nPanjang: {} karakter", encoded.len());
    println!("=========================================================\n");

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    config::init();

    let logs = Arc::new(LogBuffer::new());
    init_tracing(logs.clone());

    let mode = std::env::args().nth(1).unwrap_or_else(|| "web".to_string());

    match mode.as_str() {
        "web" => run_web(logs).await,
        "cli" => run_cli(logs).await,
        "export-session" => run_export_session().await,
        "test-image" => match std::env::args().nth(2) {
            Some(path) => run_test_image(&path).await,
            None => {
                eprintln!("Usage: antikarbit test-image <path-to-image>");
                eprintln!("Example: antikarbit test-image test_waifu.png");
                Ok(())
            }
        },
        other => {
            eprintln!("Unknown mode: {other}");
            eprintln!("Available modes: web, cli, export-session, test-image");
            Ok(())
        }
    }
}
