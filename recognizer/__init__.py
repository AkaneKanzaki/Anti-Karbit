import logging
from typing import Optional
from .base import BaseRecognizer, CharacterInfo
from .iqdb_vision import IQDBRecognizer
from .tracemoe_vision import TraceMoeRecognizer

logger = logging.getLogger("antikarbit.recognizer")


class InternetSearchRecognizer(BaseRecognizer):
    """
    Recognizer berbasis Pencarian Gambar di Internet (Reverse Image Search):
    1. Mencari di IQDB.org (Danbooru, Gelbooru, Konachan, yande.re, Anime-Pictures, e-shuushuu).
    2. Fallback ke Trace.moe jika gambar adalah cuplikan adegan anime.
    100% Gratis, 0 API Key, 0 Biaya, dan Bebas dari Limit Kuota AI!
    """

    def __init__(self, iqdb_min_sim: float = 55.0, tracemoe_min_sim: float = 0.80):
        self.iqdb = IQDBRecognizer(min_similarity=iqdb_min_sim)
        self.tracemoe = TraceMoeRecognizer(min_similarity=tracemoe_min_sim)

    async def identify(self, image_bytes: bytes) -> Optional[CharacterInfo]:
        # 1. Coba pencarian di IQDB terlebih dahulu (paling akurat untuk fanart & booru)
        logger.info("🌐 [1/2] Mencari gambar di database IQDB...")
        res = await self.iqdb.identify(image_bytes)
        if res and res.full_name:
            logger.info(f"✨ IQDB berhasil mengenali: {res.full_name}")
            return res

        # 2. Fallback ke Trace.moe jika bukan fanart atau merupakan frame anime
        logger.info("🌐 [2/2] Mencoba pencarian adegan di Trace.moe...")
        res_trace = await self.tracemoe.identify(image_bytes)
        if res_trace and res_trace.full_name:
            logger.info(f"✨ Trace.moe berhasil mengenali: {res_trace.full_name}")
            return res_trace

        logger.warning("❌ Gambar tidak ditemukan baik di IQDB maupun Trace.moe.")
        return None


# Alias
SearchVisionRecognizer = InternetSearchRecognizer


def get_recognizer(
    iqdb_min_sim: float = 40.0,
    tracemoe_min_sim: float = 0.80,
) -> BaseRecognizer:
    """
    Mengembalikan recognizer berbasis pencarian internet IQDB + Trace.moe.
    """
    logger.info(
        f"Menggunakan InternetSearchRecognizer (IQDB min: {iqdb_min_sim}%, Trace.moe min: {tracemoe_min_sim:.0%}, 0 API Key)."
    )
    return InternetSearchRecognizer(
        iqdb_min_sim=iqdb_min_sim,
        tracemoe_min_sim=tracemoe_min_sim,
    )


__all__ = [
    "BaseRecognizer",
    "CharacterInfo",
    "IQDBRecognizer",
    "TraceMoeRecognizer",
    "InternetSearchRecognizer",
    "SearchVisionRecognizer",
    "get_recognizer",
]
