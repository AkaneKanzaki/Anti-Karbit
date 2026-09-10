import asyncio
import aiohttp

async def test_jikan():
    # Test Frieren on Jikan (MyAnimeList ID: 52991)
    url = "https://api.jikan.moe/v4/anime/52991/characters"
    headers = {"User-Agent": "Mozilla/5.0"}
    async with aiohttp.ClientSession() as session:
        async with session.get(url, headers=headers) as resp:
            print("Jikan status:", resp.status)
            if resp.status == 200:
                data = await resp.json()
                char_list = data.get("data", [])
                print(f"Jikan returned {len(char_list)} characters!")
                for c in char_list[:3]:
                    ch = c.get("character", {})
                    role = c.get("role")
                    print(f" - {ch.get('name')} (Role: {role})")
            else:
                print(await resp.text()[:200])

asyncio.run(test_jikan())
