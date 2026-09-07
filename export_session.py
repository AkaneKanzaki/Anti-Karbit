"""
Script helper untuk mengekspor session Telegram lokal (waifu_claimer_session.session)
menjadi TELEGRAM_STRING_SESSION.
Sangat berguna untuk deploy ke Railway.app / cloud tanpa perlu upload file .session!
"""
import sys
import asyncio
from telethon import TelegramClient
from telethon.sessions import StringSession
from config import Config

async def export():
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")

    print("=" * 60)
    print("   EKSPOR TELEGRAM STRING SESSION (UNTUK CLOUD / RAILWAY)")
    print("=" * 60)

    # Buka session file yang sudah ada
    client = TelegramClient(
        Config.TELEGRAM_SESSION_NAME,
        Config.TELEGRAM_API_ID,
        Config.TELEGRAM_API_HASH,
    )
    await client.connect()

    if not await client.is_user_authorized():
        print("\n[PERINGATAN] Sesi lokal belum login. Harap login terlebih dahulu via main.py / app.py!")
        await client.disconnect()
        return

    # Konversi ke StringSession
    string_session = StringSession.save(client.session)
    await client.disconnect()

    print("\n✅ BERHASIL MENGHASILKAN STRING SESSION!\n")
    print("Salin nilai di bawah ini dan tempel di Environment Variables Railway:")
    print("-" * 60)
    print(f"TELEGRAM_STRING_SESSION={string_session}")
    print("-" * 60)
    print("\nDengan variabel ini di Railway, bot Anda langsung login otomatis 24/7 tanpa perlu upload file .session!\n")

if __name__ == "__main__":
    asyncio.run(export())
