import io
import time
import logging
import dataclasses
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

        # Analisis gambar dengan multi-engine sequential fallback
        engines = (
            self.recognizer.get_engines()
            if hasattr(self.recognizer, "get_engines")
            else [("Recognizer", self.recognizer)]
        )

        tried_names: set = set()  # Nama yang sudah pernah dicoba (mencegah spam duplikat)
        claimed_successfully = False
        last_result = None
        last_character = None

        for engine_idx, (engine_name, engine) in enumerate(engines, 1):
            logger.info(f"🔍 [{engine_idx}/{len(engines)}] Mencoba pengenalan karakter via {engine_name}...")
            character = await engine.identify(image_bytes)

            if not character or not character.full_name:
                logger.info(f"ℹ️ {engine_name} tidak menemukan kandidat karakter.")
                continue

            # Bangun daftar nama yang akan dicoba untuk engine ini:
            # Mulai dari full_name, lalu coba alternate_names (dari TraceMoe+AniList dll)
            names_to_try = [character.full_name]
            if character.alternate_names:
                names_to_try.extend(character.alternate_names)

            # Filter nama yang sudah pernah dicoba
            names_to_try = [n for n in names_to_try if n.strip().lower() not in tried_names]

            if not names_to_try:
                logger.info(f"ℹ️ Semua kandidat nama dari {engine_name} sudah pernah dicoba. Lanjut engine berikutnya...")
                continue

            logger.info(
                f"Karakter Ditemukan ({engine_name}): {character.full_name} "
                f"(Seri: {character.series or 'Unknown'}) | "
                f"Confidence: {character.confidence:.0%} | "
                f"Kandidat nama: {names_to_try}"
            )

            # Coba setiap nama kandidat dari engine ini
            engine_success = False
            for name_idx, candidate_name in enumerate(names_to_try, 1):
                norm_name = candidate_name.strip().lower()
                if norm_name in tried_names:
                    continue
                tried_names.add(norm_name)

                # Buat CharacterInfo sementara dengan nama yang sedang dicoba
                # supaya claimer bisa kirim nama yang benar
                import dataclasses
                char_with_candidate = dataclasses.replace(
                    character,
                    full_name=candidate_name,
                )

                if name_idx > 1:
                    logger.info(f"🔄 Mencoba karakter alternatif ke-{name_idx}: '{candidate_name}'...")

                result = await self.claimer.execute_claim(
                    client=self.client,
                    chat_id=event.chat_id,
                    character=char_with_candidate,
                    reply_to_msg_id=msg.id,
                    game_sender_id=game_sender_id,
                )

                last_result = result
                last_character = char_with_candidate

                if result.success:
                    claimed_successfully = True
                    engine_success = True
                    self.stats["claimed"] += 1
                    logger.info(
                        f"🎉 Klaim BERHASIL dengan {engine_name}! "
                        f"Nama yang berhasil: '{result.name}' "
                        f"(Karakter asli dari engine: '{character.full_name}')"
                    )

                    event_payload = {
                        "type": "claim_result",
                        "timestamp": time.time(),
                        "character_name": candidate_name,
                        "first_name": char_with_candidate.first_name,
                        "last_name": char_with_candidate.last_name,
                        "series": character.series or "Unknown",
                        "confidence": round(character.confidence * 100, 1),
                        "source": character.source or engine_name,
                        "success": True,
                        "claim_name": result.name,
                        "response": result.response_text,
                        "chat_id": event.chat_id,
                        "bot_name": sender_name,
                    }
                    await self._emit_event(event_payload)
                    break
                else:
                    logger.warning(
                        f"❌ Klaim '{candidate_name}' ditolak oleh game bot. "
                        f"{'Mencoba karakter alternatif berikutnya...' if name_idx < len(names_to_try) else 'Beralih ke engine berikutnya...'}"
                    )

            if engine_success or claimed_successfully:
                break

            if not engine_success:
                logger.warning(
                    f"❌ Semua kandidat dari {engine_name} ditolak. "
                    f"Beralih ke reverse search engine berikutnya..."
                )

        if not claimed_successfully:
            self.stats["failed"] += 1
            logger.error(
                "💀 Seluruh engine pencarian selesai dicoba dan tidak ada klaim yang berhasil."
            )
            event_payload = {
                "type": "claim_failed",
                "timestamp": time.time(),
                "reason": (
                    f"Klaim gagal ditolak bot: {last_result.response_text}"
                    if last_result and last_result.response_text
                    else "Karakter tidak dikenali di semua engine pencarian"
                ),
                "chat_id": event.chat_id,
                "bot_name": sender_name,
            }
            if last_character:
                event_payload["character_name"] = last_character.full_name
                event_payload["series"] = last_character.series
            await self._emit_event(event_payload)
