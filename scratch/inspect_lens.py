with open("lens_resp.html", "r", encoding="utf-8") as f:
    html = f.read()

print("HTML snippet (first 1500 chars):")
print(html[:1500])

import re
scripts = re.findall(r'<script[^>]*>(.*?)</script>', html, re.DOTALL)
print(f"\nTotal scripts: {len(scripts)}")
for idx, s in enumerate(scripts):
    if len(s) > 1000:
        print(f"Script {idx}: length {len(s)}")
        # Print first 200 chars of this long script
        print(s[:300])
        print("...")
