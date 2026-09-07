import re
import logging
from typing import Optional
import aiohttp  # type: ignore

from .base import BaseRecognizer, CharacterInfo

logger = logging.getLogger("antikarbit.tracemoe")


class TraceMoeRecognizer(BaseRecognizer):
    """
    Mencari asal adegan anime menggunakan Trace.moe API (https://trace.moe).
    Sangat akurat untuk gambar berupa screenshot/frame langsung dari episode anime.
    100% Gratis dan TANPA API KEY apapun.
    """

    def __init__(self, min_similarity: float = 0.80):
        self.endpoint = "https://api.trace.moe/search?anilistInfo=1"
        self.min_similarity = min_similarity

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
                            f"Kecocokan Trace.moe ({similarity:.1%}) di bawah ambang {self.min_similarity:.0%}."
                        )
                        return None

                    anilist = top.get("anilist", {})
                    title_obj = anilist.get("title", {})
                    series_name = (
                        title_obj.get("romaji")
                        or title_obj.get("english")
                        or title_obj.get("native")
                        or "Unknown Anime"
                    )

                    episode = top.get("episode")
                    filename = top.get("filename", "")

                    logger.info(
                        f"Trace.moe Menemukan Adegan: '{series_name}' "
                        f"(Episode: {episode}, Kemiripan: {similarity:.1%})"
                    )

                    # Bersihkan nama seri untuk kandidat
                    clean_series = re.sub(r"\s*\(TV\)\s*", "", series_name)

                    return CharacterInfo(
                        full_name=clean_series,
                        first_name=clean_series.split()[0] if clean_series else clean_series,
                        last_name=None,
                        series=clean_series,
                        confidence=similarity,
                        source="trace_moe",
                    )

        except Exception as e:
            logger.error(f"Error saat mencari di Trace.moe: {e}")
            return None
