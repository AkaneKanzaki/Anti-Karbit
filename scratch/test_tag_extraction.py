import sys
sys.path.insert(0, ".")

from recognizer.iqdb_vision import _extract_character_from_tags

test_cases = [
    (
        "Rating: s Score: 15 Tags: 1girl solo animal_ears hololive_production hololive inugami_korone looking_at_viewer",
        "Inugami Korone",
        "Hololive Production"
    ),
    (
        "Tags: 1girl, solo, virtual_youtuber, houshou_marine_(hololive)",
        "Houshou Marine",
        "Hololive"
    ),
    (
        "Rating: s Tags: nijisanji_en pomu_rainpuff solo maid_uniform",
        "Pomu Rainpuff",
        "Nijisanji EN"
    ),
    (
        "Tags: 1girl, solo, original_character, maid_bikini, kantoku",
        None,
        None
    ),
    (
        "Tags: blue_archive,shiroko_(blue_archive),solo",
        "Shiroko",
        "Blue Archive"
    )
]

print("Running tag extraction tests:")
for raw, expected_char, expected_series in test_cases:
    char, series = _extract_character_from_tags(raw)
    print(f"Input: {raw[:60]}...")
    print(f" -> Result: Character='{char}', Series='{series}'")
    assert char == expected_char, f"Expected char '{expected_char}', got '{char}'"
    if expected_series:
        assert series == expected_series, f"Expected series '{expected_series}', got '{series}'"

print("\nALL TAG EXTRACTION TESTS PASSED! No group/theme tags were misidentified as characters!")
