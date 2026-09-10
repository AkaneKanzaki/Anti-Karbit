import asyncio
import aiohttp
import re
import json

async def test_lens_aiohttp():
    with open("test_waifu.png", "rb") as f:
        img_bytes = f.read()

    headers = {
        "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:123.0) Gecko/20100101 Firefox/123.0"
    }

    url = "https://lens.google.com/upload"
    params = {"hl": "en", "gl": "us"}

    data = aiohttp.FormData()
    data.add_field("encoded_image", img_bytes, filename="waifu.png", content_type="image/png")
    data.add_field("image_content", "")

    async with aiohttp.ClientSession(headers=headers) as session:
        print("Sending upload to Google Lens...")
        async with session.post(url, data=data, params=params, allow_redirects=False) as resp:
            print("Upload status:", resp.status)
            print("Headers:", resp.headers)
            location = resp.headers.get("Location")
            print("Location:", location)

        if location:
            print("\nFetching redirect location...")
            async with session.get(location, headers=headers) as resp2:
                print("Resp2 status:", resp2.status)
                text = await resp2.text()
                print("HTML length:", len(text))
                with open("lens_redirect.html", "w", encoding="utf-8") as f_out:
                    f_out.write(text)

                # Check if Houraisan or Kaguya is in text
                print("Kaguya in html:", "kaguya" in text.lower())
                print("Houraisan in html:", "houraisan" in text.lower())

                # Find all AF_initDataCallback
                af_blocks = re.findall(r'AF_initDataCallback\((.*?)\);', text, re.DOTALL)
                print(f"Total AF_initDataCallback blocks: {len(af_blocks)}")
                for idx, block in enumerate(af_blocks):
                    if "ds:0" in block or "Houraisan" in block or "Kaguya" in block:
                        print(f"Block {idx} matches!")
                        print(block[:300])

if __name__ == "__main__":
    asyncio.run(test_lens_aiohttp())
