import re
import json

with open("lens_resp.html", "r", encoding="utf-8") as f:
    html = f.read()

# Let's search for text, titles, or AF_initDataCallback
print("Checking for Houraisan or Kaguya in html:")
print("Houraisan:", "Houraisan" in html or "houraisan" in html)
print("Kaguya:", "Kaguya" in html or "kaguya" in html)

# Find visual matches titles
titles = re.findall(r'class="[a-zA-Z0-9_\- ]*title[a-zA-Z0-9_\- ]*"[^>]*>(.*?)<', html, re.I)
print("Titles found:", len(titles), titles[:5])

# Find all matches in AF_initDataCallback or JSON blocks
scripts = re.findall(r'AF_initDataCallback\((.*?)\);', html, re.DOTALL)
print("AF callbacks:", len(scripts))

# Let's check regex for strings near Kaguya
matches = re.findall(r'.{0,50}(?:Houraisan|Kaguya).{0,50}', html, re.I)
for m in matches[:10]:
    print("Match:", m.strip().replace("\n", " "))
