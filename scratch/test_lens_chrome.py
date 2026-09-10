import asyncio
import aiohttp
import re

async def test_lens_chrome():
    with open("test_waifu.png", "rb") as f:
        img_bytes = f.read()

    headers = {
        "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8",
        "Accept-Language": "en-US,en;q=0.9",
        "sec-ch-ua": '"Google Chrome";v="131", "Chromium";v="131", "Not_A Brand";v="24"',
        "sec-ch-ua-mobile": "?0",
        "sec-ch-ua-platform": '"Windows"',
    }

    url = "https://lens.google.com/upload"
    params = {"hl": "en", "gl": "us"}

    data = aiohttp.FormData()
    data.add_field("encoded_image", img_bytes, filename="waifu.png", content_type="image/png")
    data.add_field("image_content", "")

    async with aiohttp.ClientSession(headers=headers) as session:
        print("Uploading to Lens...")
        async with session.post(url, data=data, params=params, allow_redirects=False) as resp:
            print("Status:", resp.status)
            location = resp.headers.get("Location")

        if location:
            print("Location found, fetching...")
            # Use same session (keeps cookies!)
            fetch_headers = {
                **headers,
                "Sec-Fetch-Dest": "document",
                "Sec-Fetch-Mode": "navigate",
                "Sec-Fetch-Site": "same-site",
                "Sec-Fetch-User": "?1",
                "Upgrade-Insecure-Requests": "1",
                "Referer": "https://lens.google.com/",
            }
            async with session.get(location, headers=fetch_headers) as resp2:
                print("Resp2 status:", resp2.status)
                text = await resp2.text()
                print("Length:", len(text))
                print("Kaguya in text:", "kaguya" in text.lower())
                print("Houraisan in text:", "houraisan" in text.lower())
                print("Touhou in text:", "touhou" in text.lower())
                
                # Check for titles in HTML
                # In google search visual matches, titles are in aria-label, alt, or span
                titles = re.findall(r'<div[^>]*aria-label="([^"]+)"', text)
                print("Aria labels:", titles[:5])

if __name__ == "__main__":
    asyncio.run(test_lens_chrome())
