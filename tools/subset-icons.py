#!/usr/bin/env python3
"""Make assets/fonts/MaterialSymbolsSubset.ttf: the Material Symbols (Rounded) glyphs
openMapper uses, as a small static font (weight 400, outlined, optical size 20).

The full variable font and its codepoint table are kept in assets/fonts (Apache 2.0, from
github.com/google/material-design-icons). To add an icon: put its name in ICONS, run
`python3 tools/subset-icons.py` (needs fonttools), use the name in src/icons.rs.
"""
from pathlib import Path
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
from fontTools import subset

ICONS = """
arrow_selector_tool road home_work landscape format_paint grid_on
note_add folder_open save undo redo download settings help play_arrow
arrow_upward arrow_downward waves horizontal_rule add delete center_focus_strong
swap_horiz map directions_bus search close check info
apartment park emoji_objects signpost alt_route directions_car train category
tram directions_walk fence layers terrain brush ink_eraser
sensors local_parking route flag volume_up visibility visibility_off traffic
check_box check_box_outline_blank radio_button_checked radio_button_unchecked chevron_right expand_more
star history schedule warning error check_circle content_copy content_paste select_all
open_with rotate_right open_in_full code colorize
polyline line_curve call_made deployed_code height
""".split()

fonts = Path(__file__).resolve().parent.parent / "assets" / "fonts"
codes = {}
for line in (fonts / "MaterialSymbolsRounded.codepoints").read_text().splitlines():
    name, cp = line.split()
    codes[name] = int(cp, 16)
missing = [n for n in ICONS if n not in codes]
if missing:
    raise SystemExit(f"not in Material Symbols: {missing}")
font = TTFont(fonts / "MaterialSymbolsRounded.ttf")
font = instancer.instantiateVariableFont(font, {"wght": 400, "FILL": 0, "GRAD": 0, "opsz": 20})
opts = subset.Options()
opts.layout_features = []
opts.name_IDs = ["*"]
opts.notdef_outline = True
sub = subset.Subsetter(opts)
sub.populate(unicodes=[codes[n] for n in ICONS])
sub.subset(font)
out = fonts / "MaterialSymbolsSubset.ttf"
font.save(out)
print(f"{out}: {len(ICONS)} icons, {out.stat().st_size} bytes")
for n in ICONS:
    print(f"    {n} = {codes[n]:#x}")
