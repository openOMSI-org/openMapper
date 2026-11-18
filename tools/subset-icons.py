#!/usr/bin/env python3
"""Make assets/fonts/MaterialSymbolsSubset.ttf: the Material Symbols (Rounded) glyphs
openMapper uses, as a small static font (weight 400, outlined, optical size 20).

The full variable font and its codepoint table are kept in assets/fonts (Apache 2.0, from
github.com/google/material-design-icons). To add an icon: put its name in ICONS, run
`python3 tools/subset-icons.py` (needs fonttools), give src/icons.rs a variant with the
name and its code (a test checks the two agree and the glyph is in the subset).
"""
from pathlib import Path
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
from fontTools import subset

ICONS = """
arrow_selector_tool road polyline line_curve call_made deployed_code height home_work
landscape format_paint grid_on sensors open_with rotate_right rotate_left open_in_full
colorize adjust brush ink_eraser arrow_upward arrow_downward waves horizontal_rule
water trending_up elevation roundabout_right content_cut link link_off
vertical_align_bottom u_turn_left add_road flip swap_horiz swap_vert linear_scale
horizontal_distribute shuffle pentagon category call_split merge straighten
auto_fix_high texture image lightbulb map directions_bus directions_car apartment park
alt_route fork_right tram directions_walk fence layers local_parking route gps_fixed
flag signpost volume_up traffic cable satellite_alt public view_in_ar construction
note_add folder_open folder save download upload_file share code build undo redo
content_copy content_paste library_add select_all deselect ad_group ad_group_off
inventory_2 add delete edit search keyboard_command_key center_focus_strong bookmark
photo_camera videocam zoom_in zoom_out fit_screen label settings tune restart_alt
refresh update settings_backup_restore history hourglass schedule calendar_month
light_mode ac_unit contrast format_size translate keyboard memory high_quality group
login logout lock lock_open school info play_arrow stop fact_check verified check
done_all close block warning error check_circle pause step_over step_into step_out
bug_report terminal data_object description extension add_box electric_bolt arrow_back
arrow_forward visibility visibility_off check_box check_box_outline_blank
radio_button_checked radio_button_unchecked chevron_left chevron_right expand_more star
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
