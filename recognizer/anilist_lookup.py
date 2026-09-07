import logging
from typing import Optional, Tuple
import aiohttp  # type: ignore

logger = logging.getLogger("antikarbit.anilist")

ANILIST_GRAPHQL = "https://graphql.anilist.co"

_CHAR_QUERY = """
query ($search: String) {
  Character(search: $search) {
    name {
      full
      native
    }
    media(sort: POPULARITY_DESC, page: 1, perPage: 1) {
      nodes {
        title {
          romaji
          english
        }
        type
      }
    }
  }
}
"""


async def lookup_series_from_character(char_name: str) -> Tuple[Optional[str], Optional[str]]:
    """
    Mencari nama seri (anime/manga) berdasarkan nama karakter di AniList.
    Mengembalikan (full_name_canonical, series_name) atau (None, None) jika tidak ditemukan.
    Gratis & tanpa API Key.
    """
    try:
        async with aiohttp.ClientSession() as session:
            payload = {
                "query": _CHAR_QUERY,
                "variables": {"search": char_name},
            }
            headers = {
                "Content-Type": "application/json",
                "Accept": "application/json",
                "User-Agent": (
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
                    "AppleWebKit/537.36 (KHTML, like Gecko) "
                    "Chrome/120.0.0.0 Safari/537.36"
                ),
            }
            async with session.post(
                ANILIST_GRAPHQL,
                json=payload,
                headers=headers,
                timeout=aiohttp.ClientTimeout(total=8),
            ) as resp:
                if resp.status != 200:
                    logger.debug(f"AniList HTTP {resp.status} untuk karakter '{char_name}'")
                    return None, None

                data = await resp.json()

        char_data = data.get("data", {}).get("Character")
        if not char_data:
            logger.debug(f"AniList tidak menemukan karakter: '{char_name}'")
            return None, None

        name_obj = char_data.get("name", {})
        canonical_name = name_obj.get("full") or char_name

        media_nodes = char_data.get("media", {}).get("nodes", [])
        series_name = None
        if media_nodes:
            title_obj = media_nodes[0].get("title", {})
            series_name = (
                title_obj.get("romaji")
                or title_obj.get("english")
            )

        logger.info(f"AniList: '{char_name}' → karakter='{canonical_name}', seri='{series_name}'")
        return canonical_name, series_name

    except Exception as e:
        logger.debug(f"AniList lookup error untuk '{char_name}': {e}")
        return None, None
