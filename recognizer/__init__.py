import logging
from typing import Optional, List, Tuple
from .base import BaseRecognizer, CharacterInfo
from .iqdb_vision import IQDBRecognizer
from .tracemoe_vision import TraceMoeRecognizer
from .saucenao_vision import SauceNAORecognizer
from .lens_vision import GoogleLensRecognizer

logger = logging.getLogger("antikarbit.recognizer")


class InternetSearchRecognizer(BaseRecognizer):
    """
    Recognizer berbasis Multi-Engine Pencarian Gambar di Internet:
    1. IQDB.org (Danbooru, Gelbooru, Konachan, yande.re, Anime-Pictures).
    2. SauceNAO (Pixiv, Twitter/X, Danbooru, anime, game CG) — butuh API Key gratis.
    3. Trace.moe (Adegan anime dengan resolusi karakter AniList).
    4. Google Lens (Fallback terakhir, heuristik text parsing + AniList verify).

    Setiap engine dicoba secara berurutan. Jika engine menghasilkan karakter
    tetapi klaim ditolak game bot, listener akan mencoba engine berikutnya.
    """

    def __init__(
        self,
        iqdb_min_sim: float = 60.0,
        tracemoe_min_sim: float = 0.85,
        saucenao_api_key: str = "",
        saucenao_min_sim: float = 70.0,
        lens_enabled: bool = True,
    ):
        self.iqdb = IQDBRecognizer(min_similarity=iqdb_min_sim)
        self.tracemoe = TraceMoeRecognizer(min_similarity=tracemoe_min_sim)
        self.saucenao = SauceNAORecognizer(api_key=saucenao_api_key, min_similarity=saucenao_min_sim)
        self.lens = GoogleLensRecognizer(enabled=lens_enabled)

    def get_engines(self) -> List[Tuple[str, BaseRecognizer]]:
        """
        Daftar engine pencarian berurutan untuk fallback bertahap.
        Urutan: IQDB → SauceNAO (jika ada API key) → Trace.moe → Google Lens
        """
        engines: List[Tuple[str, BaseRecognizer]] = [
            ("IQDB", self.iqdb),
        ]
        if self.saucenao.api_key:
            engines.append(("SauceNAO", self.saucenao))
        engines.append(("Trace.moe", self.tracemoe))
        if self.lens.enabled:
            engines.append(("Google Lens", self.lens))
        return engines

    async def identify(self, image_bytes: bytes) -> Optional[CharacterInfo]:
        """Metode default yang mencoba engine secara berurutan hingga menemukan hasil."""
        engines = self.get_engines()
        for idx, (name, engine) in enumerate(engines, 1):
            logger.info(f"🌐 [{idx}/{len(engines)}] Mencari gambar di {name}...")
            res = await engine.identify(image_bytes)
            if res and res.full_name:
                logger.info(f"✨ {name} berhasil mengenali: {res.full_name}")
                return res

        logger.warning("❌ Gambar tidak berhasil dikenali oleh seluruh engine pencarian.")
        return None


# Alias
SearchVisionRecognizer = InternetSearchRecognizer


def get_recognizer(
    iqdb_min_sim: float = 60.0,
    tracemoe_min_sim: float = 0.85,
    saucenao_api_key: str = "",
    saucenao_min_sim: float = 70.0,
    lens_enabled: bool = True,
) -> InternetSearchRecognizer:
    """Mengembalikan multi-engine recognizer internet."""
    engine_list = ["IQDB"]
    if saucenao_api_key:
        engine_list.append("SauceNAO")
    engine_list.append("Trace.moe")
    if lens_enabled:
        engine_list.append("Google Lens")

    logger.info(
        f"Menggunakan InternetSearchRecognizer: {' → '.join(engine_list)} | "
        f"IQDB min: {iqdb_min_sim}%, "
        f"Trace.moe min: {tracemoe_min_sim:.0%}, "
        f"SauceNAO: {'Aktif' if saucenao_api_key else 'Nonaktif'}, "
        f"Google Lens: {'Aktif' if lens_enabled else 'Nonaktif'}"
    )
    return InternetSearchRecognizer(
        iqdb_min_sim=iqdb_min_sim,
        tracemoe_min_sim=tracemoe_min_sim,
        saucenao_api_key=saucenao_api_key,
        saucenao_min_sim=saucenao_min_sim,
        lens_enabled=lens_enabled,
    )


__all__ = [
    "BaseRecognizer",
    "CharacterInfo",
    "IQDBRecognizer",
    "TraceMoeRecognizer",
    "SauceNAORecognizer",
    "GoogleLensRecognizer",
    "InternetSearchRecognizer",
    "SearchVisionRecognizer",
    "get_recognizer",
]
