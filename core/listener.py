import io
import time
import logging
from collections import deque
from typing import List, Callable, Optional, Dict, Any
from telethon import events, TelegramClient  # type: ignore
from config import Config
from recognizer.base import BaseRecognizer
from .claimer import Claimer

logger = logging.getLogger("antikarbit.listener")


class WaifuListener:
    def __init__(self, client: TelegramClient, recognizer: BaseRecognizer, claimer: Claimer):
        self.client = client
        self.recognizer = recognizer
        self.claimer = claimer
        self.is_active: bool = True
        self.event_callbacks: List[Callable[[Dict[str, Any]], Any]] = []
        self.stats = {
            "detected": 0,
            "claimed": 0,
            "failed": 0,
            "start_time": time.time(),
        }
        self.recent_events: deque = deque(maxlen=50)

    def add_event_callback(self, cb: Callable[[Dict[str, Any]], Any]):
        self.event_callbacks.append(cb)

    async def _emit_event(self, event_data: Dict[str, Any]):
        self.recent_events.append(event_data)
        for cb in self.event_callbacks:
            try:
                res = cb(event_data)
                if hasattr(res, "__await__"):
                    await res
            except Exception as e:
                logger.debug(f"Error in event callback: {e}")

    def register(self):
        """Mendaftarkan handler event NewMessage ke Telethon client."""
        chats = Config.TARGET_CHAT_IDS if Config.TARGET_CHAT_IDS else None

        @self.client.on(events.NewMessage(chats=chats))
        async def on_new_message(event: events.NewMessage.Event):
            if not self.is_active:
                return
            await self._handle_message(event)

        logger.info("WaifuListener berhasil didaftarkan pada client.")

    async def _handle_message(self, event: events.NewMessage.Event):
        msg = event.message
        if not msg:
            return

        text = msg.raw_text or ""

        # Periksa apakah pesan mengandung salah satu kata kunci pemicu
        matched_trigger = any(kw.lower() in text.lower() for kw in Config.TRIGGER_KEYWORDS)
        if not matched_trigger:
            return

        # Periksa apakah pesan menyertakan gambar
        has_photo = bool(msg.photo)
        has_image_doc = (
            msg.media
            and hasattr(msg.media, "document")
            and getattr(msg.media.document, "mime_type", "").startswith("image/")
        )
        if not has_photo and not has_image_doc:
            logger.debug("Pesan memicu kata kunci, tetapi tidak mengandung gambar.")
            return

        self.stats["detected"] += 1

        # Identifikasi pengirim game bot untuk verifikasi klaim nantinya
        sender = await event.get_sender()
        sender_name = getattr(sender, "username", None) or getattr(sender, "first_name", "Unknown")
        game_sender_id = getattr(sender, "id", None)

        logger.info(f"==> KARAKTER MUNCUL! Dari bot: @{sender_name} (ID:{game_sender_id}) di Chat: {event.chat_id} <==")

        # Download gambar langsung ke memory (bytes)
        try:
            image_stream = io.BytesIO()
            await event.download_media(file=image_stream)
            image_bytes = image_stream.getvalue()

            if not image_bytes:
                logger.warning("Gagal mengunduh gambar karakter.")
                return

            logger.info(f"Ukuran gambar: {len(image_bytes)} bytes. Mulai pengenalan reverse search...")
        except Exception as e:
            logger.error(f"Error saat mengunduh gambar: {e}")
            return

        # Analisis gambar dengan Vision Recognizer
        character = await self.recognizer.identify(image_bytes)
        if not character:
            logger.warning("Karakter tidak berhasil dikenali oleh engine pencarian.")
            await self._emit_event({
                "type": "claim_failed",
                "timestamp": time.time(),
                "reason": "Karakter tidak dikenali di IQDB/Trace.moe",
                "chat_id": event.chat_id,
                "bot_name": sender_name,
            })
            return

        logger.info(
            f"Karakter Ditemukan: {character.full_name} "
            f"(First: {character.first_name}, Last: {character.last_name}) "
            f"dari [{character.series}] | Confidence: {character.confidence:.0%}"
        )

        # Lakukan klaim otomatis dengan verifikasi
        result = await self.claimer.execute_claim(
            client=self.client,
            chat_id=event.chat_id,
            character=character,
            reply_to_msg_id=msg.id,
            game_sender_id=game_sender_id,
        )

        event_payload = {
            "type": "claim_result",
            "timestamp": time.time(),
            "character_name": character.full_name,
            "first_name": character.first_name,
            "last_name": character.last_name,
            "series": character.series or "Unknown",
            "confidence": round(character.confidence * 100, 1),
            "source": character.source,
            "success": result.success,
            "claim_name": result.name,
            "response": result.response_text,
            "chat_id": event.chat_id,
            "bot_name": sender_name,
        }

        if result.success:
            self.stats["claimed"] += 1
            logger.info(f"🎉 Klaim selesai! Nama yang berhasil: '{result.name}'")
        else:
            self.stats["failed"] += 1
            logger.error(
                f"💀 Klaim gagal untuk '{character.full_name}'. "
                "Semua variasi nama sudah dicoba namun ditolak game bot."
            )

        await self._emit_event(event_payload)
