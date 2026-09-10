import re
import logging
from typing import Optional, List, Tuple
import aiohttp  # type: ignore

from .base import BaseRecognizer, CharacterInfo
from .anilist_lookup import lookup_series_from_character

logger = logging.getLogger("antikarbit.iqdb")

# Tag umum booru untuk pose, atribut fisik, pakaian, background, dll.
GENERIC_TAGS = {
    "1girl", "2girls", "3girls", "4girls", "5girls", "6+girls", "multiple_girls",
    "1boy", "2boys", "3boys", "multiple_boys", "solo", "highres", "absurdres", "incredibly_absurdres",
    "long_hair", "short_hair", "medium_hair", "very_long_hair",
    "black_hair", "brown_hair", "blonde_hair", "blue_hair", "pink_hair", "purple_hair",
    "red_hair", "white_hair", "green_hair", "silver_hair", "grey_hair", "orange_hair",
    "two-tone_hair", "streaked_hair", "multicolored_hair", "gradient_hair",
    "blue_eyes", "red_eyes", "brown_eyes", "green_eyes", "pink_eyes", "purple_eyes",
    "yellow_eyes", "amber_eyes", "grey_eyes", "heterochromia",
    "blush", "smile", "open_mouth", "closed_eyes", "looking_at_viewer", "looking_away",
    "simple_background", "white_background", "transparent_background", "grey_background",
    "black_background",
    "school_uniform", "sailor_uniform", "sailor_suit", "skirt", "dress", "thighhighs", "kneehighs",
    "gloves", "hair_ornament", "hair_ribbon", "hair_flower", "ribbon", "bow",
    "official_art", "fanart", "original", "rating", "score", "tags", "pixiv", "danbooru",
    "gelbooru", "konachan", "yande.re", "anime-pictures", "zerochan",
    "japanese_clothes", "kimono", "yukata", "maid", "pantyhose", "female", "male", "ecchi",
    "uniform", "twin_tails", "twintails", "ponytail", "braid", "side_ponytail", "french_braid",
    "ahoge", "animal_ears", "cat_ears", "dog_ears", "fox_ears", "bunny_ears", "wolf_ears",
    "barefoot", "cleavage", "swimsuit", "bikini", "monochrome", "comic", "parody",
    "translated", "western", "korean", "crying", "tears", "food", "drink", "weapon",
    "sword", "gun", "wings", "halo", "horns", "tail", "jewelry", "necklace", "earrings",
    "glasses", "sunglasses", "hat", "cap", "beret", "hood", "jacket", "coat", "sweater",
    "cardigan", "shirt", "collarbone", "navel", "bare_shoulders", "sleeveless",
    "no_character", "no_people", "flower", "water", "moon", "magic", "night", "sky",
    "clouds", "stars", "tree", "plant", "scenery", "instrument", "violin", "piano",
    "img", "image", "preview", "thumbnail", "photo", "picture", "safe", "questionable", "explicit",
    "signed", "single", "reflection", "feathers", "traditional_clothes", "full_moon",
    "alcd", "girl", "boy", "wink", "petals", "aliasing", "anthropomorphism",
    "headband", "headphones", "zettai_ryouiki", "hairpin",
    "profile", "upper_body", "lower_body", "full_body", "portrait", "close-up", "cowboy_shot",
    "sitting", "standing", "lying", "kneeling", "leaning_forward",
    "holding", "arms_behind_back", "arms_up", "hand_on_hip", "peace_sign", "v",
    "cleavage_cutout", "bare_arms", "bare_legs", "midriff", "navel", "sideboob", "underboob",
}

# Tag tema, kostum, tropi, atau meta yang sering memiliki 2+ kata dan BUKAN nama karakter
THEMES_AND_COSTUMES = {
    "maid_bikini", "maid_uniform", "bunny_girl", "bunny_suit", "reverse_bunny_suit",
    "school_swimsuit", "competition_swimsuit", "one-piece_swimsuit", "micro_bikini",
    "gothic_lolita", "sweet_lolita", "lolita_fashion", "cheerleader", "racing_queen",
    "miko", "nurse", "office_lady", "police", "soldier", "military_uniform",
    "wedding_dress", "bridal_gauntlets", "santa_costume", "christmas", "halloween_costume",
    "halloween", "cyberpunk", "steampunk", "mecha_musume", "monster_girl",
    "magical_girl", "armor", "power_armor", "tracksuit", "gym_uniform", "bloomers",
    "hoodie", "oversized_clothes", "catgirl", "foxgirl", "wolfgirl", "cowgirl",
    "original_character", "fan_character", "chibi", "alternate_costume",
    "alternate_hairstyle", "costume_switch", "clothes_lift", "skirt_lift",
    "pantyshot", "upskirt", "underbust", "crossover", "gender_bend", "gender_swap",
    "bad_id", "bad_link", "bad_pixiv_id", "copyright_request", "artist_request",
    "virtual_youtuber", "vtuber", "indie_vtuber", "envtuber", "jpvtuber", "idvtuber",
    "voicevox", "vocaloid", "utaite",
}

# Tag grup VTuber, agensi, franchise, studio, atau nama seri
KNOWN_SERIES_TAGS = {
    # VTuber Agencies & Groups
    "hololive", "hololive_production", "hololive_english", "hololive_indonesia",
    "hololive_gamers", "hololive_fantasy", "hololive_dev_is", "holox",
    "hololive_0th_gen", "hololive_1st_gen", "hololive_2nd_gen", "hololive_3rd_gen",
    "hololive_4th_gen", "hololive_5th_gen", "holostars", "holostars_english",
    "nijisanji", "nijisanji_en", "nijisanji_id", "nijisanji_kr", "virtuareal",
    "vshojo", "vspo", "vspo!", "774inc", "brave_group", "phase_connect",
    "neo-porte", "idol_corporation", "idol_corp", "noripro", "kawaii_production",
    "production_kawaii", "wactor", "aogiri_high_school", "upd8",

    # Populer Game / Anime Franchises
    "genshin_impact", "honkai_impact", "honkai_impact_3rd", "houkai_gakuen_2",
    "honkai_star_rail", "zenless_zone_zero", "wuthering_waves",
    "blue_archive", "arknights", "azur_lane", "girls_frontline", "girls_frontline_2",
    "nikke", "goddess_of_victory:_nikke", "nikke:_goddess_of_victory",
    "fate/grand_order", "fate_grand_order", "fate/stay_night", "fate_stay_night",
    "fate/extra", "fate/apocrypha", "fate_series", "type-moon", "typemoon", "tsukihime",
    "touhou", "touhou_project", "kantai_collection", "kancolle",
    "the_idolm@ster", "idolmaster", "idolmaster_cinderella_girls", "idolmaster_million_live",
    "idolmaster_shiny_colors", "idolmaster_side_m", "gakuen_idolmaster",
    "love_live!", "love_live", "love_live!_sunshine!!", "love_live!_nijigasaki_high_school_idol_club",
    "love_live!_superstar!!", "love_live!_hasunosora_jogakuin_school_idol_club",
    "bang_dream!", "bang_dream", "bandori", "project_sekai", "project_sekai_colorful_stage!",
    "d4dj", "uma_musume", "uma_musume_pretty_derby", "granblue_fantasy",
    "princess_connect!", "princess_connect!_re:dive", "alchemy_stars",
    "re:zero_kara_hajimeru_isekai_seikatsu", "re:zero", "sword_art_online",
    "danganronpa", "tokyo_kushu", "tokyo_ghoul", "atelier_series", "atelier_live",
    "league_of_legends", "riot_games", "valorant", "overwatch", "pokemon",
    "fire_emblem", "honkai", "atelier", "dungeon_meshi", "frieren",
    "sousou_no_frieren", "bocchi_the_rock!", "bocchi_the_rock", "oshi_no_ko",
    "chainsaw_man", "jujutsu_kaisen", "spy_x_family", "lycoris_recoil",
    "neon_genesis_evangelion", "evangelion", "dragon_ball", "naruto",
    "one_piece", "bleach", "fairy_tail", "sword_art_online", "attack_on_titan",
    "shingeki_no_kyojin", "fullmetal_alchemist", "hunter_x_hunter", "demon_slayer",
    "kimetsu_no_yaiba", "my_hero_academia", "boku_no_hero_academia",
}

# Kata kunci tunggal yang mengindikasikan seri/grup, bukan nama karakter
# Digunakan untuk partial match dalam tag 2+ kata
KNOWN_SERIES_WORDS = {
    "hololive", "nijisanji", "holostars", "vshojo", "vspo", "noripro",
    "genshin", "honkai", "arknights", "kancolle", "touhou",
    "idolmaster", "lovelive", "bandori", "sekai", "granblue",
    "pokemon", "vtuber", "nijisanji", "phase", "connect",
}

# Mapping alias seri/grup yang pendek → nama lengkap yang rapi
SERIES_ALIASES = {
    # VTuber
    "hololive": "Hololive",
    "hololive_production": "Hololive Production",
    "hololive_english": "Hololive English",
    "hololive_indonesia": "Hololive Indonesia",
    "hololive_gamers": "Hololive Gamers",
    "hololive_fantasy": "Hololive Fantasy",
    "hololive_dev_is": "Hololive DEV_IS",
    "holox": "holoX",
    "holostars": "HOLOSTARS",
    "nijisanji": "Nijisanji",
    "nijisanji_en": "Nijisanji EN",
    "vshojo": "VShojo",
    "vspo": "VSPO!",
    "vspo!": "VSPO!",
    "phase_connect": "Phase Connect",
    "neo-porte": "Neo-Porte",
    "idol_corporation": "Idol Corp",
    "idol_corp": "Idol Corp",
    "noripro": "NoriPro",

    # Anime & Game
    "touhou": "Touhou Project",
    "touhou_project": "Touhou Project",
    "kancolle": "Kantai Collection",
    "kantai_collection": "Kantai Collection",
    "re:zero_kara_hajimeru_isekai_seikatsu": "Re:Zero",
    "re:zero": "Re:Zero",
    "idolmaster": "The iDOLM@STER",
    "the_idolm@ster": "The iDOLM@STER",
    "idolmaster_cinderella_girls": "The iDOLM@STER Cinderella Girls",
    "idolmaster_shiny_colors": "The iDOLM@STER Shiny Colors",
    "gakuen_idolmaster": "Gakuen Idolmaster",
    "genshin_impact": "Genshin Impact",
    "honkai_impact": "Honkai Impact 3rd",
    "honkai_impact_3rd": "Honkai Impact 3rd",
    "houkai_gakuen_2": "Honkai Impact 3rd",
    "honkai_star_rail": "Honkai: Star Rail",
    "zenless_zone_zero": "Zenless Zone Zero",
    "wuthering_waves": "Wuthering Waves",
    "azur_lane": "Azur Lane",
    "blue_archive": "Blue Archive",
    "arknights": "Arknights",
    "nikke": "Goddess of Victory: Nikke",
    "nikke:_goddess_of_victory": "Goddess of Victory: Nikke",
    "goddess_of_victory:_nikke": "Goddess of Victory: Nikke",
    "league_of_legends": "League of Legends",
    "riot_games": "League of Legends",
    "tokyo_kushu": "Tokyo Ghoul",
    "tokyo_ghoul": "Tokyo Ghoul",
    "sword_art_online": "Sword Art Online",
    "danganronpa": "Danganronpa",
    "love_live!": "Love Live!",
    "love_live": "Love Live!",
    "fate/grand_order": "Fate/Grand Order",
    "fate_grand_order": "Fate/Grand Order",
    "fate/stay_night": "Fate/Stay Night",
    "fate_stay_night": "Fate/Stay Night",
    "fate_series": "Fate Series",
    "type-moon": "TYPE-MOON",
    "typemoon": "TYPE-MOON",
    "bang_dream!": "BanG Dream!",
    "bang_dream": "BanG Dream!",
    "bandori": "BanG Dream!",
    "project_sekai": "Project SEKAI",
    "project_sekai_colorful_stage!": "Project SEKAI",
    "uma_musume": "Uma Musume",
    "uma_musume_pretty_derby": "Uma Musume Pretty Derby",
    "granblue_fantasy": "Granblue Fantasy",
    "princess_connect!": "Princess Connect!",
    "princess_connect!_re:dive": "Princess Connect! Re:Dive",
    "sousou_no_frieren": "Sousou no Frieren",
    "frieren": "Sousou no Frieren",
    "bocchi_the_rock!": "Bocchi the Rock!",
    "bocchi_the_rock": "Bocchi the Rock!",
    "oshi_no_ko": "Oshi no Ko",
    "lycoris_recoil": "Lycoris Recoil",
    "dungeon_meshi": "Dungeon Meshi",
}

# Blacklist artis booru / kata kunci non-karakter
ARTIST_BLACKLIST = {
    "pixiv", "circle", "artist", "doujin", "cosplay", "vtuber", "twitter",
    "fanart", "official_art", "cover", "album", "sketch",
    # Populer booru artists
    "kantoku", "hiten", "morikura_en", "fukahire", "mika_pikazo", "anmi",
    "tony_taka", "tiv", "redrop", "wada_arco", "takeuchi_takashi",
    "asanagi", "shindol", "canno", "namie", "lack", "ryota-h", "kuroboshi_kouhaku",
}


def _clean_text(s: str) -> str:
    s = s.replace("_", " ")
    return " ".join([w.capitalize() for w in s.split() if w])


def _is_series_tag(norm: str) -> bool:
    """
    Periksa apakah tag (normalized) adalah seri/grup bukan nama karakter.
    Cek: exact match di KNOWN_SERIES_TAGS, atau salah satu kata ada di KNOWN_SERIES_WORDS.
    """
    if norm in KNOWN_SERIES_TAGS:
        return True
    # Partial word match — jika salah satu kata dalam tag ada di kata-kata series yang dikenal
    words = norm.replace("-", "_").split("_")
    for word in words:
        if word in KNOWN_SERIES_WORDS and len(word) > 4:
            return True
    return False


def _parse_parentheticals(tag: str) -> Tuple[Optional[str], Optional[str]]:
    """
    Parse tag dengan format: character_name_(qualifier)_(series) atau character_name_(series).
    Contoh:
      - "houshou_marine_(hololive)" → ("houshou_marine", "hololive")
      - "inugami_korone_(dog)_(hololive)" → ("inugami_korone", "hololive") [ambil paling kanan yang adalah seri]
      - "ram_(re:zero)" → ("ram", "re:zero")
      - "rem_(re:zero)" → ("rem", "re:zero")
    """
    # Temukan semua pasangan parenthetical
    parentheticals = re.findall(r'\(([^)]+)\)', tag)
    if not parentheticals:
        return None, None

    # Hapus semua parenthetical dari tag untuk dapat nama dasarnya
    char_part = re.sub(r'\s*\([^)]*\)', '', tag).strip()
    char_norm = char_part.lower().replace(" ", "_")

    if not char_part or char_norm in GENERIC_TAGS or char_norm in THEMES_AND_COSTUMES:
        return None, None

    # Cari parenthetical yang merupakan seri (bukan qualifier fisik seperti "dog", "cat", "blue")
    # Prioritas: parenthetical yang ada di KNOWN_SERIES_TAGS atau SERIES_ALIASES
    series_part = None
    for paren in reversed(parentheticals):  # Dari kanan ke kiri
        paren_norm = paren.lower().replace(" ", "_")
        if paren_norm in KNOWN_SERIES_TAGS or paren_norm in SERIES_ALIASES:
            series_part = paren
            break
        # Jika parenthetical tidak dikenali sebagai qualifier fisik (1 kata pendek), anggap seri
        if len(paren.split()) <= 3 and paren_norm not in ARTIST_BLACKLIST and paren_norm not in GENERIC_TAGS:
            series_part = paren

    return char_part, series_part


def _extract_character_from_tags(raw_tags_str: str) -> Tuple[Optional[str], Optional[str]]:
    """
    Menganalisis string tag dari IQDB (alt text gambar)
    dan mengekstrak (nama_karakter, nama_seri).

    Sangat cerdas membedakan:
    - Nama karakter (misal: "inugami_korone", "houshou_marine_(hololive)", "houraisan_kaguya")
    - Nama grup/agensi (misal: "hololive_production", "nijisanji_en") → masuk ke series!
    - Nama tema/pakaian (misal: "maid_bikini", "school_uniform") → diabaikan!
    - Nama artis (misal: "kantoku", "morikura_en") → diabaikan!
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

        # Lewati jika terlalu pendek atau ada di generic tags
        if len(norm) <= 2 or norm in GENERIC_TAGS or norm in THEMES_AND_COSTUMES:
            continue

        # Lewati jika seluruh kata adalah nama artis / blacklist
        if norm in ARTIST_BLACKLIST:
            continue

        # --- Cek apakah ini nama seri / grup VTuber standalone ---
        if _is_series_tag(norm):
            series_cand = SERIES_ALIASES.get(norm, _clean_text(t))
            if not detected_series or len(series_cand) > len(detected_series):
                detected_series = series_cand
            continue

        # --- Format paling akurat: char_name_(series) atau char_name_(qualifier)_(series) ---
        if "(" in t:
            char_part, series_part = _parse_parentheticals(t)
            if char_part:
                char_norm = char_part.lower().replace(" ", "_").strip("_")

                # Pastikan nama karakter bukan seri/tema/generic
                if (
                    char_norm not in GENERIC_TAGS
                    and char_norm not in THEMES_AND_COSTUMES
                    and not _is_series_tag(char_norm)
                    and char_norm not in ARTIST_BLACKLIST
                    and len(char_norm) > 2
                ):
                    if series_part:
                        series_norm = series_part.lower().replace(" ", "_")
                        # Jika series_part adalah artist / cosplay / generic — lewati series saja
                        if series_norm in ARTIST_BLACKLIST or series_norm in THEMES_AND_COSTUMES:
                            series_display = detected_series
                        else:
                            series_display = SERIES_ALIASES.get(series_norm, _clean_text(series_part))
                    else:
                        series_display = detected_series

                    char_candidates.append((_clean_text(char_part), series_display, 10))
            continue

        # --- Tag 2+ kata yang bukan generic/tema/grup (kemungkinan nama karakter) ---
        parts = t.replace("_", " ").split()
        if len(parts) >= 2:
            # Periksa apakah salah satu kata atau gabungan kata merupakan tema/grup/artis
            if any(p.lower().replace(" ", "_") in GENERIC_TAGS for p in parts):
                continue
            if any(p.lower().replace(" ", "_") in ARTIST_BLACKLIST for p in parts):
                continue
            if _is_series_tag(norm) or norm in THEMES_AND_COSTUMES:
                continue
            # Tambahan: cek setiap kata tunggal apakah ada di KNOWN_SERIES_WORDS
            if any(p.lower() in KNOWN_SERIES_WORDS and len(p) > 4 for p in parts):
                # Tag ini mengandung kata seri → catat sebagai seri bukan karakter
                series_cand = SERIES_ALIASES.get(norm, _clean_text(t))
                if not detected_series or len(series_cand) > len(detected_series):
                    detected_series = series_cand
                continue

            # Ini kandidat kuat nama karakter
            char_candidates.append((_clean_text(t), None, 5))
            continue

        # --- Kata tunggal spesifik (kurang prioritas, misal 'Kaguya', 'Marine') ---
        if (
            norm not in GENERIC_TAGS
            and norm not in THEMES_AND_COSTUMES
            and not _is_series_tag(norm)
            and norm not in ARTIST_BLACKLIST
            and len(norm) > 3
        ):
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
    IQDB memindai basis data anime booru (Danbooru, Gelbooru, Konachan, yande.re, Anime-Pictures).
    100% Gratis dan TANPA API KEY apapun.
    """

    def __init__(self, min_similarity: float = 60.0):
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
                # Cek apakah tabel mengandung "No relevant matches" atau "Your image"
                if "no relevant matches" in table.lower() or "your image" in table.lower():
                    continue

                # Periksa persentase kemiripan yang riil
                sim_m = re.search(r"(\d+)%\s*similarity", table, re.IGNORECASE)
                if not sim_m:
                    # Tanpa persentase kemiripan numerik, jangan pernah mengasumsikan 95%!
                    continue

                similarity = float(sim_m.group(1))

                # Abaikan jika di bawah ambang batas yang diset
                if similarity < self.min_similarity:
                    continue

                # Ambil SEMUA alt text dari tabel, cari yang paling informatif
                all_alts = re.findall(r'alt=["\']([^"\']*)["\']', table)

                char_found = None
                series_found = None

                for alt_text in all_alts:
                    char_name, series_name = _extract_character_from_tags(alt_text)
                    if char_name:
                        char_found = char_name
                        series_found = series_name
                        break

                if char_found and similarity > best_sim:
                    best_sim = similarity
                    best_char = char_found
                    best_series = series_found

            if not best_char:
                logger.warning(
                    f"IQDB tidak menemukan kecocokan karakter dengan similarity >= {self.min_similarity}%."
                )
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
