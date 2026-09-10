import asyncio
import random
import logging
from typing import TYPE_CHECKING, Optional, List
from recognizer.base import CharacterInfo

if TYPE_CHECKING:
    from telethon import TelegramClient

logger = logging.getLogger("antikarbit.claimer")

# Urutan fallback nama yang dicoba secara otomatis jika klaim ditolak game bot
# Contoh: full_name gagal → first_name → last_name → full_name tanpa spasi
def _build_name_candidates(character: CharacterInfo, name_format: str = "full") -> List[str]:
    """
    Membangun daftar nama untuk klaim.
    Sesuai permintaan untuk mengurangi spam di grup:
    Hanya mengirim 1 nama spesifik (default: nama lengkap).
    """
    if name_format == "first" and character.first_name:
        return [character.first_name.strip()]
    return [character.full_name.strip()]


class ClaimResult:
    """Hasil dari satu percobaan klaim."""
    def __init__(self, name: str, success: bool, response_text: Optional[str] = None):
        self.name = name
        self.success = success
        self.response_text = response_text

    def __repr__(self):
        status = "✅ BERHASIL" if self.success else "❌ GAGAL"
        return f"ClaimResult({status}, name='{self.name}', response='{self.response_text}')"


class Claimer:
    def __init__(
        self,
        command_prefix: str = "/protecc",
        name_format: str = "full",
        min_delay: float = 0.5,
        max_delay: float = 1.5,
        verify_timeout: float = 5.0,
        success_keywords: Optional[List[str]] = None,
        fail_keywords: Optional[List[str]] = None,
    ):
        self.command_prefix = command_prefix
        self.name_format = name_format
        self.min_delay = min_delay
        self.max_delay = max_delay
        self.verify_timeout = verify_timeout
        self.success_keywords = success_keywords or [
            "now protected", "added to your harem", "added to your collection",
            "is now yours", "congratulations"
        ]
        self.fail_keywords = fail_keywords or [
            "not quite right", "wrong name", "already claimed",
            "already protecc", "rip", "try again"
        ]

    async def _wait_for_game_reply(
        self,
        client: "TelegramClient",
        chat_id: int,
        after_msg_id: int,
        game_sender_id: Optional[int],
    ) -> Optional[str]:
        """
        Menunggu balasan dari game bot setelah klaim dikirim.
        Mengembalikan teks balasan, atau None jika timeout.
        """
        import time
        deadline = time.monotonic() + self.verify_timeout

        while time.monotonic() < deadline:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break

            try:
                # Ambil pesan-pesan terbaru di chat setelah msg_id yang kita kirim
                messages = await client.get_messages(
                    chat_id,
                    min_id=after_msg_id,
                    limit=5,
                )
                for msg in messages:
                    # Hanya pesan yang bukan dari kita sendiri
                    me = await client.get_me()
                    if msg.sender_id == me.id:
                        continue
                    # Filter ke pengirim game bot jika kita tahu ID-nya
                    if game_sender_id and msg.sender_id != game_sender_id:
                        continue
                    text = msg.raw_text or ""
                    if text:
                        return text
            except Exception as e:
                logger.debug(f"Error saat polling balasan: {e}")

            await asyncio.sleep(0.8)

        return None

    def _detect_result(self, response_text: str) -> Optional[bool]:
        """
        Menganalisis teks balasan game bot.
        Returns: True = berhasil, False = gagal, None = tidak diketahui
        """
        text_lower = response_text.lower()
        if any(kw in text_lower for kw in self.success_keywords):
            return True
        if any(kw in text_lower for kw in self.fail_keywords):
            return False
        return None

    async def execute_claim(
        self,
        client: "TelegramClient",
        chat_id: int,
        character: CharacterInfo,
        reply_to_msg_id: Optional[int] = None,
        game_sender_id: Optional[int] = None,
    ) -> ClaimResult:
        """
        Mengirim perintah klaim dengan verifikasi balasan game bot.
        Jika gagal (nama ditolak), otomatis retry dengan nama alternatif.
        """
        # Jeda awal seperti manusia
        delay = random.uniform(self.min_delay, self.max_delay)
        logger.info(f"Menunggu jeda {delay:.2f}s sebelum klaim...")
        await asyncio.sleep(delay)

        # Bangun nama untuk klaim
        candidates = _build_name_candidates(character, self.name_format)
        logger.info(f"Kandidat nama untuk klaim: {candidates}")

        for idx, name in enumerate(candidates):
            cmd = f"{self.command_prefix} {name}"
            if idx > 0:
                logger.info(f"🔄 Retry percobaan ke-{idx + 1} dengan nama alternatif...")
                await asyncio.sleep(random.uniform(0.8, 1.5))

            logger.info(f"📤 Mengirim: '{cmd}' ke chat {chat_id}")
            try:
                sent = await client.send_message(
                    chat_id,
                    cmd,
                    reply_to=reply_to_msg_id,
                )
            except Exception as e:
                logger.error(f"Gagal mengirim pesan '{cmd}': {e}")
                continue

            # Tunggu balasan dari game bot
            logger.info(f"⏳ Menunggu balasan game bot (timeout {self.verify_timeout}s)...")
            response = await self._wait_for_game_reply(
                client, chat_id, sent.id, game_sender_id
            )

            if response is None:
                logger.warning(f"⚠️  Tidak ada balasan dari game bot dalam {self.verify_timeout}s untuk '{name}'. Anggap berhasil.")
                return ClaimResult(name=name, success=True, response_text=None)

            logger.info(f"📨 Balasan game bot: \"{response[:120]}\"")
            result = self._detect_result(response)

            if result is True:
                logger.info(f"✅ KLAIM BERHASIL! '{character.full_name}' berhasil diklaim dengan nama '{name}'!")
                return ClaimResult(name=name, success=True, response_text=response)

            elif result is False:
                logger.warning(f"❌ Klaim '{name}' ditolak. Mencoba nama berikutnya...")
                continue

            else:
                # Balasan tidak dikenal — asumsikan berhasil dan berhenti
                logger.info(f"❓ Balasan tidak dikenal untuk '{name}', dianggap berhasil.")
                return ClaimResult(name=name, success=True, response_text=response)

        # Semua kandidat habis dicoba dan semuanya gagal
        logger.error(f"💀 Semua kandidat nama gagal diklaim untuk karakter '{character.full_name}'.")
        return ClaimResult(name=candidates[0] if candidates else "", success=False, response_text=None)
