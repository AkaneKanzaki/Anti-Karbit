import sys
import os

# Add scratch/google_lens_repo to sys.path
sys.path.insert(0, os.path.abspath("scratch/google_lens_repo"))

try:
    from googlelens import GoogleLens
    lens = GoogleLens()
    res = lens.search_by_file("test_waifu.png")
    print("Match:", res.get("match"))
    print("Total similar:", len(res.get("similar", [])))
    for s in res.get("similar", [])[:5]:
        print(" - Similar title:", s.get("title"))
except Exception as e:
    import traceback
    traceback.print_exc()
