import re
import logging
from typing import Optional
import aiohttp  # type: ignore

from .base import BaseRecognizer, CharacterInfo
from .anilist_lookup import lookup_series_from_character

logger = logging.getLogger("antikarbit.saucenao")


def _clean_text(s: str) -> str:
    s = s.replace("_", " ")
    return " ".join([w.capitalize() for w in s.split() if w])


class SauceNAORecognizer(BaseRecognizer):
    """
    Mencari identitas karakter dan asal karya melalui SauceNAO (https://saucenao.com).
    Sangat ampuh untuk ilustrasi Pixiv, Twitter/X, Danbooru, dan fanart anime/VTuber.
    Mendukung API Key gratis (dapat diperoleh dari https://saucenao.com/user.php).
    """

    def __init__(self, api_key: str = "", min_similarity: float = 65.0):
        self.endpoint = "https://saucenao.com/search.php"
        self.api_key = api_key.strip()
        self.min_similarity = min_similarity

    async def identify(self, image_bytes: bytes) -> Optional[CharacterInfo]:
        if not self.api_key:
            logger.debug("SauceNAO API Key tidak diisi, melewati pencarian SauceNAO.")
            return None

        # Deteksi mime type
        mime_type = "image/jpeg"
        if image_bytes.startswith(b"\x89PNG"):
            mime_type = "image/png"
        elif image_bytes.startswith(b"RIFF") and b"WEBP" in image_bytes[:16]:
            mime_type = "image/webp"

        data = aiohttp.FormData()
        data.add_field("file", image_bytes, filename="waifu.jpg", content_type=mime_type)
        data.add_field("output_type", "2")
        data.add_field("numres", "5")
        if self.api_key:
            data.add_field("api_key", self.api_key)

        headers = {
            "User-Agent": (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
                "AppleWebKit/537.36 (KHTML, like Gecko) "
                "Chrome/120.0.0.0 Safari/537.36"
            ),
        }

        try:
            logger.info("Mengirim gambar ke SauceNAO...")
            async with aiohttp.ClientSession() as session:
                async with session.post(
                    self.endpoint,
                    data=data,
                    headers=headers,
                    timeout=aiohttp.ClientTimeout(total=15),
                ) as resp:
                    if resp.status != 200:
                        logger.warning(f"SauceNAO HTTP status {resp.status}")
                        return None

                    res_json = await resp.json()
                    results = res_json.get("results", [])
                    if not results:
                        logger.warning("SauceNAO tidak menemukan hasil gambar yang cocok.")
                        return None

            best_char = None
            best_series = None
            best_sim = 0.0

            for r in results:
                header = r.get("header", {})
                similarity_str = header.get("similarity", "0")
                try:
                    sim = float(similarity_str)
                except ValueError:
                    sim = 0.0

                if sim < self.min_similarity:
                    continue

                data_block = r.get("data", {})
                # Cari nama karakter dari field SauceNAO
                raw_char = (
                    data_block.get("characters")
                    or data_block.get("character")
                    or data_block.get("eng_name")
                    or data_block.get("jp_name")
                )

                raw_series = (
                    data_block.get("material")
                    or data_block.get("source")
                    or data_block.get("title")
                )

                char_name = None
                if raw_char:
                    # Ambil karakter pertama jika koma dipisahkan
                    first_c = raw_char.split(",")[0].strip()
                    # Bersihkan tanda kurung atau suffix
                    first_c = re.sub(r"\(.*?\)", "", first_c).strip()
                    if first_c:
                        char_name = _clean_text(first_c)

                if not char_name and raw_series and sim >= 80.0:
                    # Jika title mengandung nama karakter (misal di Pixiv title)
                    title = data_block.get("title", "")
                    if title and len(title.split()) <= 3 and not any(w in title.lower() for w in ["chapter", "ep", "vol"]):
                        char_name = _clean_text(title)

                if char_name and sim > best_sim:
                    best_sim = sim
                    best_char = char_name
                    if raw_series:
                        first_s = raw_series.split(",")[0].strip()
                        best_series = _clean_text(first_s)

            if not best_char:
                logger.warning("SauceNAO menemukan kecocokan tetapi tidak ada metadata karakter yang valid.")
                return None

            # Coba AniList jika series belum ada
            if not best_series:
                _, anilist_series = await lookup_series_from_character(best_char)
                if anilist_series:
                    best_series = anilist_series

            parts = best_char.split()
            if len(parts) >= 2:
                first_name = parts[-1]
                last_name = " ".join(parts[:-1])
            else:
                first_name = best_char
                last_name = None

            logger.info(f"SauceNAO Berhasil Menemukan: {best_char} (Seri: {best_series or 'Unknown'}, Sim: {best_sim}%)")
            return CharacterInfo(
                full_name=best_char,
                first_name=first_name,
                last_name=last_name,
                series=best_series,
                confidence=best_sim / 100.0,
                source="saucenao",
            )

        except Exception as e:
            logger.error(f"Error saat mencari di SauceNAO: {e}")
            return None
