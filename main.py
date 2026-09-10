import sys
import asyncio
import logging

# Pastikan output konsol Windows mendukung karakter Unicode & emoji
if hasattr(sys.stdout, "reconfigure"):
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

from telethon import TelegramClient  # type: ignore
from config import Config
from recognizer import get_recognizer
from core import Claimer, WaifuListener

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
    handlers=[
        logging.StreamHandler(sys.stdout),
    ],
)
logger = logging.getLogger("antikarbit")


def print_banner():
    banner = f"""
============================================================
   ANTI-KARBIT: WAIFU CLAIMER BOT (CLI HEADLESS MODE)   
============================================================
* Engine Vision : IQDB → SauceNAO → Trace.moe → Google Lens (Fallback Bertahap)
* Claim Command : {Config.CLAIM_COMMAND}
* Name Format   : {Config.NAME_FORMAT}
* Trigger Words : {", ".join(Config.TRIGGER_KEYWORDS)}
* Chat Target   : {Config.TARGET_CHAT_IDS if Config.TARGET_CHAT_IDS else "Semua Grup"}
* Web Dashboard : Jalankan 'run_app.bat' untuk antarmuka visual
============================================================
"""
    print(banner)


async def main():
    print_banner()

    # Validasi konfigurasi Telegram
    errors = Config.validate()
    if errors:
        logger.error("Terjadi kesalahan pada konfigurasi .env:")
        for err in errors:
            logger.error(f" - {err}")
        print("\nSilakan lengkapi TELEGRAM_API_ID dan TELEGRAM_API_HASH di file .env.")
        return

    logger.info("Menginisialisasi Telegram Client...")
    from telethon.sessions import StringSession  # type: ignore

    session_target = (
        StringSession(Config.TELEGRAM_STRING_SESSION)
        if Config.TELEGRAM_STRING_SESSION
        else Config.TELEGRAM_SESSION_NAME
    )
    client = TelegramClient(
        session_target,
        Config.TELEGRAM_API_ID,
        Config.TELEGRAM_API_HASH,
    )

    # Inisialisasi Vision Recognizer (IQDB + SauceNAO + Trace.moe + Google Lens)
    recognizer = get_recognizer(
        iqdb_min_sim=Config.IQDB_MIN_SIMILARITY,
        tracemoe_min_sim=Config.TRACEMOE_MIN_SIMILARITY,
        saucenao_api_key=Config.SAUCENAO_API_KEY,
        saucenao_min_sim=Config.SAUCENAO_MIN_SIMILARITY,
        lens_enabled=Config.LENS_ENABLED,
    )

    # Inisialisasi Claimer
    claimer = Claimer(
        command_prefix=Config.CLAIM_COMMAND,
        name_format=Config.NAME_FORMAT,
        min_delay=Config.MIN_DELAY_SECONDS,
        max_delay=Config.MAX_DELAY_SECONDS,
        verify_timeout=Config.VERIFY_TIMEOUT_SECONDS,
        success_keywords=Config.SUCCESS_KEYWORDS,
        fail_keywords=Config.FAIL_KEYWORDS,
    )

    # Inisialisasi Listener
    listener = WaifuListener(client, recognizer, claimer)
    listener.register()

    # Mulai client
    await client.start()  # type: ignore
    me = await client.get_me()
    me_name = getattr(me, "first_name", "User")
    me_username = getattr(me, "username", None) or "No Username"
    me_id = getattr(me, "id", 0)
    logger.info(f"Berhasil terhubung sebagai: {me_name} (@{me_username}) [ID: {me_id}]")
    logger.info("Bot siap! Menunggu kemunculan waifu di grup...")

    await client.run_until_disconnected()  # type: ignore


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except (KeyboardInterrupt, SystemExit):
        logger.info("Bot dihentikan oleh pengguna.")
