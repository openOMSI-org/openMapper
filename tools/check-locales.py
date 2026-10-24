#!/usr/bin/env python3
"""Check intentionally partial Portuguese locale catalogues against Rust source strings."""
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PAIR = re.compile(r'\b(?:tr\(\s*|cmd!\(\s*Icon::\w+\s*,\s*)("(?:[^"\\]|\\.)*")\s*,\s*("(?:[^"\\]|\\.)*")', re.S)
source = set()
for path in (ROOT / "src").rglob("*.rs"):
    for match in PAIR.finditer(path.read_text(encoding="utf-8")):
        source.add(json.loads(match[1]))
catalogues = {}
for language in ("pt-BR", "pt-PT"):
    entries = json.loads((ROOT / "locales" / (language + ".json")).read_text(encoding="utf-8"))
    assert isinstance(entries, dict)
    obsolete = entries.keys() - source
    assert not obsolete, (language, "obsolete source keys", sorted(obsolete))
    for key, value in entries.items():
        assert isinstance(value, str) and value.strip(), (language, "empty", key)
        assert "▁" not in value, (language, "tokenisation marker", key)
        for token in re.findall(r'\{[^{}]*\}|\bOPENMAPPER_[A-Z_]+\b|https?://\S+', key):
            assert token in value, (language, "changed protected literal", token, key)
    catalogues[language] = entries
    print(f"{language}: {len(entries)} corrected-subset entries, {len(source) - len(entries)} English fallbacks")
assert catalogues["pt-BR"].keys() == catalogues["pt-PT"].keys()
for key in (
    "Save", "File", "View", "Help", "Language", "Roads", "Objects", "Tiles",
    # Keep the complete Map menu translated in both Portuguese variants.
    "Calendar and place…",
    "Signal routes…",
    "Chrono scenarios…",
    "Ground textures…",
    "Check map…",
    "Make the junctions again",
    "Lines and timetable…",
    "Traffic and AI…",
    "Depot files (destinations)…",
    "Satellite and terrain…",
    "From OpenStreetMap…",
    "GPS tracks to trace…",
    "Tiles from another map…",
    "Test drive in openOMSI",
    "Test drive from the start point nearest the view",
    "See this view in openOMSI",
    "Test drive in OMSI 2 (the original)…",
    "Check in openOMSI…",
):
    assert key in source, ("core label no longer in source", key)
    for language, entries in catalogues.items():
        assert key in entries, (language, "missing core label", key)
assert catalogues["pt-BR"]["Save"] == "Salvar"
assert catalogues["pt-PT"]["Save"] == "Guardar"
print("Portuguese catalogue checks: OK")
