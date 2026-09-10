import asyncio
import aiohttp

async def test_kitsu():
    url = "https://kitsu.io/api/edge/anime?filter[text]=frieren&include=characters"
    headers = {
        "Accept": "application/vnd.api+json",
        "Content-Type": "application/vnd.api+json",
        "User-Agent": "Mozilla/5.0"
    }
    async with aiohttp.ClientSession() as session:
        async with session.get(url, headers=headers) as resp:
            print("Kitsu status:", resp.status)
            if resp.status == 200:
                data = await resp.json()
                print("Kitsu data length:", len(data.get("data", [])))
                included = data.get("included", [])
                print("Kitsu included length:", len(included))

asyncio.run(test_kitsu())
