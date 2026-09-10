import re
import json
import logging
from typing import Optional, List, Tuple
import aiohttp  # type: ignore

from .base import BaseRecognizer, CharacterInfo
from .anilist_lookup import lookup_series_from_character

logger = logging.getLogger("antikarbit.lens")

# Header browser yang realistis untuk Google Lens
_BROWSER_HEADERS = {
    "User-Agent": (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
        "AppleWebKit/537.36 (KHTML, like Gecko) "
        "Chrome/124.0.0.0 Safari/537.36"
    ),
    "Accept-Language": "en-US,en;q=0.9",
    "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
    "Sec-Fetch-Site": "same-origin",
    "Sec-Fetch-Mode": "navigate",
    "Sec-Fetch-User": "?1",
    "Sec-Fetch-Dest": "document",
}

# Kata kunci yang mengindikasikan hasil visual dari Lens adalah karakter anime
_ANIME_HINTS = {
    "anime", "manga", "character", "vtuber", "virtual youtuber",
    "hololive", "nijisanji", "touhou", "genshin", "arknights",
    "blue archive", "fate", "kancolle", "idolmaster",
}

# Kata-kata blacklist yang sering muncul di hasil Lens tapi bukan nama karakter
_LENS_BLACKLIST_WORDS = {
    "illustration", "artwork", "wallpaper", "fanart", "official art",
    "download", "pinterest", "twitter", "instagram", "deviantart",
    "pixiv", "zerochan", "danbooru", "gelbooru",
    "resolution", "pixels", "image", "photo", "picture",
    "figure", "merchandise", "poster", "print", "acrylic",
    "cosplay", "costume", "wig",
}


def _extract_character_from_lens_text(text_snippets: List[str]) -> Tuple[Optional[str], Optional[str]]:
    """
    Analisis teks hasil Google Lens untuk mencari nama karakter anime.
    Mengembalikan (character_name, series_name) atau (None, None).
    """
    # Pola umum nama karakter: "Character Name from Series" atau "Character Name (Series)"
    for snippet in text_snippets:
        if not snippet or len(snippet) < 3:
            continue

        snippet_lower = snippet.lower()

        # Filter snippet yang jelas bukan nama karakter
        if any(bl in snippet_lower for bl in _LENS_BLACKLIST_WORDS):
            continue

        # Pola: "X from Y" atau "X - Y"
        match_from = re.search(r"^(.+?)\s+(?:from|in|of)\s+(.+)$", snippet, re.IGNORECASE)
        if match_from:
            char_part = match_from.group(1).strip()
            series_part = match_from.group(2).strip()
            # Validasi: nama karakter tidak boleh terlalu panjang
            if 2 <= len(char_part.split()) <= 4 and len(char_part) <= 40:
                return char_part, series_part

        # Pola: "X (Y)"
        match_paren = re.match(r"^(.+?)\s*\(([^)]+)\)\s*$", snippet)
        if match_paren:
            char_part = match_paren.group(1).strip()
            series_part = match_paren.group(2).strip()
            if 1 <= len(char_part.split()) <= 4 and len(char_part) <= 40:
                return char_part, series_part

        # Potensi nama karakter standalone: 1-4 kata, kapital, tidak terlalu panjang
        words = snippet.split()
        if (
            1 <= len(words) <= 4
            and len(snippet) <= 40
            and all(w[0].isupper() for w in words if w and w[0].isalpha())
            and not any(snippet_lower.startswith(bl) for bl in _LENS_BLACKLIST_WORDS)
        ):
            return snippet, None

    return None, None


def _parse_lens_response(html: str) -> List[str]:
    """
    Parse HTML response dari Google Lens untuk mengekstrak teks-teks hasil pencarian visual.
    Google Lens menyimpan data di blok JSON `AF_initDataCallback`.
    """
    text_results: List[str] = []

    # Coba extract dari blok AF_initDataCallback (format data utama Google Lens)
    callbacks = re.findall(r'AF_initDataCallback\(({.*?})\)', html, re.DOTALL)
    for cb in callbacks:
        try:
            # Ambil value dari "data": [...]
            data_match = re.search(r'"data":\s*(\[.*?\])\s*[,}]', cb, re.DOTALL)
            if not data_match:
                continue
            # Cari semua string dalam struktur data
            strings = re.findall(r'"([^"]{3,80})"', data_match.group(1))
            text_results.extend(strings)
        except Exception:
            continue

    # Fallback: cari title dari tag <h3> atau <span> di dalam div hasil visual
    h3_texts = re.findall(r'<h3[^>]*>([^<]+)</h3>', html)
    text_results.extend(h3_texts)

    # Fallback: cari tag <title> dari halaman (sering berisi query)
    title_match = re.search(r'<title>([^<]+)</title>', html)
    if title_match:
        title = title_match.group(1).replace(" - Google Search", "").strip()
        if title:
            text_results.append(title)

    # Cari string pendek (kemungkinan nama) dari dalam JSON embedding
    json_strings = re.findall(r'"([A-Z][a-zA-Z\s]{3,40})"', html)
    text_results.extend(json_strings)

    # Deduplicate sambil tetap mempertahankan urutan
    seen = set()
    unique_results = []
    for t in text_results:
        t_clean = t.strip()
        if t_clean and t_clean not in seen:
            seen.add(t_clean)
            unique_results.append(t_clean)

    return unique_results


class GoogleLensRecognizer(BaseRecognizer):
    """
    Mencari identitas karakter anime melalui Google Lens (https://lens.google.com).
    Digunakan sebagai fallback terakhir setelah IQDB, SauceNAO, dan Trace.moe gagal.
    100% Gratis dan TANPA API KEY — menggunakan endpoint publik Google Lens.

    Catatan: Karena Google Lens tidak memberikan metadata terstruktur, akurasi
    bergantung pada parsing teks dari hasil visual. Hasilnya dikonfirmasi via
    AniList untuk memastikan nama yang digunakan adalah nama karakter, bukan judul anime.
    """

    def __init__(self, enabled: bool = True):
        self.enabled = enabled
        self.upload_endpoint = "https://lens.google.com/v3/upload"

    async def identify(self, image_bytes: bytes) -> Optional[CharacterInfo]:
        if not self.enabled:
            logger.debug("Google Lens Recognizer dinonaktifkan, melewati...")
            return None

        # Deteksi mime type
        mime_type = "image/jpeg"
        if image_bytes.startswith(b"\x89PNG"):
            mime_type = "image/png"
        elif image_bytes.startswith(b"RIFF") and b"WEBP" in image_bytes[:16]:
            mime_type = "image/webp"

        # Buat cookie jar untuk session yang valid
        cookie_jar = aiohttp.CookieJar()

        try:
            logger.info("Mengirim gambar ke Google Lens untuk pencarian visual...")

            # Header khusus untuk upload Lens
            upload_headers = {
                **_BROWSER_HEADERS,
                "Referer": "https://lens.google.com/",
                "Origin": "https://lens.google.com",
                "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
            }

            # Buat multipart form data
            form_data = aiohttp.FormData()
            form_data.add_field(
                "encoded_image",
                image_bytes,
                filename="image.jpg",
                content_type=mime_type,
            )
            form_data.add_field("image_content", "")
            form_data.add_field("re", "df")
            form_data.add_field("s", "4")
            form_data.add_field("st", "")
            form_data.add_field("lp", "1")

            async with aiohttp.ClientSession(
                cookie_jar=cookie_jar,
                headers=_BROWSER_HEADERS,
            ) as session:
                # Inisiasi cookie dengan kunjungan ke Google terlebih dahulu
                try:
                    async with session.get(
                        "https://www.google.com/",
                        timeout=aiohttp.ClientTimeout(total=8),
                        allow_redirects=True,
                    ) as _:
                        pass
                except Exception:
                    pass  # Jika gagal, lanjutkan saja

                # Upload gambar ke Google Lens
                async with session.post(
                    self.upload_endpoint,
                    data=form_data,
                    headers=upload_headers,
                    timeout=aiohttp.ClientTimeout(total=20),
                    allow_redirects=True,
                    max_redirects=5,
                ) as resp:
                    if resp.status not in (200, 302):
                        logger.warning(f"Google Lens HTTP {resp.status}")
                        return None

                    html = await resp.text()

            if not html or len(html) < 500:
                logger.warning("Google Lens mengembalikan respons kosong atau sangat pendek.")
                return None

            # Parse hasil dari HTML response
            text_snippets = _parse_lens_response(html)

            if not text_snippets:
                logger.warning("Google Lens tidak dapat mengekstrak teks dari respons.")
                return None

            logger.info(
                f"Google Lens menghasilkan {len(text_snippets)} fragmen teks. "
                f"Beberapa contoh: {text_snippets[:5]}"
            )

            # Analisis teks untuk cari nama karakter
            char_name, series_name = _extract_character_from_lens_text(text_snippets)

            if not char_name:
                logger.warning("Google Lens tidak berhasil mengidentifikasi nama karakter dari hasil visual.")
                return None

            logger.info(f"Google Lens mendeteksi kandidat: '{char_name}' dari '{series_name or 'Unknown'}'")

            # Verifikasi via AniList untuk memastikan ini adalah nama karakter valid
            anilist_name, anilist_series = await lookup_series_from_character(char_name)

            if anilist_name:
                logger.info(f"AniList mengkonfirmasi: '{char_name}' → '{anilist_name}' dari '{anilist_series}'")
                char_name = anilist_name
                if anilist_series:
                    series_name = anilist_series
            else:
                logger.info(
                    f"AniList tidak menemukan '{char_name}' — menggunakan hasil Lens apa adanya. "
                    "Mungkin karakter VTuber atau non-AniList."
                )

            # Parse first/last name
            parts = char_name.split()
            if len(parts) >= 2:
                first_name = parts[-1]
                last_name = " ".join(parts[:-1])
            else:
                first_name = char_name
                last_name = None

            logger.info(
                f"Google Lens Berhasil: {char_name} "
                f"(Seri: {series_name or 'Unknown'})"
            )

            return CharacterInfo(
                full_name=char_name,
                first_name=first_name,
                last_name=last_name,
                series=series_name,
                confidence=0.60,  # Confidence lebih rendah karena heuristik text-parsing
                source="google_lens",
            )

        except Exception as e:
            logger.error(f"Error saat mencari di Google Lens: {e}")
            return None
