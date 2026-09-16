# AntiKarbit

A Telegram automation bot that claims waifu/husbando characters in game groups
(the Protecc / Harem style games), with a web dashboard.

The bot watches for characters appearing in a group, identifies the character
name through **reverse image search**, sends the claim command
(`/protecc <name>`), verifies the game bot's reply, and retries with alternate
names when a claim is rejected.

> **Free, and no AI API keys.** Uses IQDB, SauceNAO, Trace.moe and Google Lens,
> enriched with data from AniList.

---

## Why Rust

The project started as Python. The Rust implementation in `rust/` replaces it
entirely.

| | Python | Rust |
|---|---|---|
| Runtime size | `.venv` ~63 MB | binary ~11 MB |
| Reverse search | 4 engines **sequentially** = 7.40 s | 4 engines **in parallel** = 2.78 s |
| Claim verification | polls every 0.8 s, **blocks** the listener | event driven, **non-blocking** |
| Repeated character | always ~2.8 s | hash cache, effectively instant |
| Delay before claiming | 0.5–1.5 s (artificial) | 0 (configurable) |

The 2.78 s figure is measured, not estimated — on `test_waifu.png` with all four
engines enabled.

**Why non-blocking verification matters.** Previously a single claim waiting for
a reply locked the handler for up to 5 seconds. Any character spawning inside
that window was **dropped entirely** — not delayed, lost. Each message is now
handled in its own task, and replies fill a wait slot through a channel instead
of being polled for.

---

## How it works

```
Message arrives in a group
      |
      +-- matches a trigger keyword? ----- no --> ignore
      |
      +-- contains an image? ------------ no --> ignore
      |
      +-- image hash in cache? --------- yes --> claim immediately
      |
      +-- fan out to every active engine
            IQDB ---+
         SauceNAO ---+  results stream back as
         Trace.moe ---+  each engine finishes
        Google Lens ---+
                      |
                      +-- claim using the FASTEST result;
                          other engines stay as backup
```

Each engine reads its similarity threshold from configuration **at call time**,
so changing it in the dashboard takes effect without a restart.

---

## Running it

Requires a Rust toolchain. On Windows without Visual Studio, use
`stable-x86_64-pc-windows-gnu`.

```powershell
cd rust
cargo build --release
```

### Web dashboard (default)

```powershell
.\target\release\antikarbit.exe web
```

Open <http://localhost:8080>. The dashboard covers bot status and statistics,
a pause/resume control, a live console, and a settings form.

> The dashboard starts **before** Telegram is connected. If the credentials are
> wrong, it stays up and the error appears in the live console — so the problem
> can be diagnosed from the browser instead of a dead container.

### Headless (VPS)

```powershell
.\target\release\antikarbit.exe cli
```

### Testing a single image

```powershell
.\target\release\antikarbit.exe test-image test_waifu.png
```

Runs every engine in parallel and reports each one's result, which is useful for
working out which engine is failing.

### Exporting the Telegram session

```powershell
.\target\release\antikarbit.exe export-session
```

See the deployment section below.

---

## Configuration

Copy `.env.example` to `.env`. Every key is documented in that file.

The one worth calling out:

```env
# Artificial delay before the claim is sent. 0 disables it.
# This value directly reduces your chance of winning a claim.
MIN_DELAY_SECONDS=0
MAX_DELAY_SECONDS=0
```

---

## Deploying to Railway

The `Dockerfile` at the repository root is the Rust one (multi-stage:
`rust:bookworm` -> `debian:bookworm-slim`). `railway.json` needs no changes.

### The Telegram session must be handled

Railway without a persistent volume **deletes the `*.session` file on every
deploy**, so the bot would ask for an OTP again each time. Pick one:

**Option A — environment variable (no extra cost)**

1. Sign in once locally: `antikarbit export-session`
2. Copy the base64 output into `TELEGRAM_STRING_SESSION` on Railway
3. On startup the bot restores the session file from that variable

An existing local session is never overwritten, so this is safe to run
repeatedly.

**Option B — Railway Volume**

Attach a volume, then point `TELEGRAM_SESSION_NAME` at a path inside it, for
example `/data/waifu_claimer_session`.

### Other variables to set

`TELEGRAM_API_ID`, `TELEGRAM_API_HASH`, `DASHBOARD_PASSWORD`, and optionally
`SAUCENAO_API_KEY`.

---

## Development

```powershell
cd rust
cargo test          # 54 unit and HTTP integration tests
cargo clippy        # lints
```

### Layout

```
rust/src/
  main.rs              four modes: web, cli, export-session, test-image
  config.rs            reads and writes .env
  http.rs              shared HTTP client (connection pooling)
  cache.rs             image hash -> character cache
  session.rs           Telegram session portability
  core/
    claimer.rs         sending claims, event-driven verification
    listener.rs        group monitoring, parallel fan-out
  recognizer/
    mod.rs             engine fan-out
    base.rs            CharacterInfo, name splitting
    iqdb.rs            IQDB plus booru tag classification
    saucenao.rs        SauceNAO
    tracemoe.rs        Trace.moe
    lens.rs            Google Lens (must pass AniList verification)
    anilist.rs         AniList GraphQL lookups
  web/
    mod.rs             axum routes, auth, rate limiting, SSE
    logs.rs            log capture for the live console
rust/templates/        Askama templates, compiled at build time
  index.html           dashboard shell (three tabs plus a login overlay)
  partials/            fragments swapped in by HTMX
web/                   static assets: style.css, app.js, htmx.min.js
```

### Interface architecture

The dashboard is server-driven. Pages are rendered by Rust through Askama
templates that are **checked at compile time** — a mistyped variable name fails
`cargo build` instead of silently breaking in the browser.

HTMX handles the request/response parts of the UI:

| Action | Route | Swapped into |
|---|---|---|
| Pause / resume bot | `POST /htmx/toggle` | `#bot-toggle-wrapper` |
| Refresh statistics | `GET /htmx/stats` | `#stat-cards-grid` |
| Save settings | `POST /htmx/settings` | `#settings-toast-area` |
| Sign in / out | `POST /htmx/login`, `/htmx/logout` | redirect |

The live console and claim feed use a single `EventSource` against
`/api/events`, consumed in `web/app.js`. Claim events carry JSON that has to be
formatted into markup, which plain HTMX attribute swapping cannot express — so
the SSE extension is not used and is not shipped.

`htmx.min.js` is served from `web/` — **no CDN, no Node.js, no bundler.**

### Notes

- The `/api/...` endpoints are kept for compatibility and testing. Their JSON
  contract (`memory_mb`, `is_active`, `stats.*`, `config.*`, and the `log` /
  `claim_event` SSE events) is locked down by integration tests in
  `rust/src/web/mod.rs`.
- The dashboard is configured in `rust/templates/`. Adding or changing a field
  means updating the matching template struct in `src/web/mod.rs`; Askama checks
  both at compile time.
- Google Lens is the most fragile engine because Google's crawl results are
  unstructured. Every candidate it produces **must** pass AniList verification
  before it may be used in a claim.
- Timestamps in the live console are **UTC**.
