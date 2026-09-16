# ============================================================
# AntiKarbit (Rust) — build multi-stage untuk Railway
# Target: 1 GB RAM / 2 vCPU / 1 GB disk
# ============================================================

# ---------- Stage 1: builder ----------
FROM rust:1-bookworm AS builder

# reqwest dikonfigurasi memakai native-tls, sehingga butuh OpenSSL saat menautkan.
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Salin manifest lebih dulu lalu bangun dependensi saja. Layer ini akan
# ter-cache dan tidak diulang setiap kali kode berubah.
#
# Tahap ini tidak butuh templates/ karena main.rs tiruan tidak memakai Askama.
COPY rust/Cargo.toml rust/Cargo.lock ./
RUN mkdir -p src \
    && printf 'fn main() {}\n' > src/main.rs \
    && cargo build --release \
    && rm -rf src

# Sekarang salin kode sebenarnya dan bangun ulang crate kita saja.
#
# PENTING: templates/ wajib ada di sini. Askama membaca berkas template saat
# kompilasi (derive macro), jadi tanpa folder ini build gagal dengan
# "template not found" — bukan error saat runtime.
COPY rust/templates ./templates
COPY rust/src ./src
RUN touch src/main.rs && cargo build --release

# ---------- Stage 2: runtime ----------
FROM debian:bookworm-slim

# libssl3 dibutuhkan native-tls saat runtime; ca-certificates untuk HTTPS.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/antikarbit /usr/local/bin/antikarbit
COPY web ./web

ENV WEB_DIR=/app/web \
    WEB_HOST=0.0.0.0 \
    PORT=8080 \
    RUST_LOG=info

EXPOSE 8080

CMD ["antikarbit", "web"]
