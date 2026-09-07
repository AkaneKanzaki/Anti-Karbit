# 🌸 AntiKarbit - Telegram Waifu Claimer Bot & Desktop Dashboard

Bot otomasi cerdas untuk Telegram yang memantau kemunculan karakter waifu/husbando di grup game Telegram (seperti Protecc/Harem game), mengidentifikasi nama karakter anime menggunakan **Mesin Pencari Gambar Internet (IQDB.org & Trace.moe)**, mengirim perintah klaim (misal: `/protecc <nama>`), memverifikasi balasan game bot, serta otomatis melakukan retry dengan variasi nama jika ditolak.

> **100% GRATIS & TANPA API KEY AI APAPUN!**
> - Menggunakan Reverse Image Search (IQDB.org & Trace.moe) + AniList enrichment.
> - Bebas dari error server overload (503) dan kuota rate limit.
> - Hemat memori (< 65 MB RAM) & hemat disk (0 MB model lokal).

---

## 🚀 Cara Menjalankan

### 1. Mode Desktop App / Web Dashboard (Rekomendasi)
Cukup klik ganda:
```cmd
run_app.bat
```
Atau jalankan via terminal:
```powershell
.\.venv\Scripts\python.exe app.py
```
Aplikasi akan otomatis membuka antarmuka visual di **http://localhost:8080** dengan fitur:
- 📊 **Dashboard & Statistik Real-time**: Pantau waifu terdeteksi, sukses diklaim, dan akurasi.
- ⏸️ **Kontrol Bot**: Tombol Pause/Resume listener instan dari UI.
- 🖼️ **Interactive Waifu Tester**: Drag & drop gambar untuk menguji hasil deteksi tanpa perlu menunggu spawn grup.
- 💻 **Live Console Stream**: Pantau log sistem langsung dari web browser.
- ⚙️ **Pengaturan Visual**: Ubah trigger, perintah klaim, batas kemiripan, dan delay langsung dari form.

### 2. Mode Headless CLI (Tanpa Web)
Cocok untuk dijalankan di server Linux / VPS latar belakang:
```powershell
.\.venv\Scripts\python.exe main.py
```

### 3. Menguji Gambar Secara Manual
```powershell
.\.venv\Scripts\python.exe test_recognizer.py <path_ke_gambar.jpg>
```

---

## ⚙️ Konfigurasi `.env`

File `.env` hanya membutuhkan kredensial akun Telegram Anda (dari [my.telegram.org](https://my.telegram.org)):

```env
TELEGRAM_API_ID=12345678
TELEGRAM_API_HASH=abcdef0123456789abcdef0123456789
TELEGRAM_SESSION_NAME=waifu_claimer_session

# Reverse Search Settings
IQDB_MIN_SIMILARITY=40.0
TRACEMOE_MIN_SIMILARITY=0.80

# Claim Settings
CLAIM_COMMAND=/protecc
NAME_FORMAT=full
TRIGGER_KEYWORDS=A waifu has appeared!,A husbando has appeared!,/protecc name,Add her to your harem,Add him to your harem

# Target Chats (kosongkan untuk semua grup)
TARGET_CHAT_IDS=
MIN_DELAY_SECONDS=0.5
MAX_DELAY_SECONDS=1.5
VERIFY_TIMEOUT_SECONDS=5.0
```

---

## ☁️ Deployment ke Railway.app (24/7 di Cloud)

Aplikasi ini sudah dilengkapi `Dockerfile`, `Procfile`, dan dukungan `TELEGRAM_STRING_SESSION` sehingga dapat berjalan 24/7 di Railway.app tanpa memerlukan persistent volume:

1. **Dapatkan String Session**:
   Jalankan script helper di lokal:
   ```powershell
   .\.venv\Scripts\python.exe export_session.py
   ```
2. **Push Repository ke GitHub** (file `.env` & `.session` otomatis diabaikan oleh `.gitignore`).
3. **Deploy di Railway**:
   - Buka [Railway.app](https://railway.app), klik **New Project** → **Deploy from GitHub repo**.
   - Masuk ke tab **Variables**, tambahkan:
     - `TELEGRAM_API_ID`
     - `TELEGRAM_API_HASH`
     - `TELEGRAM_STRING_SESSION` (hasil dari langkah 1)
     - `DASHBOARD_PASSWORD` (password untuk mengunci dashboard web dari publik)
     - `CLAIM_COMMAND` (contoh: `/protecc`)
     - `IQDB_MIN_SIMILARITY` (contoh: `40.0`)
   - Masuk ke tab **Settings** → **Networking** → klik **Generate Domain** untuk mendapatkan URL publik Web Dashboard Anda.
