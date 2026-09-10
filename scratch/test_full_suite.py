import sys
import asyncio

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

sys.path.insert(0, ".")

from recognizer.base import CharacterInfo
from core.claimer import _build_name_candidates, Claimer
from recognizer import get_recognizer
from recognizer.anilist_lookup import lookup_characters_by_media_id

async def run_suite():
    print("=== TEST 1: Anti-Spam Candidates ===")
    char = CharacterInfo(
        full_name="Houraisan Kaguya",
        first_name="Kaguya",
        last_name="Houraisan",
        series="Touhou Project",
        confidence=0.9,
        source="test"
    )
    candidates_full = _build_name_candidates(char, "full")
    print("Full format candidates:", candidates_full)
    assert candidates_full == ["Houraisan Kaguya"], f"Expected ['Houraisan Kaguya'], got {candidates_full}"

    candidates_first = _build_name_candidates(char, "first")
    print("First format candidates:", candidates_first)
    assert candidates_first == ["Kaguya"], f"Expected ['Kaguya'], got {candidates_first}"
    print("✅ TEST 1 PASSED: Anti-spam verified, only single exact name used!")

    print("\n=== TEST 2: Recognizer & Engines Pipeline ===")
    recognizer = get_recognizer(
        iqdb_min_sim=40.0,
        tracemoe_min_sim=0.80,
        saucenao_api_key="sample_key",
        saucenao_min_sim=65.0
    )
    engines = recognizer.get_engines()
    engine_names = [e[0] for e in engines]
    print("Registered engines:", engine_names)
    assert "IQDB" in engine_names
    assert "SauceNAO" in engine_names
    assert "Trace.moe" in engine_names
    print("✅ TEST 2 PASSED: Multi-engine pipeline correctly registered!")

    print("\n=== TEST 3: AniList GraphQL Media Character Lookup ===")
    series, characters = await lookup_characters_by_media_id(154587)
    print(f"Media 154587 series: {series}")
    print(f"Media 154587 characters: {characters}")
    assert isinstance(characters, list), "Expected characters to be a list"
    print("✅ TEST 3 PASSED: AniList GraphQL handler runs cleanly and safely!")

    print("\n🎉 ALL TESTS PASSED SUCCESSFULLY!")

if __name__ == "__main__":
    asyncio.run(run_suite())
