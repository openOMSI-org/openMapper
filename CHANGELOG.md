# Changelog

Releases are tagged `v<version>`; the downloads are on the
[Releases](https://github.com/openOMSI-org/openMapper/releases) page, each with the notes of its
version from this file.

## 0.1.19 - 2026-10-30

### Added
- **A Settings window** (File > Settings…, or the command search): General (language, interface size, camera style, OMSI 2's controls, how many tiles are loaded at once), Graphics, Camera, Keys and Mouse in tabs.
- **Camera speeds and sensitivities**: flying speed, keyboard speed over the map, what Shift multiplies it by, the speed of turning with Q / E, looking round, orbiting, panning, the wheel's step, the up-and-down and wheel inversions, and whether flying builds up speed. They are kept, and one button puts them back.
- **The graphics API can be chosen** (Settings > Graphics, or View > Graphics API): Auto, DirectX 12, Vulkan, Metal or OpenGL, as far as the system has them. A choice that cannot start falls back to Auto; `OPENMAPPER_BACKEND=dx12` sets it for one run. Takes effect after a restart.
- The key list in Settings can be searched by command or by key.
- A spline can be started from the end of a kerb, a wall or a footpath (the ends of splines that no road joins were never offered).

### Fixed
- The complex junction editor draws a junction's plate as the map does: the roadway in its asphalt, the kerbs and footways with their own textures, and no bristling fans at the corners.
- Settings open from the start page too, before a map is chosen.
- **"Buses only" on a path now keeps the cars off it**: it also writes `no_cars` for that path (`bus` alone only opens a path to buses), as OMSI 2's editor does.
- **Invisible streets and AI paths are seen with the editor's switch** (what the game does not show), and the join of two free ends of an AI path.
- The Discord links in the README and the issue template point to the right server.
- Clicking an object made of many materials (a merged lamp, for instance) no longer widens the properties panel with the long texture names, which slid the map view away under the click.

## 0.1.18 - 2026-10-29

URGENT UPDATE: IMPORTANT FIXES

### Added
- **Blender's keys** (View > Preset Key Shortcut (from other applications), or View > Controls): a third set of keys beside openMapper's own and OMSI 2's editor: G moves the selection, Shift+S scales it, X deletes, Tab edits the mesh (its keys were Blender's already), Shift+A adds an object, Shift+D duplicates, F3 searches for a command, Numpad 7 looks straight down. W A S D and Q E move the camera there as they do everywhere else, so Blender's scale is on Shift+S and its select all is left to the menu. Any key can be changed in View > Controls, and a key of your own is kept when the set changes.
- **OMSI 2's editor as one switch, mouse and keys**: View > Preset Key Shortcut (from other applications) > "OMSI 2's editor: mouse and keys" now brings the keys of OMSI 2's own map editor with the mouse: G moves the selection, R turns it, N puts a new object at the cursor and goes into move mode for it, Enter confirms what is being drawn (the road is built on Ctrl+Enter), Del deletes, and the tile grid moves to Shift+G. Every key stays changeable in View > Controls, where the three sets are chosen and one's own keys are kept across them.
- **A press drags only in openMapper's own keys**: in OMSI 2's and Blender's key sets a press on something and a drag of it now only picks it; the move there is G, as it is in those editors (a click no longer carries a whole selection off by accident). The Move / Rotate / Scale arrows, a road's grips and a mesh's vertices are dragged as before, and openMapper's own keys keep dragging the selection as they always did.
- **Blender's grab, in OMSI 2's and Blender's key sets**: G takes hold of the selection at once and it follows the pointer (no handles to find first), X, Y or Z locks the move to that axis (the same key again lets it go free), numbers typed say how far it goes (`.` and `-` as well), a click or Enter lets it go and a right click or Esc puts everything back where it was, with one undo step either way. The bottom bar says the axis and how far it has gone.
- **Faster on big maps, especially while a great many things are selected**: reading the selection no longer costs more the more is in it; a grab reads the links of what it carries once, when it takes hold, instead of walking the whole map for every piece on every frame; every piece is looked up in its tile from a place read once, instead of the tile being walked for each of them; what a grab carries is moved out and back rather than copied every frame; the selected things are put in a fixed order by numbers rather than by a name made for each of them; the AI paths of a selection of thousands are no longer looked up in the assets on every frame (the tint says what is picked), and the handles' size is read for one thing alone, which is the only time it is drawn; and the entry points of the map (`[entry]`) are found from an index instead of the whole map being walked for each of them. A whole 485-tile map with all 117,553 of its things selected and held by G (the worst case there is) now follows the pointer in about 55 ms a frame, where it stood at 43 seconds a frame and the window stopped answering. The log now says "a frame took 0.5 s", and which part of a grab was slow when something does take long; `cargo test --release --bin openmapper what_a_big_selection_costs -- --ignored --nocapture` measures it on a real map.
- **Moving things no longer draws the whole map again**: while things are held, the overview map in the corner is left as it was and made again once they are let go (each frame of a move of something in a 485-tile map drew all of its roads into a picture of its own, 25 ms of a frame); the "Parallel copy" and overhead-wire section of the properties no longer looks up every one of the selected splines in the catalogue on every frame (one look-up, of the road being copied, instead of thousands); the Move / Rotate / Scale handles read their middle from a sample of a selection of thousands rather than from each of them; and the handles no longer ask what is selected twice a frame. A thing, or a few, held by G now draws that whole map in about 12 ms a frame (80 a second), where it took 36 ms.

### Changed
- **A spline, not only a road**: what follows a spline is now attached and let go in the word a map file uses, since a road is only one kind of spline: "Attach to the spline" and "Detach from the spline" (繁中 依附到 Spline／從 Spline 分離, 簡中 依附到样条线／从样条线分离), where the buttons said "the road". Nothing about what they do has changed.

## 0.1.17 - 2026-10-29

URGENT UPDATE: IMPORTANT FIXES

## 0.1.16 - 2026-10-28

### Changed
- **A map's own aerial photos at their own size**: the size a photo is put up at on the graphics card comes from how many are held at once (the window round the camera), not from how many the map has, so a map with hundreds of photos shows the ones near the camera at 1024 or 2048 instead of 512 a side.
- **Faster on big maps**: the overview map's roads are drawn once into a picture instead of being laid out again in every frame, the scene's materials are no longer collected anew in every frame, and which picture a tile shows is looked up on the disk once a second rather than every frame (a map of 66,000 splines went from about 10 to 45 frames a second).

### Fixed
- **Start-up on old GL drivers** ([#48](https://github.com/openOMSI-org/openMapper/issues/48)): a driver without surface view formats (such as a GeForce GTX 550 Ti on driver 391.35) no longer ends start-up with "Downlevel flags SURFACE_VIEW_FORMATS are required"; the frame is made sRGB itself and the interface draws into it.

## 0.1.15 - 2026-10-27

### Added
- **A crossing's signal heads show their light**: the lenses of a signal head are kept (dark) as an object loads, and a crossing's program lights them red, red and yellow, green, yellow as OMSI does; a head that cannot be linked to a crossing shows red.
- **A clock in the view**: View > Time of day offers "let time run" at 1x, 5x and 60x, which speeds up the sun and the signal programs together; the bottom bar shows the hour, and a click on it starts or stops the clock.
- **A map's own aerial photos**: Satellite and terrain has an "OMSI's aerial photos" section that finds the photos OMSI's editor keeps beside the map (`maps/aerials/<map>/tile_X_Y.map.aerial.jpg`), copies them into the map's imagery and records them in `geo.json`; the copying runs in the background, and only the photos of the tiles near the camera can be held on the graphics card.

### Changed
- **Traditional Chinese** uses the terminology of OMSI's own maps and one word for a thing (圖塊, 貼圖, 車廠, 修正).
- Dependencies: Tokio 1.53.3 (patched), zip 9.0.5, sha2 0.11, pollster 1.0.

### Fixed
- **Start-up on old graphics drivers**: when an adapter is found but its device cannot be opened, the next graphics backend (then GL, then the software one) is tried instead of ending start-up.

## 0.1.14 - 2026-10-27

### Changed
- **Auto-Junction in the complex junction editor works as in the Roads tool**: draw the roads as they are, and where they cross, one ends on another's middle, or three or more end at one place, they are cut back by the room the junction needs and a plate is laid with rounded kerbs, footways and lanes through it ("Plate with kerbs and footways", on by default; turns, rounded corners and straight-on pieces are off by default). It is drawn in the editor, saved into custom junctions and objects, and put on the map with the drawing.
- **Bus pocket**: the bay is the road's own type widened along the kerb its traffic keeps to (left on a left-hand road), the other side stays exactly where it was, so a bay no longer pushes the kerb inward on one side; nothing has to be picked. It works on a closed ring road, which before reported a bay as laid while none was; a bay that could not be finished is now rolled back whole.
- **Complex junction editor**: the plate's lanes and kerbs follow the road type's own sides whichever way a road was drawn (before, a road drawn away from the junction had its left/right and lane directions mirrored), and the Lanes view shows the lanes as they will be, the roads cut back and the turns through the plate.

## 0.1.13 - 2026-10-26

### Added
- **Auto-Junction builds the junction** in the complex junction editor: from the arms it lays turns (of each arm's road type) between neighbouring arms, rounded corners of a set radius where the roads' middle lines meet, and a piece straight on between arms that face each other. Each is a switch under Auto-Junction; the built pieces are listed in Layers as "auto", are kept in step with the drawing, never touch pieces drawn by hand, and are put on the map and saved like the others.
- **Bus pocket settings**: the bay's own width (a road type made in the road maker is written again with that pocket lane), the taper into and out of the bay set separately (or kept alike), as a ratio or as a length in metres, and a bay always gets a taper at both ends: near a road's end or a junction it slides along the road to where there is room, and where there is none nothing is changed (no step in the road).
- **Complex junction editor, rebuilt**: Auto-Align (a piece goes on in line with the piece it starts from, tangent arcs in two clicks, lines in steps of 15°), Auto-Adapt (a road of another width keeps the edge on the side it bends to, with a taper), Auto-Junction (arms are found from the roads' free ends), a Curve tool for complex turns (a smooth path through any number of points), a Preview of the junction with the roads running out of its arms and the AI's lanes, zoom (wheel, pinch, buttons, F to fit), undo and redo, drag handles at a piece's ends, numbers for the picked piece, mirror, turn, reverse and copy.

- **Controls as in OMSI 2's editor** ([#44](https://github.com/openOMSI-org/openMapper/issues/44)): a new submenu in the View menu next to Camera, then called "Advanced users", off by default. With it on, the middle button turns the view, the right button held and moved up and down zooms, a right click on the ground goes there (Shift + right click opens the menu), the arrows move the camera and the wheel zooms. The scheme is put together from what players report of the original editor, so tell us what differs.

### Changed
- **Junction plates keep the road's texture format** ([#41](https://github.com/openOMSI-org/openMapper/issues/41)): the asphalt made for a plate is written in the format of the road's own texture (bmp, dds, tga, jpg) as `<name>_junction.<ext>`, with its seasonal pictures made the same way and its .cfg and .surf files copied alongside.
- **Equal junctions share one object** ([#43](https://github.com/openOMSI-org/openMapper/issues/43)): plates that come out the same (shape, lanes, textures) are one object file and one model, named by their contents, instead of a copy for every junction.

### Fixed
- **Start-up hangs leave a trace** ([#40](https://github.com/openOMSI-org/openMapper/issues/40)): the program logs the stage it is at, writes to the log when the window does not answer for ten seconds (`logs/openmapper.log` in openOMSI's data folder), and lists the installation's maps on a thread of its own.

## 0.1.12 - 2026-10-26

### Fixed
- **A junction's roadway is as high as the roads' surface** ([#39](https://github.com/openOMSI-org/openMapper/issues/39)): road types drawn above the line they are laid along (the road maker's own sit 10 cm over it) got a junction whose asphalt lay that much under the roads and the kerbs; the plate's roadway now takes the height of the roads' surface.
- **No crash on exit on Wayland** ([#37](https://github.com/openOMSI-org/openMapper/issues/37)): the program ends right after saving instead of tearing its threads down in order, which crashed in libwayland-client on some Linux systems.

## 0.1.11 - 2026-10-26

### Added
- **Top view and side view in the road maker**: the road type being made is drawn from above (footways, kerbs, lanes with their lines and arrows, the middle strip, the pocket, a scale) as well as in section; two tabs under the settings switch between them.
- **Bus Pocket Tool** (Roads tool, a box with the others): tick it, pick the road type of the pocket, set how long the bay is at full width, and press on a road: that stretch of the road takes the pocket's type, the roads at both ends taper to its width with plates (lanes joined for the AI), one undo step. It stays on, pocket after pocket, until unticked. The road maker has a new Bus pocket lane (right of the roadway, before the footway; ready-made "Street with a bus pocket") for the road type a pocket is laid with.
- **Complex junction editor** (Tools > Complex junction editor…, also in the Roads tool's panel): a top-view editor where a junction is drawn out of small splines of any type in the catalogue (asphalt, road markings, footways, kerbs) with the line and arc tools, snapping to the grid and to each other's ends, moved, lifted over the asphalt and put in order. "Put on the map" lays them as ordinary, separate splines at the middle of the view (turned by a number of degrees); nothing about them is automatic.
- **Custom junctions**: the same drawing with its arms marked (where a road comes in, which way it faces, of which type) is saved as a junction piece of the content folder: a scenery object with the drawing's mesh, helpers at the arms and lanes for the AI along every lane of its pieces; it shows in the catalogue under Junctions. With "Use in automatic junctions" on, the Roads tool's automatic junctions put it where roads meet as its arms do (the same number of roads, facing about the same ways, of about the same type): the roads are cut back to its arms and bent onto them, and the plate openMapper would make is left out. "Use my custom junctions where they fit" in the Roads tool turns that off.

## 0.1.9 - 2026-10-26

### Fixed
- **Test drive no longer starts two sessions**: openMapper keeps track of the openOMSI it started; a second click or key press while it runs starts nothing.
- **Fewer freezes with no apparent cause**: the autosave copies only the changed tiles on the main thread and makes their files on another; the ground textures are looked for on a thread of its own (a big installation's folders held the window at the start and after every change of content files); the catalogue of switches is made once per library, not on every frame when a library has none; sorting the catalogue no longer allocates in every comparison; what a running openOMSI writes into the content folder no longer makes the catalogue and models start over.

### Changed
- **The bottom bar scales with the window**: when it is too narrow for all the switches (Auto-Terrain, Auto-Junction, Auto-Adapt, Auto-Align...) the last ones go into a "···" menu instead of vanishing one after another; the cursor position, the counts and the graphics card's name give way first.
- **Test drive asks what to drive** (the button, F5): where to start (where the camera looks, a start point, a bus stop, or just this view), the vehicle or none, the time of day, the weather, the traffic and the passengers. While a session runs the button reads "Close test drive" and ends it.

### Added
- **Export from the right-click menu**: right-click any object or road, then Export... writes it as O3D, OBJ, FBX, GLB or X (as text, or in binary form), through the system's file dialog.
- Simple bridge over from openMapper to other apps (MCP)

## 0.1.8 - 2026-10-25

### Fixed
- **A cable, a fence or a footpath no longer joins a junction** ([#24](https://github.com/openOMSI-org/openMapper/issues/24)): drawn near a junction plate or a road end, splines nothing drives on took them for roads and became arms of the junction or snapped to its places. Only roads join now, and holding Ctrl while drawing joins nothing, not even a road (no arm, no junction made where it crosses another road).

### Changed
- **How many tiles are loaded at once is a choice** ([#29](https://github.com/openOMSI-org/openMapper/issues/29)): View > Tiles loaded at once: 9, 25, 49, 100 (as before) or 225 tiles round the camera, the others hidden until a click on the empty zone, for big or very detailed maps that were too heavy.

### Added
- **Roads of different width are joined by a taper** ([#30](https://github.com/openOMSI-org/openMapper/issues/30)): where a road is drawn on from the end of one of another width (a 4-lane into a 6-lane), or two such roads meet at a node, a plate that narrows or widens at 1:10 is laid between them, with its lanes joined, as for a junction of two roads. Roads of one width are joined as before. Auto-Adapt in the bottom bar (on by default) turns it off.
- **Compulsory stops, request stops and termini**: a bus stop has a kind (Game logic > Bus stops while placing, and in the stop's properties). A compulsory stop always has people waiting and every bus stops; a request stop only serves a bus when somebody gets on or off; a terminus has everybody get off. Each trip says how its buses serve every stop (Lines > a trip > How the stops are served: always, when wanted, drive through, when early: OMSI 2's own `[profile_otherstopping]`), by default from the stop's kind.
- **How many people wait, get off and where**: a stop's properties hold the most and the fewest people waiting, how many get off (the weight among the stops ahead), the length of the stop and the side of its platform, as OMSI 2 and openOMSI read them from the stop's texts.
- **Split a pair of stops, and no stop names twice**: the two stops across a road keep one name (renaming one renames the other) until "Split" gives one a name of its own; stops put down by hand get a name no other stop far from them has; a stop whose name another far stop has says so with a button to make it different; the map check lists stops sharing a name with one click to fix.
- **A junction piece on a road's end**: select a road with a free end and choose a junction piece (the ones made for its road type first): it is turned and put so its place for that road meets the end. Road types can name the junction piece they are joined with by preference (Road types and junctions).
- **A better road maker**: ready-made roads to start from (quiet street, main road, avenue with a green strip, one-way street, country road, service road), a middle strip with a texture of its own that can stand as high as the footways with a kerb along it, a footway of one width on the left and another on the right (or none on one side).
- **Scripts** (Map > Scripts): the map's own Lua script for openOMSI (written into the content folder's `plugins`, run on this map only, checked as you type, with ready-made scripts to start from) and the object scripts (`.osc`) of OMSI 2 and openOMSI, checked by openOMSI's own compiler; a light switched by a script (during certain hours, blinking, in certain months) can be made in one click. The map's Lua script goes into the map's shared zip; importing it does not install it (it is code).
- **Which vehicles drive a line** (Lines > a line > Vehicles): OMSI 2's `car_use/*.ocu` files: for a span of dates the vehicle types its tours are given (only these, or mostly these with a share), and a fleet number for a single tour.

## 0.1.7 - 2026-10-25

### Fixed
- **Signs and gantries take the picture the map names** ([#28](https://github.com/openOMSI-org/openMapper/pull/28)): an object whose material is declared `[matl_freetex]` ships no picture on purpose, the map's label for the object names one; openMapper ignored the tag and drew a blank board. Now the Hong Kong gantries, light boxes and bus huts show their pictures, each placement its own, and the map check no longer calls such a slot's missing picture missing.
- **A sign panel or road marking is no longer eaten by the road it stands on** ([#26](https://github.com/openOMSI-org/openMapper/pull/26)): a picture with no transparency at all is drawn solid, so it keeps its whole face instead of being cut in bands. A sign whose picture the content does not ship stays bare, as in the game, instead of taking a same-named picture from another folder.
- **Fences and guard rails no longer flicker** ([#27](https://github.com/openOMSI-org/openMapper/pull/27)): splines are drawn from the side their faces face, as OMSI draws them, so the two faces a couple of centimetres apart stop fighting in the depth buffer (`OPENMAPPER_NO_CULL=1` draws both sides again, for a comparison).
- **Junction plates came out a dull dark grey** beside the roads they join: they were laid with the installation's dark junction asphalt. They are made of the asphalt of the roads that meet there again (the widest road's, cut so no patch comes along in rows), the installation's only where those roads have none. Use Map > Make the junctions again for junctions made before.

## 0.1.6 - 2026-10-25

### Added
- **The interface speaks all twenty-eight languages of openOMSI** (View > Language, and the command palette): English, German, French, Russian, Ukrainian, Belarusian, Kazakh, Polish, Czech, Hungarian, Spanish, Catalan, Brazilian and European Portuguese, Italian, Dutch, Swedish, Turkish, Japanese, Traditional and Simplified Chinese, Korean, Thai, Vietnamese, Indonesian, Malay, Filipino and Hindi, every text translated. The system's language is taken when none is chosen (also on Windows and for `pt-BR`, `zh-Hant`...). Chinese, Japanese, Korean, Thai and Hindi are drawn with the system's own font (a system without one keeps those languages' texts in English instead of squares). Translators work in `locales/<code>.tsv` and need no Rust; CI checks that no text lacks a translation (see CONTRIBUTING.md).

### Changed
- **Quicker names over stops and start points**: looking up an object in the catalogue no longer allocates, and the map picture's file is checked once a second instead of every frame.

### Fixed
- **Objects were drawn from both sides** ([#20](https://github.com/openOMSI-org/openMapper/issues/20)): text on OMSI's helpers and signs (stop signs, numbers) showed through mirrored, and windows modelled once per side showed twice. The back of a face shows nothing now, as in the game.
- **A long object name or path broke the properties panel** ([#21](https://github.com/openOMSI-org/openMapper/issues/21)): a name without spaces made the panel wider than its range, its rows were cut off and the view shifted on a click. Long names and paths wrap anywhere, long names on buttons are shortened.
- **Junction plates are written with both sides** ([#23](https://github.com/openOMSI-org/openMapper/issues/23)): a game that culls back faces by another rule than the one tested no longer leaves a gap where the plate should be. Use Edit > Make the junctions again for junctions made before.

## 0.1.5 - 2026-10-24

### Changed
- **A selected object glows orange** (soft, translucent, over the whole object) instead of wearing a ball or an outline; a selected road glows the same way and no longer has lines along its sides.
- **The move, rotate and scale handles follow the size of the selected object**: a small object gets short handles, a big mesh long ones, never shorter than a few dozen points on the screen.
- **README.md names the files of the latest release**: the release workflow updates the download table by itself.

### Fixed
- **Selecting a big junction mesh drew a huge cone and cloud**: the lines and dots to the objects hanging on it are thin now and as big as those objects.

### Added
- **A Windows installer and a macOS disk image**, as openOMSI has: `openMapper-<version>-windows-<arch>-setup.exe` installs for the user alone (no administrator rights, Start menu entry, desktop shortcut, listed under Settings > Apps, removed cleanly: maps and settings are never touched) and `openMapper-<version>-macos-<arch>.dmg` opens to a window with the app and an Applications folder to drag it onto. Both are in openMapper's own look. The zips stay.

## 0.1.4 - 2026-10-24

### Added
- **Multiplayer** (File > Multiplayer…): several people edit one map at once. One hosts the map open in their openMapper; the others join on the same network or from anywhere through a free Cloudflare quick tunnel (openOMSI's own). Tiles, the global settings and groups are shared as they are made; an edit made on a tile somebody else changed first is put back and its author told. Undo leaves the others' edits alone, and each editor hands out IDCodes of its own. The others' cursors, names and selections are drawn in the view in their colours. The timetable, the traffic and the descriptions stay the host's for now, and the host saves the map.

- **Load 100 tiles at a time** (View > Load 100 tiles at a time, off by default; asked once when a map is first opened): as in OMSI 2's own editor, only the 100 tiles nearest the camera are kept on the graphics card and drawn; the rest are hidden. A click on the empty zone loads the 100 tiles round that place and hides the earlier ones, so very big maps stay light.
- **Styled Markdown** in the release notes of the update window: headings, bold, italics, `code`, code blocks, lists, quotes, rules and links.

### Changed
- A window nobody touches redraws about 20 times a second instead of every frame.
- New editor installations start with Medium graphics ([#15](https://github.com/openOMSI-org/openMapper/issues/15)).
- Both ZIP dependencies and TLS updated; ZIP 9 compatibility fixed ([#14](https://github.com/openOMSI-org/openMapper/pull/14)).

### Fixed
- **The editor starts on native Wayland**: an 8-bit window format is picked first ([#19](https://github.com/openOMSI-org/openMapper/pull/19)).
- **A save that cannot finish rolls the map files back** instead of leaving them half written ([#13](https://github.com/openOMSI-org/openMapper/pull/13)).
- **The Map editor camera** looks and flies like a 3D editor's: the right button held looks round (the cursor held and hidden), W A S D fly with Q down and E up, the wheel sets the speed meanwhile and otherwise zooms to the cursor, the middle button pans.
- **Unity-style camera was jerky** with the right button: the cursor is held and hidden while it looks round, the turn comes from the mouse's own motion applied once per frame (no more jumps at the screen's edge), and lifting the eye over the ground is shown at once instead of easing while the view turns.
- **A road can leave a junction** ([#18](https://github.com/openOMSI-org/openMapper/issues/18)): clicking on a junction's plate (anywhere on it, its corners and the open side too) starts a new road from its middle, as another arm of it. The road ends at the plate's edge and a loose road end at it no longer takes the click.
- **Out of graphics memory** ([#16](https://github.com/openOMSI-org/openMapper/issues/16)): a tile of plain ground now costs the graphics card only its terrain (the paint masks and the ground cut are made only for tiles that have paint or roads, and one shared blank is used otherwise), so a map of thousands of tiles fits. If the graphics memory still runs out, the editor lets go of what it can build again and goes on instead of closing; blocks are no longer set aside beyond what is used.

## 0.1.3 - 2026-10-24

### Changed
- **Selection no longer covers what is selected** ([#11](https://github.com/openOMSI-org/openMapper/issues/11)): a selected road or object is not painted orange any more. A road shows a thin outline along its two edges, an object a small ball; the road, kerb or object stays visible under it.
- **Grips are balls sized by the road**: the points that reshape a selected spline are small shaded balls, as big as the spline is wide (a kerb's small, a street's larger) and never smaller than a few pixels however far the camera is. They are picked by the ray through the ball, with room beyond its edge, so a click near the edge works.
- The licence is GNU AGPL v3 only (`AGPL-3.0-only`), as openOMSI's; the earlier MIT notice stays for the versions distributed under it.
- Dependencies updated: `rfd` 0.17, `base64` 0.23, `tiny-skia` 0.12, `serde_json`, and the GitHub Actions of the release setup.

### Fixed
- The version badge in the README showed a fixed number; it now follows the latest release by itself.

## 0.1.2 - 2026-10-23

### Added
- **Hot reload**: new, changed and removed objects, splines and textures in the content folder (and, a little slower, in the OMSI 2 folder) show up in the catalogue and on the map by themselves, a moment after the files stop changing. Nothing to press; what openMapper writes itself does not trigger it.
- **Test drive in the original OMSI 2** (Map > Test drive in OMSI 2): the saved map and the objects, roads and textures made for it are copied into the OMSI 2 folder (asked once) and OMSI 2 is started (Windows).
- **Pictures and patterns**: Objects > Shapes and lamps > Picture / pattern: a picture or texture becomes a panel object (on the ground like a decal, or standing like a sign) that is placed, moved, turned and scaled like any object; repeats make a pattern; transparency is taken from the picture.
- **Light sources** (a lamp that glows and lights the ground round it, at night or always) and **sound sources** (a .wav that plays where the marker stands, with a reach and a volume; the marker shows in the editor only).
- **A log file**: `~/.openomsi/logs/openmapper.log` (the last run's as `openmapper.previous.log`), with the version and the graphics adapter, every warning, and the message, place and backtrace of a crash. A crash of the program shows where the log is.

### Fixed
- **Maps made or saved in openMapper did not open in OMSI 2's own editor**: global.cfg lacked the sixth line of `[backgroundimage]` OMSI 2 reads (it took the next keyword for it), and tiles lacked the empty line after `[terrain]` and `[water]`, the line for the mirror flag of a spline and the `Object Nr.` lines OMSI 2's editor writes. Tiles and global.cfg of the stock maps are now written back the way OMSI 2 wrote them, and the ground painting masks have the header of OMSI 2's own.
- Satellite imagery and terrain: a download that fails (dropped connection, a busy server) is tried again twice, then the tile again; a tile that fails no longer ends the job at three in a row by chance; an error inside the job ends it with a message instead of leaving the window waiting; the picture of a tile is built in place instead of held twice in memory (the usual end of long runs); locks poisoned by a crash elsewhere no longer cascade.
- Start on older graphics drivers (Windows 10): the editor tries every graphics backend, then OpenGL alone, then the software adapter, before giving up, and uses the most edge smoothing the adapter can really do.
- "This app can't run on your PC" on Windows 10: it is the ARM64 download on an Intel / AMD PC. Each Windows zip now has WHICH-FILE.txt saying which to use and how to tell, and the program has a manifest for Windows 10 and 11.
- A crash with a wgpu validation error ("Texture … has been destroyed"): the interface's textures are now freed after the frame that may still draw them is submitted.
- The mouse wheel over the world map (satellite choice) zooms the map only and no longer scrolls the panel beside it; the zoom steps are finer.

### Changed
- Rust 2024 edition (rust-version 1.85).

## 0.1.1 - 2026-10-23

### Added
- **Updates from GitHub**: at start openMapper looks for a newer release and offers it (Install and restart, Not now, or never offer again). Help > Updates chooses what happens: offer, install and restart by itself, or never look. The download is checked against the release's SHA256SUMS, the program's file is replaced and openMapper starts again once nothing is unsaved.
- The Objects tool shows the points where roads and junction pieces can be joined (free road ends and the helper points of pieces on the map); a junction piece lines up with another piece's point as well as with a road end.

### Fixed
- Google imagery: when Google refuses with "not available for your account and region" (accounts billed in the European Economic Area get no Google satellite tiles) the message now says so and what to do ([#7](https://github.com/openOMSI-org/openMapper/issues/7)).
- An object put on a road stays on it, turned along the road when Auto-Align is on, at the road's own height, instead of being pushed to the roadside or sunk in the ground under a raised road.

## 0.1.0 - 2026-10-23

The first release: a map editor for [openOMSI](https://openomsi.org/) that opens, edits and saves
OMSI 2 maps. Builds for Windows, macOS and Linux, each for x64 and ARM64. It needs an installed
OMSI 2; the original folder is never written to, maps and imported assets go into openOMSI's
content folder.

### Drawing and shaping
- **Roads**: two clicks make an arc that continues smoothly from a road end, joins another end with an S-curve or bend, is cut into OMSI splines and linked with its neighbours; drawing by numbers (length, radius, heading), "keep the height" for bridges and embankments, roundabouts, parallel copies (a second track, the other carriageway), tracks with overhead wire.
- **Road shape**: drag end and middle dots, heights, gradients and cant at both ends, length and radius by numbers, skew and ground cut away for tunnels, lay on the ground, reverse direction, join, the height profile coloured by gradient, and the knife (`K`) that cuts a road where you click.
- **Junctions** made automatically where roads meet or cross (asphalt plate, `[path]` lanes, footpaths round the corners), with **traffic lights** (OMSI light programs and signal heads on poles) and **zebra crossings**; junctions moved or turned carry the roads along.
- **Switches**: a track drawn off another's middle lays OMSI 2's own switch; Auto-Switch, Auto-Junction and Auto-Align switches in the status bar.
- **Road types**: a road maker (lanes each way, widths, middle strip, footways, kerb, textures) writing an ordinary OMSI spline, and per type how its junctions are made.
- **Objects**: a catalogue with the installation's folder tree and thumbnails, an assets panel with search and filters, lining up with the nearest road, objects along a road as spline attachment rows, copies in a row or a grid, hanging and linked objects, texts shown with OMSI's fonts, groups on the map and groups kept for other maps.
- **Select, Move, Rotate and Scale** tools with handles on the ground; location, rotation and scale by numbers, scaled copies of an object's type.
- **Plants**: a plant brush with a plant mix, clearing, and filling an outline (a wood, a park).
- **Terrain**: raise, lower, smooth, flatten and roughen brushes, ramps, level ground under a building, height maps exported and laid back as 16-bit PNG or TIFF.
- **Paint**: the ground's texture layers with detail textures and repeats, painting along a road, painting by slope and height.
- **Tiles**: add 300 × 300 m tiles or a ring of them, remove them, water and street-lamp light maps per tile, tiles taken from another map.
- **Own pictures** on tiles (a scan, a plan), turned, mirrored and scaled by eye.

### Real places
- **Satellite and terrain**: a world map to place the map on, imagery cut exactly to each tile from Esri, official orthophotos of several countries or your own Google, Mapbox, MapTiler or any XYZ, WMTS or WMS source, and terrain heights.
- **From OpenStreetMap**: roads, one-way streets, tram and railway tracks with switches, bus stops, bus lines with their trips, trees and woods, lakes and rivers, and buildings matched to the catalogue's models, in one undo step.
- **GPS tracks** (GPX, KML, GeoJSON) to trace or to build roads from.

### Game logic, timetable and traffic
- Start points, bus stops (in pairs on both sides), car parks, AI paths, sound sources and path rules (speed limits, density, priority, vehicle kinds).
- **Lines and timetable**: lines, trips and tours written as OMSI 2's `TTData`, routes found along the roads, departures from a first and a last time, copies of lines, holidays.
- **Traffic and AI**: road and passenger density over the day, car groups and depots, parked cars and people.
- **Depot files** (`.hof`) made from the map's lines with destinations, stops and IBIS routes, and edited in a window of their own.
- **Signal routes**, **chrono scenarios** (the map's changes over the years, with their own timetable and AI lists), **map settings**, **calendar and place**, number plates, descriptions in several languages.

### Your own models
- **Import** of mods (folders and zips), `.sco` folders and 3D models (OBJ, glTF / GLB, FBX, X, O3D) with their materials, textures and PBR maps; imported models keep their pieces as meshes.
- **Meshes**: shapes and lamps, a mesh editor (vertices, edges and faces; extrude, inset, bevel, loop cut, bridge, spin, mirror and more), booleans, a modifier stack, materials with transparency, glow, normal and ORM maps, UV maps with a UV editor, baking, lamps, street lamps and signal heads, and export to OBJ, glTF, GLB, FBX, Collada, USD, STL, PLY, 3MF, OFF, `.x` and `.o3d`.
- **PBR maker**: normal, roughness, metal and occlusion maps made from a texture, written as openOMSI reads them.

### Checking, testing and sharing
- **Test drive** in openOMSI with one key (`F5`), from the nearest start point or with a free camera.
- **Check in openOMSI** runs the game's own checks unseen and shows what they found; **Map check** names missing files, roads not joined, stops without a lane, ID clashes and more, many fixed with one click; `openmapper --check` does it without a window.
- **Safe saving**: every file written under a temporary name first, the last ten saves kept, autosave every three minutes, earlier versions brought back from the File menu.
- **Export for sharing** as a zip laid out like OMSI 2, **a plan of the map** as an SVG, and import of both.

### Workspace
- A viewport like Unity's scene view with fly, orbit and trackpad controls, a map-editor camera, an overview map, saved views, layers, names in the view, time of day with sun and shadows, seasons, graphics quality and PBR.
- **Controls** window: every key can be changed; a search for any command (`Ctrl+K`); find in map; the right-click menu; an outliner of everything on the map; undo and redo for every action with a history list.
- **New map** from OMSI 2's template, a getting-started checklist, a written tutorial, and the interface in English and Russian at 80–150 % size.
