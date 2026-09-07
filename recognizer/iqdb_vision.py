import re
import logging
from typing import Optional, List, Tuple
import aiohttp  # type: ignore

from .base import BaseRecognizer, CharacterInfo
from .anilist_lookup import lookup_series_from_character

logger = logging.getLogger("antikarbit.iqdb")

# Tag umum booru yang bukan nama karakter
GENERIC_TAGS = {
    "1girl", "2girls", "3girls", "4girls", "5girls", "6+girls", "multiple_girls",
    "1boy", "2boys", "solo", "highres", "absurdres", "incredibly_absurdres",
    "long_hair", "short_hair", "medium_hair", "very_long_hair",
    "black_hair", "brown_hair", "blonde_hair", "blue_hair", "pink_hair", "purple_hair",
    "red_hair", "white_hair", "green_hair", "silver_hair", "grey_hair", "orange_hair",
    "blue_eyes", "red_eyes", "brown_eyes", "green_eyes", "pink_eyes", "purple_eyes",
    "yellow_eyes", "amber_eyes", "grey_eyes", "heterochromia",
    "blush", "smile", "open_mouth", "closed_eyes", "looking_at_viewer", "looking_away",
    "simple_background", "white_background", "transparent_background",
    "school_uniform", "sailor_uniform", "skirt", "dress", "thighhighs", "kneehighs",
    "gloves", "hair_ornament", "hair_ribbon", "hair_flower", "ribbon", "bow",
    "official_art", "fanart", "original", "rating", "score", "tags", "pixiv",
    "japanese_clothes", "kimono", "maid", "pantyhose", "female", "male", "ecchi",
    "uniform", "twin_tails", "twintails", "ponytail", "braid", "side_ponytail",
    "ahoge", "animal_ears", "cat_ears", "dog_ears", "fox_ears", "bunny_ears",
    "barefoot", "cleavage", "swimsuit", "bikini", "monochrome", "comic", "parody",
    "translated", "western", "korean", "crying", "tears", "food", "drink", "weapon",
    "sword", "gun", "wings", "halo", "horns", "tail", "jewelry", "necklace", "earrings",
    "glasses", "sunglasses", "hat", "cap", "beret", "hood", "jacket", "coat", "sweater",
    "cardigan", "shirt", "collarbone", "navel", "bare_shoulders", "sleeveless",
    "no_character", "no_people", "flower", "water", "moon", "magic", "night", "sky",
    "clouds", "stars", "tree", "plant", "scenery", "instrument", "violin", "piano",
    "img", "image", "preview", "thumbnail", "photo", "picture", "safe", "questionable", "explicit",
    "signed", "single", "reflection", "feathers", "traditional_clothes", "full_moon",
    "alcd", "girl", "boy", "wink", "drink", "petals", "aliasing", "anthropomorphism",
    "headband", "headphones", "zettai_ryouiki", "hairpin", "hair_ribbon",
}

# Tag yang merupakan nama seri/franchise, bukan nama karakter
KNOWN_SERIES_TAGS = {
    "touhou", "kantai_collection", "kancolle",
    "re:zero_kara_hajimeru_isekai_seikatsu", "re:zero",
    "sword_art_online", "fate/grand_order", "fate/stay_night",
    "genshin_impact", "honkai_impact", "league_of_legends",
    "azur_lane", "blue_archive", "arknights",
    "danganronpa", "idolmaster", "love_live",
    "tokyo_kushu", "tokyo_ghoul",
    "vocaloid", "hatsune_miku",
    "houkai_gakuen_2", "honkai_star_rail",
    "atelier_live", "immortal_journey",
    "riot_games",
}

# Mapping alias seri yang pendek → nama lengkap
SERIES_ALIASES = {
    "touhou": "Touhou Project",
    "kancolle": "Kantai Collection",
    "kantai_collection": "Kantai Collection",
    "re:zero_kara_hajimeru_isekai_seikatsu": "Re:Zero",
    "re:zero": "Re:Zero",
    "idolmaster": "The iDOLM@STER",
    "genshin_impact": "Genshin Impact",
    "honkai_impact": "Honkai Impact 3rd",
    "houkai_gakuen_2": "Honkai Impact 3rd",
    "honkai_star_rail": "Honkai: Star Rail",
    "azur_lane": "Azur Lane",
    "blue_archive": "Blue Archive",
    "arknights": "Arknights",
    "league_of_legends": "League of Legends",
    "vocaloid": "Vocaloid",
    "tokyo_kushu": "Tokyo Ghoul",
    "tokyo_ghoul": "Tokyo Ghoul",
    "sword_art_online": "Sword Art Online",
    "danganronpa": "Danganronpa",
    "love_live": "Love Live!",
    "fate/grand_order": "Fate/Grand Order",
    "fate/stay_night": "Fate/Stay Night",
    "atelier_live": "Atelier Live",
    "riot_games": "League of Legends",
}

# Blacklist: tag yang mengandung kata-kata ini sebagai SELURUH tag adalah non-karakter
ARTIST_BLACKLIST = {"pixiv", "circle", "artist", "doujin", "cosplay", "vtuber"}


def _clean_text(s: str) -> str:
    s = s.replace("_", " ")
    return " ".join([w.capitalize() for w in s.split() if w])


def _extract_character_from_tags(raw_tags_str: str) -> Tuple[Optional[str], Optional[str]]:
    """
    Menganalisis string tag dari IQDB (alt text gambar)
    dan mengekstrak (nama_karakter, nama_seri).

    Mendukung format:
    - Booru Konachan/yande.re: "Rating: s Score: X Tags: char_name_(series) generic_tag..."
    - Anime-Pictures: "Tags: alcd,barefoot,char_name,series_name,..."
    - Zerochan: "Rating: s Tags: Female, CharName, SeriesName, ..."
    """
    if not raw_tags_str or raw_tags_str.strip() in {"[IMG]", "[IMAGE]", "IMG", "icon"}:
        return None, None

    clean = raw_tags_str.strip()
    # Hapus prefix metadata
    clean = re.sub(r"Rating:\s*[a-z]\s*", "", clean, flags=re.I)
    clean = re.sub(r"Score:\s*\d+\s*", "", clean, flags=re.I)
    clean = re.sub(r"^\s*Tags:\s*", "", clean, flags=re.I)

    # Pisahkan per tag (koma untuk Anime-Pictures/Zerochan, spasi untuk Konachan/yande.re)
    if "," in clean:
        tags = [t.strip() for t in clean.split(",") if t.strip()]
    else:
        tags = [t.strip() for t in clean.split() if t.strip()]

    char_candidates: List[Tuple[str, Optional[str], int]] = []  # (char, series, priority)
    detected_series: Optional[str] = None

    for t in tags:
        t = re.sub(r"^\s*Tags:\s*", "", t.strip(), flags=re.I)
        if t.startswith("[") and t.endswith("]"):
            continue

        norm = t.lower().replace(" ", "_").strip("[]().")

        # Lewati jika terlalu pendek atau generic
        if len(norm) <= 2 or norm in GENERIC_TAGS:
            continue

        # --- Cek apakah ini nama seri standalone ---
        if norm in KNOWN_SERIES_TAGS:
            detected_series = SERIES_ALIASES.get(norm, _clean_text(t))
            continue

        # --- Format paling akurat: char_name_(series) ---
        # Contoh: ram_(re:zero), houraisan_kaguya_(touhou)
        m = re.match(r"^(.+?)\s*\(([^)]+)\)$", t)
        if m:
            char_part = m.group(1).strip()
            series_part = m.group(2).strip()
            char_norm = char_part.lower().replace(" ", "_")
            series_norm = series_part.lower().replace(" ", "_")

            # Jika series_part adalah blacklisted artist marker
            if series_norm in ARTIST_BLACKLIST:
                continue

            if char_norm not in GENERIC_TAGS:
                series_display = SERIES_ALIASES.get(series_norm, _clean_text(series_part))
                char_candidates.append((_clean_text(char_part), series_display, 10))
            continue

        # --- Tag 2+ kata yang bukan generic (kemungkinan nama karakter) ---
        parts = t.replace("_", " ").split()
        if len(parts) >= 2:
            if not any(p.lower().replace(" ", "_") in GENERIC_TAGS for p in parts):
                char_candidates.append((_clean_text(t), None, 5))
            continue

        # --- Kata tunggal spesifik (kurang prioritas) ---
        if norm not in GENERIC_TAGS and len(norm) > 3:
            char_candidates.append((_clean_text(t), None, 1))

    if not char_candidates:
        return None, detected_series

    # Urutkan: prioritas tertinggi dulu
    char_candidates.sort(key=lambda x: x[2], reverse=True)
    best_char, best_series, _ = char_candidates[0]

    # Gunakan detected_series sebagai fallback jika karakter tidak punya info seri
    if not best_series and detected_series:
        best_series = detected_series

    return best_char, best_series


class IQDBRecognizer(BaseRecognizer):
    """
    Mencari identitas karakter anime melalui mesin pencari gambar IQDB (https://iqdb.org).
    IQDB memindai 6+ basis data anime besar (Danbooru, Gelbooru, Konachan, yande.re, Anime-Pictures, e-shuushuu).
    100% Gratis dan TANPA API KEY apapun.
    """

    def __init__(self, min_similarity: float = 40.0):
        self.endpoint = "https://iqdb.org/"
        self.min_similarity = min_similarity

    async def identify(self, image_bytes: bytes) -> Optional[CharacterInfo]:
        headers = {
            "User-Agent": (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
                "AppleWebKit/537.36 (KHTML, like Gecko) "
                "Chrome/120.0.0.0 Safari/537.36"
            ),
        }

        # Deteksi mime type
        mime_type = "image/jpeg"
        if image_bytes.startswith(b"\x89PNG"):
            mime_type = "image/png"
        elif image_bytes.startswith(b"RIFF") and b"WEBP" in image_bytes[:16]:
            mime_type = "image/webp"

        data = aiohttp.FormData()
        data.add_field(
            "file",
            image_bytes,
            filename="waifu.jpg",
            content_type=mime_type,
        )

        try:
            logger.info("Mengirim gambar ke mesin pencari IQDB.org...")
            async with aiohttp.ClientSession() as session:
                async with session.post(
                    self.endpoint,
                    data=data,
                    headers=headers,
                    timeout=aiohttp.ClientTimeout(total=15),
                ) as resp:
                    if resp.status != 200:
                        logger.error(f"IQDB mengembalikan status HTTP {resp.status}")
                        return None

                    html = await resp.text()

            # Ekstraksi tabel-tabel hasil pencocokan dari HTML
            tables = re.findall(r"<table[^>]*>.*?</table>", html, re.DOTALL | re.IGNORECASE)
            if not tables:
                logger.warning("IQDB tidak mengembalikan tabel hasil.")
                return None

            best_char = None
            best_series = None
            best_sim = 0.0

            for table in tables:
                # Cek apakah tabel mengandung "No relevant matches"
                if "no relevant matches" in table.lower():
                    continue

                # Periksa apakah ada persentase kemiripan
                sim_m = re.search(r"(\d+)%\s*similarity", table, re.IGNORECASE)
                is_best = "best match" in table.lower()
                similarity = float(sim_m.group(1)) if sim_m else (95.0 if is_best else 0.0)

                if similarity < self.min_similarity:
                    continue

                # Ambil SEMUA alt text dari tabel, cari yang paling informatif
                all_alts = re.findall(r'alt=["\'](.*?)["\']', table)

                char_found = None
                series_found = None

                for alt_text in all_alts:
                    char_name, series_name = _extract_character_from_tags(alt_text)
                    if char_name:
                        char_found = char_name
                        series_found = series_name
                        break  # Ambil yang pertama berhasil diekstrak

                if char_found and similarity > best_sim:
                    best_sim = similarity
                    best_char = char_found
                    best_series = series_found

            if not best_char:
                logger.warning("IQDB menemukan gambar tetapi tidak menemukan tag nama karakter yang cocok.")
                return None

            # Jika seri belum diketahui, coba cari via AniList
            if not best_series:
                logger.info(f"Seri tidak ditemukan di IQDB, mencoba AniList untuk '{best_char}'...")
                anilist_name, anilist_series = await lookup_series_from_character(best_char)
                if anilist_series:
                    best_series = anilist_series
                if anilist_name and anilist_name != best_char:
                    logger.info(f"AniList mengoreksi nama: '{best_char}' → '{anilist_name}'")
                    best_char = anilist_name

            # Parse first/last name
            # Nama Jepang dari booru biasanya format: Lastname Firstname
            # Contoh: "Houraisan Kaguya" → first=Kaguya, last=Houraisan
            parts = best_char.split()
            if len(parts) >= 2:
                first_name = parts[-1]   # Kata terakhir = given name
                last_name = " ".join(parts[:-1])  # Sisanya = family name
            else:
                first_name = best_char
                last_name = None

            logger.info(
                f"IQDB Sukses Menemukan: {best_char} "
                f"(Seri: {best_series or 'Unknown'}, Kemiripan: {best_sim}%)"
            )

            return CharacterInfo(
                full_name=best_char,
                first_name=first_name,
                last_name=last_name,
                series=best_series,
                confidence=best_sim / 100.0,
                source="iqdb_search",
            )

        except Exception as e:
            logger.error(f"Error saat mencari di IQDB: {e}")
            return None
