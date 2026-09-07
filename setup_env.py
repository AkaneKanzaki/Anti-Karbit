import os

def setup():
    print("\n==================================================")
    print("  SETUP: ANTI-KARBIT (IQDB + TRACE.MOE SEARCH)   ")
    print("==================================================\n")
    print("Bot ini menggunakan mesin pencari gambar internet (IQDB & Trace.moe).")
    print("100% GRATIS & TANPA API KEY APAPUN!\n")

    env_path = ".env"
    if os.path.exists(env_path):
        overwrite = input("File .env sudah ada. Ingin memperbarui? (y/n): ").strip().lower()
        if overwrite != "y":
            print("Setup dibatalkan.")
            return

    print("1. Kredensial Telegram (dari https://my.telegram.org):")
    api_id = input("   Masukkan TELEGRAM_API_ID (angka): ").strip()
    api_hash = input("   Masukkan TELEGRAM_API_HASH: ").strip()

    print("\n2. Perintah Klaim Game:")
    cmd = input("   Perintah klaim (default: /protecc): ").strip() or "/protecc"

    print("\n3. Format Nama Awal:")
    print("   - 'full'  : Nama lengkap (contoh: /protecc Houraisan Kaguya)")
    print("   - 'first' : Hanya nama depan (contoh: /protecc Kaguya)")
    print("   - 'both'  : Kirim nama depan dulu lalu nama lengkap")
    print("   * Catatan: Bot otomatis mencoba variasi nama lainnya jika ditolak game bot.")
    name_fmt = input("   Pilihan format nama (full/first/both, default: full): ").strip().lower() or "full"

    content = f"""# Konfigurasi AntiKarbit Bot (Pencarian Internet: IQDB + Trace.moe - 0 API Key)
TELEGRAM_API_ID={api_id}
TELEGRAM_API_HASH={api_hash}
TELEGRAM_SESSION_NAME=waifu_claimer_session

# Reverse Search Settings
IQDB_MIN_SIMILARITY=40.0
TRACEMOE_MIN_SIMILARITY=0.80

# Claim Settings
CLAIM_COMMAND={cmd}
NAME_FORMAT={name_fmt}
TRIGGER_KEYWORDS=A waifu has appeared!,A husbando has appeared!,/protecc name,Add her to your harem,Add him to your harem

# Target Chats (kosongkan untuk semua grup)
TARGET_CHAT_IDS=
MIN_DELAY_SECONDS=0.5
MAX_DELAY_SECONDS=1.5

# Verifikasi Klaim & Auto-Retry
VERIFY_TIMEOUT_SECONDS=5.0
SUCCESS_KEYWORDS=now protected,added to your harem,added to your collection,is now yours,congratulations
FAIL_KEYWORDS=not quite right,wrong name,already claimed,already protecc,rip,try again
"""

    with open(env_path, "w", encoding="utf-8") as f:
        f.write(content)

    print("\n[SUKSES] File .env berhasil dibuat!")
    print("Sekarang Anda dapat menjalankan aplikasi dengan:")
    print("  Klik ganda 'run_app.bat' atau jalankan '.\\.venv\\Scripts\\python.exe app.py'\n")

if __name__ == "__main__":
    setup()
