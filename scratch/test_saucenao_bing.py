import asyncio
import aiohttp
import json
import re

async def test_saucenao():
    print("\n--- Testing SauceNAO ---")
    with open("test_waifu.png", "rb") as f:
        img_bytes = f.read()

    url = "https://saucenao.com/search.php"
    data = aiohttp.FormData()
    data.add_field("file", img_bytes, filename="waifu.png", content_type="image/png")
    data.add_field("output_type", "2")  # JSON output!
    data.add_field("numres", "5")

    headers = {
        "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
    }

    try:
        async with aiohttp.ClientSession() as session:
            async with session.post(url, data=data, headers=headers) as resp:
                print("SauceNAO status:", resp.status)
                if resp.status == 200:
                    res_json = await resp.json()
                    results = res_json.get("results", [])
                    print(f"SauceNAO found {len(results)} results")
                    for r in results[:3]:
                        header = r.get("header", {})
                        data_block = r.get("data", {})
                        print(" Sim:", header.get("similarity"), "% | Title:", data_block.get("title"), "| Characters:", data_block.get("characters") or data_block.get("character") or data_block.get("eng_name") or data_block.get("jp_name"))
                        print(" Raw data keys:", list(data_block.keys()))
                else:
                    print(await resp.text())
    except Exception as e:
        print("SauceNAO error:", e)

async def test_bing_visual():
    print("\n--- Testing Bing Visual Search ---")
    with open("test_waifu.png", "rb") as f:
        img_bytes = f.read()

    # Bing visual search upload endpoint
    url = "https://www.bing.com/images/search?view=detailv2&iss=sbiupload"
    data = aiohttp.FormData()
    data.add_field("imageBin", img_bytes, filename="image.png", content_type="image/png")

    headers = {
        "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        "Referer": "https://www.bing.com/visualsearch",
    }

    try:
        async with aiohttp.ClientSession() as session:
            async with session.post(url, data=data, headers=headers, allow_redirects=True) as resp:
                print("Bing status:", resp.status)
                print("Bing URL:", resp.url)
                text = await resp.text()
                print("Bing length:", len(text))
                # Check for keywords
                print("Kaguya in Bing:", "kaguya" in text.lower())
                print("Houraisan in Bing:", "houraisan" in text.lower())
                # Let's see if there are visual search titles
                matches = re.findall(r'class="[a-zA-Z0-9_\- ]*title[a-zA-Z0-9_\- ]*"[^>]*>(.*?)<', text, re.I)
                print("Bing titles:", matches[:5])
    except Exception as e:
        print("Bing error:", e)

async def main():
    await test_saucenao()
    await test_bing_visual()

if __name__ == "__main__":
    asyncio.run(main())
