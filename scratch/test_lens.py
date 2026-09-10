import asyncio
import aiohttp

async def test_lens():
    with open("test_waifu.png", "rb") as f:
        img_bytes = f.read()

    url = "https://lens.google.com/v3/upload"
    headers = {
        "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
    }
    data = aiohttp.FormData()
    data.add_field("encoded_image", img_bytes, filename="image.png", content_type="image/png")

    async with aiohttp.ClientSession() as session:
        async with session.post(url, data=data, headers=headers, allow_redirects=True) as resp:
            print("Status:", resp.status)
            print("Final URL:", resp.url)
            text = await resp.text()
            print("Length:", len(text))
            with open("lens_resp.html", "w", encoding="utf-8") as f_out:
                f_out.write(text)

if __name__ == "__main__":
    asyncio.run(test_lens())
