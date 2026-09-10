import asyncio
import aiohttp
import json

ANILIST_GRAPHQL = "https://graphql.anilist.co"

query = """
query ($id: Int) {
  Media(id: $id) {
    title {
      romaji
      english
    }
    characters(sort: [ROLE, RELEVANCE], perPage: 6) {
      nodes {
        name {
          full
          native
        }
      }
    }
  }
}
"""

async def test():
    headers = {
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "Mozilla/5.0",
    }
    async with aiohttp.ClientSession() as session:
        async with session.post(ANILIST_GRAPHQL, json={"query": query, "variables": {"id": 154587}}, headers=headers) as resp:
            print("Status:", resp.status)
            text = await resp.text()
            print("Response:", text)

asyncio.run(test())
