import asyncio
import aiohttp
import re

async def test_yandex():
    with open("test_waifu.png", "rb") as f:
        img_bytes = f.read()

    # Yandex image search
    headers = {
        "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36"
    }

    url = "https://yandex.com/images-apphost/image-download"
    params = {"curent_url": "https://yandex.com/images/search"}
    data = aiohttp.FormData()
    data.add_field("upfile", img_bytes, filename="waifu.png", content_type="image/png")

    try:
        async with aiohttp.ClientSession(headers=headers) as session:
            async with session.post(url, data=data, params=params) as resp:
                print("Yandex upload status:", resp.status)
                if resp.status == 200:
                    res_json = await resp.json()
                    print("Yandex upload json:", res_json)
                    image_id = res_json.get("image_id")
                    if image_id:
                        search_url = f"https://yandex.com/images/search?rpt=imageview&cbir_id={image_id}"
                        print("Search URL:", search_url)
                        async with session.get(search_url) as resp2:
                            print("Search resp status:", resp2.status)
                            html = await resp2.text()
                            print("Length:", len(html))
                            print("Kaguya:", "kaguya" in html.lower())
                            print("Houraisan:", "houraisan" in html.lower())
                            # Check tags / text in HTML
                            tags = re.findall(r'class="[a-zA-Z0-9_\- ]*Tags[a-zA-Z0-9_\- ]*"[^>]*>(.*?)<', html)
                            print("Yandex tags:", tags)
                else:
                    print(await resp.text()[:200])
    except Exception as e:
        print("Yandex error:", e)

if __name__ == "__main__":
    asyncio.run(test_yandex())
