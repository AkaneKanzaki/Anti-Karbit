import re
import logging
from typing import Optional, List
import aiohttp  # type: ignore

from .base import BaseRecognizer, CharacterInfo
from .anilist_lookup import lookup_characters_by_media_id

logger = logging.getLogger("antikarbit.tracemoe")


class TraceMoeRecognizer(BaseRecognizer):
    """
    Mencari asal adegan anime menggunakan Trace.moe API (https://trace.moe).
    Jika adegan ditemukan, resolusi nama karakter dilakukan secara otomatis melalui AniList GraphQL API
    sehingga bot TIDAK PERNAH mengklaim judul anime sebagai nama karakter.
    100% Gratis dan TANPA API KEY.

    Mengembalikan beberapa kandidat karakter dari AniList sehingga listener dapat
    mencoba satu-persatu jika yang pertama ditolak oleh game bot.
    """

    def __init__(self, min_similarity: float = 0.85):
        self.endpoint = "https://api.trace.moe/search?anilistInfo=1"
        self._min_similarity = 0.85
        self.min_similarity = min_similarity

    @property
    def min_similarity(self) -> float:
        return self._min_similarity

    @min_similarity.setter
    def min_similarity(self, value: float):
        try:
            v = float(value)
            self._min_similarity = v / 100.0 if v > 1.0 else v
        except (ValueError, TypeError):
            self._min_similarity = 0.85

    async def identify(self, image_bytes: bytes) -> Optional[CharacterInfo]:
        headers = {
            "Content-Type": "image/jpeg",
            "User-Agent": (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
                "AppleWebKit/537.36 (KHTML, like Gecko) "
                "Chrome/120.0.0.0 Safari/537.36"
            ),
        }

        # Deteksi mime type
        if image_bytes.startswith(b"\x89PNG"):
            headers["Content-Type"] = "image/png"
        elif image_bytes.startswith(b"RIFF") and b"WEBP" in image_bytes[:16]:
            headers["Content-Type"] = "image/webp"

        try:
            logger.info("Mengirim gambar ke Trace.moe (Anime Scene Search)...")
            async with aiohttp.ClientSession() as session:
                async with session.post(
                    self.endpoint,
                    data=image_bytes,
                    headers=headers,
                    timeout=aiohttp.ClientTimeout(total=15),
                ) as resp:
                    if resp.status != 200:
                        err_text = await resp.text()
                        logger.warning(f"Trace.moe HTTP {resp.status}: {err_text[:100]}")
                        return None

                    data = await resp.json()
                    results = data.get("result", [])
                    if not results:
                        logger.warning("Trace.moe tidak menemukan kecocokan adegan anime.")
                        return None

                    top = results[0]
                    similarity = float(top.get("similarity", 0.0))

                    if similarity < self.min_similarity:
                        logger.info(
                            f"Kecocokan Trace.moe ({similarity:.1%}) di bawah ambang batas {self.min_similarity:.0%}."
                        )
                        return None

                    anilist = top.get("anilist", {})
                    media_id = anilist.get("id") if isinstance(anilist, dict) else anilist

                    title_obj = anilist.get("title", {}) if isinstance(anilist, dict) else {}
                    series_name = (
                        title_obj.get("romaji")
                        or title_obj.get("english")
                        or title_obj.get("native")
                        or "Unknown Anime"
                    )

                    episode = top.get("episode")
                    clean_series = re.sub(r"\s*\(TV\)\s*", "", series_name)

                    logger.info(
                        f"Trace.moe Menemukan Adegan: '{clean_series}' "
                        f"(Episode: {episode}, Kemiripan: {similarity:.1%})"
                    )

                    # Resolusi karakter anime dari AniList — ambil SEMUA kandidat
                    char_candidates: List[str] = []
                    if media_id and isinstance(media_id, int):
                        logger.info(f"Mencari karakter dari anime '{clean_series}' (ID: {media_id})...")
                        _, char_candidates = await lookup_characters_by_media_id(media_id)
                        if char_candidates:
                            logger.info(
                                f"AniList mengembalikan {len(char_candidates)} karakter: "
                                f"{', '.join(char_candidates[:3])}{'...' if len(char_candidates) > 3 else ''}"
                            )

                    if not char_candidates:
                        logger.warning(
                            f"Adegan anime '{clean_series}' ditemukan di Trace.moe, namun nama karakter "
                            "spesifik tidak dapat dipastikan. Membatalkan klaim agar tidak salah menyebut judul anime."
                        )
                        return None

                    # Gunakan karakter pertama sebagai primary, sisanya sebagai alternates
                    primary_char = char_candidates[0]
                    alternate_chars = char_candidates[1:]

                    parts = primary_char.split()
                    if len(parts) >= 2:
                        first_name = parts[-1]
                        last_name = " ".join(parts[:-1])
                    else:
                        first_name = primary_char
                        last_name = None

                    logger.info(f"Karakter utama Trace.moe: '{primary_char}'")
                    if alternate_chars:
                        logger.info(f"Kandidat alternatif: {alternate_chars}")

                    return CharacterInfo(
                        full_name=primary_char,
                        first_name=first_name,
                        last_name=last_name,
                        series=clean_series,
                        confidence=similarity,
                        source="trace_moe",
                        alternate_names=alternate_chars,
                    )

        except Exception as e:
            logger.error(f"Error saat mencari di Trace.moe: {e}")
            return None
