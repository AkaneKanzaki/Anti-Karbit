//! Shared HTTP client.
//!
//! This is one of the main sources of speed-up: a single `Client` is shared by
//! every engine, with connection pooling and keep-alive, so the TLS connection
//! to IQDB/SauceNAO/Trace.moe/Lens is reused between requests instead of being
//! negotiated from scratch each time.

use std::sync::LazyLock;
use std::time::Duration;

use reqwest::Client;

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Shared client used by every search engine.
pub static CLIENT: LazyLock<Client> = LazyLock::new(|| build(false));

/// Separate client for Google Lens, which needs a cookie jar.
pub static COOKIE_CLIENT: LazyLock<Client> = LazyLock::new(|| build(true));

fn build(cookie_store: bool) -> Client {
    let mut builder = Client::builder()
        .user_agent(USER_AGENT)
        // Keep connections alive so TLS is not renegotiated on every lookup.
        .pool_max_idle_per_host(8)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .http2_adaptive_window(true);

    if cookie_store {
        builder = builder.cookie_store(true);
    }

    builder.build().expect("gagal membangun HTTP client")
}
