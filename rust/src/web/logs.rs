//! Log capture for the dashboard's live console.
//!
//! Keeps a ring buffer of recent lines and broadcasts new ones to every
//! connected SSE client.
//!
//! Lines are formatted as `[HH:MM:SS] [module] message`, and that shape is
//! **locked in** by `web/app.js`, which renders the console.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::sync::broadcast;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

const MAX_LINES: usize = 200;

pub struct LogBuffer {
    buffer: Mutex<VecDeque<Value>>,
    tx: broadcast::Sender<Value>,
}

impl LogBuffer {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            buffer: Mutex::new(VecDeque::with_capacity(MAX_LINES)),
            tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.tx.subscribe()
    }

    /// Recent log history, sent to a newly connected SSE client.
    pub fn history(&self) -> Vec<Value> {
        self.buffer
            .lock()
            .map(|b| b.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn push(&self, entry: Value) {
        if let Ok(mut buffer) = self.buffer.lock() {
            if buffer.len() >= MAX_LINES {
                buffer.pop_front();
            }
            buffer.push_back(entry.clone());
        }
        let _ = self.tx.send(entry);
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// A `tracing` layer that forwards events into [`LogBuffer`].
pub struct LogLayer {
    buffer: Arc<LogBuffer>,
}

impl LogLayer {
    pub fn new(buffer: Arc<LogBuffer>) -> Self {
        Self { buffer }
    }
}

/// Extract just the `message` field from an event.
#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
}

impl<S> Layer<S> for LogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = *metadata.level();

        // Skip DEBUG and below: the live console is for things the operator
        // actually needs to see.
        if level < Level::INFO {
            return;
        }

        let target = metadata.target();

        // Quiet the networking libraries: per-request access logs would flood
        // the console and drown out bot activity.
        if (target.starts_with("grammers")
            || target.starts_with("hyper")
            || target.starts_with("tower_http")
            || target.starts_with("reqwest")
            || target.starts_with("h2"))
            && level < Level::WARN
        {
            return;
        }

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let msg = visitor.message;
        if msg.is_empty() {
            return;
        }

        // Shortened module name: `antikarbit.core.listener` -> `core.listener`.
        let module = target
            .strip_prefix("antikarbit.")
            .unwrap_or(target)
            .replace('.', "::");

        let time_str = utc_hms();

        let mut level_str = match level {
            Level::WARN => "warning",
            Level::ERROR => "error",
            _ => "info",
        };

        // Highlight success so a winning claim is immediately visible.
        if ["✅", "🎉", "Sukses", "BERHASIL", "now protected", "added to your"]
            .iter()
            .any(|needle| msg.contains(needle))
        {
            level_str = "success";
        }

        let clean = format!("[{time_str}] [{module}] {msg}");
        self.buffer.push(json!({ "message": clean, "level": level_str }));
    }
}

/// Hours:minutes:seconds in UTC, with no extra dependency.
fn utc_hms() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let day = secs % 86_400;
    format!("{:02}:{:02}:{:02}", day / 3600, (day % 3600) / 60, day % 60)
}
