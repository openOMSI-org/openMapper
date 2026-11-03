# openMapper: everything it can do

The complete list. A short overview is in the [README](../README.md).

## Tools and features

| Tool | Key | What it does |
| --- | --- | --- |
| Select | 1 | click an object or road (double-click a road: all of it, every piece up to where it branches or ends; All like it in the properties: every one of its type) · `Shift`+click adds or takes away · `Shift`/`Ctrl` + drag on empty ground selects with a box · drag the selection to move it all · `R` turns it round its middle · `Del` deletes it · `Ctrl+C / X / V / D` copy, cut, paste at the cursor, duplicate (links inside the copy kept, rows of objects and things hanging on them come along) · several selected: move, raise and turn by numbers, give every road one type or every object the catalogue's one (also for one object: Replace with), one even slope along a chain of roads (a bridge, a ramp), objects turned each a random way, all like one, or each the way the nearest road runs beside it (also the road button by an object's rotation), leaned with the ground under them (a fence or a shed on a hillside; the hill button by an object's rotation) or stood upright again |
| Road shape | - | select a road: drag its end dots to move an end (a road joined there follows, tangent at the joint, its far end kept; an end at a junction stays), drag the middle dot to bend it. Heights, gradients and cant (banking) at both ends in the properties, the joined road following; length and radius (straight, left, right) by numbers, the road joined at its end carried along, tangent; the ends' skew (a slanted cut: fences meeting at a corner) and the ground cut away under it (tunnels, underpasses: OMSI's terrain align); Lay on the ground puts a road or a selection of them back onto the ground (after shaping it or laying a height map over it), smoothed so they climb hills rather than every bump, roads not selected keeping their joints; Reverse direction turns a road or a chain round where it lies, the traffic on it going the other way (a one-way street, a track); the knife (K, the Knife switch in the status bar or Cut in a road's properties: a click on a road cuts it there, as often as wanted, the two pieces' lengths shown at the cursor and on whole grid steps with grid snapping on; or a right-click on a road: Cut the road here) splits a road to give a stretch its own type or settings (a bridge, a tram section), the rows of objects along it running on over the cut, and Join with the next piece makes two pieces one again where they make one curve; the height profile of a road or a chain over the ground under it, coloured by gradient, with the steepest stretch, the climb and fall and how high it runs over the ground or how deep in a cut (a click on it goes there; its joints dragged up or down set their heights, the gradients kept smooth) |
| Roundabouts | - | Roads tool option: click the centre, then the ring (or drag out to it). A one-way ring of the chosen road type, anticlockwise where traffic keeps right and clockwise where it keeps left; the roads that ran into the circle are cut back and joined to the ring with junctions |
| Drawing by numbers | - | each road is two clicks (going on from the end just built is an option); while a road is drawn: type its length, radius or heading in the fields at the view's corner (empty ones follow the cursor), `Shift` draws a straight, `Enter` builds it. "Keep the height" draws bridges and embankments: an even gradient, a rise typed for the end, the ground left alone (pillars: objects along the road) |
| Junction lights and crossings | - | select one of openMapper's junctions: "Traffic lights" writes OMSI's light program into it (main road and side roads, greens editable) with the installation's own signal heads on poles bound to it, the AI stopping at red; zebra crossings per road, with a footway across. A road drawn into a plate, or a road end dropped on one, becomes another arm of it |
| Junction moves | - | a junction (or any piece with spline helpers) moved, turned or raised carries the ends of its roads along, tangent, the road before a short arm bending with it; the ground follows its plate; openMapper's own junction plates are made again for their roads, and deleting an arm leaves the others joined (two left: a smooth joint) |
| Objects along a road | - | select a road or a chain: put the object chosen in the catalogue every so many metres beside it (one or both sides, margins, height, turn, leaning), as OMSI spline attachment rows that follow the road; the road's properties list its rows to change (step, side, height, turn, leaning with the road, the texts its objects show) or remove |
| Parallel copy | - | select a road, a track or a chain: copy it alongside to the left or right at a distance (a second track, the other carriageway); bends stay exact arcs about the same centre, the copies linked as the originals. A track's overhead wire (trams, electric trains): a wire spline (`Fahrdraht`, `Oberleitung`…) laid on the selected track piece for piece and linked, as the stock maps lay theirs; the masts go in with Objects along a road |
| Move, Rotate, Scale | V, C, X | next to Select in the toolbar: handles on the selection as in Blender, the same size on the screen however far the camera is: arrows along X, Y and Z (up) and squares to move in a plane, the circle over the ground; rings to turn about X, Y or Z (snapping to the angle step; a road turns only about Z); arrows ending in cubes to scale along an axis, squares for two, the circle for all three (one object along its own axes). An OMSI 2 map keeps no size for an object, so a scaled one becomes a scaled copy of its type beside the original (`name~s1.5_1.5_2.sco` in the content folder: its meshes, collision mesh, lamps and paths scaled; scaled back to 1 it is the original again) |
| Location, rotation, scale | - | an object's properties: X, Y and Z (the height above sea level, whatever the object keeps it as; its height above the ground under it shown beneath), rotation about X, Y and Z, and its size along each axis (proportions kept or not), each on a row of its own to drag or type |
| Auto-Switch, Auto-Junction, Auto-Align | - | status bar: switches where a track branches off, junctions where roads meet, objects lined up with the nearest road (an imported model is never lined up) |
| Controls | - | File > Controls: every key, each one changeable (click it, press the new key; kept in the settings), the mouse, and which set of keys to start from: openMapper's own, OMSI 2's editor's (with its mouse too) or Blender's |
| Ruler | U | status bar: click two points for the distance, the height between them and the gradient, written on the line in the view |
| Snapping | - | status bar: `Grid` (0.5–10 m) for placing, moving, road ends and reshaping, the grid drawn on the ground round the cursor while it is on; `Angle` with its step (1° to 90°) for headings, also what `R` turns by |
| Roads | 2 | click the start, then the end: the arc builds itself. A road continues smoothly from a road end it starts at, joins another end with an S-curve or bend (biarc), is cut into OMSI splines and linked with its neighbours. It can shape the ground under itself. |
| Junctions | - | automatic: a road drawn into another's middle or across it cuts both and fills the gap with a junction object of its own (asphalt plate, `[path]` lanes for straight on, right and left, footpaths round the corners), written as ordinary OMSI content into `Sceneryobjects\openMapper\<map>` |
| Assets | B | a panel like Unity's Project window: folders of objects and splines, a grid of pictures with search, kind filter and size; an asset's facts, place or build with it, favourite, PBR, show in the file manager, delete (content folder only, into `openMapper\bin`); double-click to use, drag a picture into the view to place it |
| Switches | 2 | a track drawn off or into another's middle lays OMSI 2's own EW190 switch (left or right, for its rail type): the main track is cut out under it, the new track joins its diverging heel |
| Plants | 3 | Objects tool: "Plant brush" sows the chosen tree (or a mix: right-click entries, "Add to the plant mix") with a spacing and random turns; "Clear" takes trees away; "In an outline": click round an area (a wood, a park) and it is filled at once, or cleared, roads and tracks left free, one undo step |
| Lining up | 3 | buildings face the nearest road with their front at a set distance from its edge (Shift: put freely); junction and crossing pieces (spline helpers) click onto the road ends near them |
| AI paths | 4 | Game logic > AI paths > "Edit existing": pick a path, drag its ends and middle, Ctrl+click on it adds a point, Del deletes it; lanes are drawn thin with small arrows |
| Bus Pocket Tool | - | Roads tool: tick "Bus Pocket Tool", pick the road type of the pocket (a road type made with a Bus pocket lane in the road maker, or any other that is wider on the right) and how long the bay is at full width, then press on a road: that stretch (the bay and a taper at each end) is cut out and takes the pocket's type, moved sideways so the through lanes stay where they were, and the width adapter lays a tapering plate (1:6) with its lanes joined at each end. One undo step each; the tool stays on, pocket after pocket, until the box is unticked |
| Complex junction editor | - | Tools > Complex junction editor…: a top view to draw a junction out of small splines (asphalt, road markings, footways, kerbs, any type of the catalogue) with Line and Arc (three points) tools, snapping to a grid and to other pieces' ends; pieces picked, moved, given a lift above the asphalt, put under one another. "Put on the map" lays the drawing as ordinary linked-per-piece splines at the middle of the view, turned as chosen: not automatic in any way, so it can be changed spline by spline afterwards. Painted lines (solid, dashed, double, a crossing's bars; colour, width, dash and gap) are made in the editor itself as splines with a transparent ground (`Splines\openMapper\marks`) and laid like any piece. Drawings are saved in the content folder (`openMapper\junctions\<name>.json`) and can be opened again |
| Custom junctions | - | a drawing with its arms marked (Arm tool: where a road comes in, then a point out along it; with the road type it takes) saved as a junction piece: a scenery object (`Sceneryobjects\openMapper\Junctions\…`) whose mesh is swept from the drawing's splines, with `[splinehelper]`s at the arms and a `[path]` along every lane of its pieces, so the AI drives through it. It is in the catalogue under Junctions like any junction piece (roads snap to it, it moves with its roads), and where "Use in automatic junctions" is on, the Roads tool's automatic junctions use it instead of their own plate when the roads meet as its arms do: the same number of roads, each facing within 30° of an arm once the piece is turned to fit, of the same type (or about as wide); the roads are cut back to its arms and bent onto them |
| Road types | - | Roads tool > "Road types and junctions…": a road maker (lanes each way, a bus pocket lane, a top view and a side view of the road made, lane width, middle strip, footways, kerb, textures) writing an ordinary OMSI spline, and per road type how its junctions are made (corners, lights, zebras, or no junction at all) and which junction piece it is joined with; the maker starts from ready-made roads and has a middle strip (textured, raised or level) and different footways each side. A road's free end takes a junction piece with its place for that road type meeting the end (Junction on an end in its properties). Tracks branch off one another as switches and cross roads on the level |
| PBR maker | - | an object's "Create PBR" (or Tools > PBR maps for a picture): normal, roughness, metal and occlusion maps made from a texture with a live preview, written as openOMSI reads them (`_nn`, `_orm`); then "Configure PBR" to tune them again or delete them |
| Objects | 3 | catalogue as the installation's folder tree with thumbnails; classes read from the files themselves (roads, tracks, footpaths, decoration, technical AI/helper splines; trees, stops, junction pieces, technical helpers), search, turning towards the nearest road |
| Game logic | 4 | start points (`[entrypoint]`), bus stops (compulsory, request or terminus; at the kerb of the nearest lane, and with "On both sides of the road" a twin of the same name across it facing the other way; renaming one renames the other until "Split"; the people waiting, getting off, the length and the platform's side in their properties; names never twice on stops far apart), car parks (one by one or in a row along the kerb), AI paths for cars and people drawn like roads and snapped to lane ends, sound sources and other editor-only helpers |
| Terrain | 5 | raise / lower / smooth / flatten (to where the drag starts or to a height typed) / roughen brush (rolls and dips of a natural field); Ramp: click a start and an end and the ground between becomes one even slope as wide as the brush (an embankment, a ramp, a level bed), its gradient said; Level ground in an object's properties makes the ground under its outline level with its foot (a house on a slope). Height map: the whole map's ground exported as a 16-bit grey PNG (a pixel per terrain sample, its lowest and highest height in a `.txt` beside it) to shape in World Machine, Gaea or an image editor, and any grey picture (8- or 16-bit PNG, TIFF) laid back over all the tiles, replacing the ground or added to it, smoothed if it steps |
| Paint | 6 | painting with the map's ground textures (OMSI `[groundtex]` masks), each layer's texture and detail texture (the fine grain seen close up) changed with its painting kept, their repeats, the layers' order (a later one drawn over an earlier one, its painting moving with it) and paint detail (128 to 1024 texels a tile: its masks made again at the new size); a selected road's (or chain's) "Paint the ground along it" lays a layer beside it or under it too, out from its edge with a soft rim (a gravel verge, dirt by a track); By slope and height paints the chosen layer over the whole map where the ground is so steep (and so high), fading in at the edges: rock on slopes, sand by the water, snow on the tops |
| Tiles | 7 | add a 300 × 300 m tile next to the map (or a whole ring of them round it with one button), remove one with right-click; a tile clicked gets water (OMSI's `[water]`, a surface at a height, seen where the ground lies below it: lower the ground for a lake or a river) and says whether its street lamps light the ground at night (`[variable_terrainlightmap]`, on for every new tile as on the stock maps' tiles) |

* **Lines and timetable** (Map menu or `L`): lines, their trips and tours, written as OMSI
  2's `TTData` (`.ttl`, `.ttp`, `.ttr`, `StnLinks.cfg`, `Busstops.cfg`). Pick the stops on the
  map or from a list; the route along the roads is found by itself (drawn blue, each problem
  named), "Make the way back" turns a trip round, a line is copied with its trips and tours (a variant to change), departures fill themselves from a first and a
  last time with a break, and a map without a depot gets one so AI buses drive the tours. The
  files of a stock map stay as they are until its timetable is edited.
* **Traffic and AI** (Map menu or `T`): the road and passenger density over the day as a
  curve with draggable points, car groups and depots of `ailists.cfg` with vehicles picked
  from the installation, which groups drive about on their own, parked cars and people. A group
  the map lacks can be copied from OMSI 2's own maps.
* **Vehicles of a line** (the Lines window's Vehicles tab): OMSI 2's `car_use/*.ocu` files: for a span of dates the types its tours are given (only these, or mostly these with a share) and fleet numbers of single tours. **How a trip serves its stops** (always, when wanted, drive through, when early) per station.
* **Scripts** (Map > Scripts): the map's Lua script for openOMSI (`plugins/map_<folder>/main.lua`, run on this map only, checked as typed, ready-made scripts to start from), object scripts (`.osc`) read and edited with openOMSI's own compiler checking them, and lights switched by a script made in one click.
* **Depot files** (Map > Depot files, or the Traffic window's last tab): the map's `.hof`
  files, which give its buses their destinations, the stop names on their displays and the
  routes their IBIS knows. One click makes one from the map's lines (a destination for each
  trip's terminus, the stops it calls at, a route with its code for each way a line goes, the
  front and side signs' texts split into two lines where they are long); later lines are
  added with another click, what is there kept. The sample map and a town from OpenStreetMap
  get theirs with their lines (their depot's buses use it, not Grundorf's). Every text of a destination or stop, a
  route's code, destination and stops in order can be changed. Saved into the map's folder
  (where OMSI 2 looks) and the content's `HOFs` folder (where openOMSI looks). The map check
  names a depot file a depot uses that is nowhere, one of another map that knows none of the
  lines' destinations (a new map's depot has Grundorf's), and one lacking what the lines
  need, each put right with one click.
* When roads a trip runs on are cut by a new junction, moved or deleted, its way is found
  again on saving (a lane its saved track names gone, or no longer as long), so the game
  does not lose the bus; the other trips' files, and trains on their tracks, stay as they
  are.
* Timetable and traffic edits are part of undo / redo; trips have times at each stop, spread
  by distance or fixed by hand.
* **Export map for sharing** (File menu, or `openmapper --export global.cfg out.zip`): a zip
  laid out like OMSI 2 with the map and every file of the content folder it uses (generated
  junctions, imported models with their textures and PBR maps, mod assets), never the
  installation's own files; a `readme.txt` lists what it needs. Dropping the zip on openMapper
  (or `openmapper --import file`) installs it.
* **A plan of the map** (File menu, or `openmapper --plan global.cfg out.svg`): an SVG
  picture from above with the tiles, water, roads at their widths, tracks and footpaths,
  every bus line's way in its own colour with a key, the stops with their names, a scale
  bar and north: to share with players or print.
* **Descriptions in several languages** (`global_<LANG>.dsc`) in the map's properties, and
  **holidays and school holidays** (`Holidays.txt`) in the Lines window.
* **Satellite and terrain** (Map menu or `I`): a map of the world in the window (drag, wheel, search a
  place by name), "Place the map here" and a click put the map's middle tile there (north turned
  by any angle); a `[worldcoordinates]` map lies on the Earth by itself. Then imagery for all tiles,
  the selected one, or each new tile by itself, and the terrain heights with it or on their own
  (relative to the middle tile, raised, smoothed; one undo step; the tiles' shared edges match).
  Each tile's picture is cut from the source exactly to the tile at the source's own detail (up to
  4096 px), so tiles meet without a seam, and lies on the ground in the 3D view; the overlay is on
  or off and as opaque as wanted, as a whole (also View menu) and per tile (also in the Tiles tool).
  The sources and how sharp they are at the map's place are listed, the sharpest picked: Esri World
  Imagery (no key), official orthophotos with no key (Berlin, Brandenburg, NRW, Bavaria, Lower
  Saxony, the Netherlands, Switzerland, Austria, France, New York State), and with the user's own
  key Google (Map Tiles API), Mapbox or MapTiler; any XYZ, WMTS or WMS server can be added. Heights
  come from the AWS Terrain Tiles (Terrarium), place search from OpenStreetMap's Nominatim. Each
  source's attribution is shown. Downloads are cached in `~/.openomsi/cache/openmapper/tiles`
  and never fetched twice; the map keeps its pictures in `openMapper/` in its folder (the game
  ignores it), left out of shared exports unless asked for.
* **From OpenStreetMap** (Map menu): for a map placed on the Earth, what OpenStreetMap
  has of its place (Overpass API, kept a week in the cache), shown with how much there is:
  * roads by kind (main roads, streets, service roads), each built with the road type chosen
    for it as if drawn: one street's ways joined into one road, corners rounded into OMSI's
    arcs, cut exactly at the map's edge, junctions (with Auto-Junction on) where roads meet or
    cross, where streets end at one node, and roads ending in a junction plate there before
    joined to it as arms; one-way roads
    (each carriageway of a dual road) with a one-way type if one is chosen; tunnels and roads
    lying on ones the map has are left out;
  * tram and railway tracks with a track type, crossing roads on the level; with Auto-Switch
    on, a track branching off another or joining it gets a switch there (the installation's
    own, its new track starting from the diverging heel), else the track it meets is cut there;
  * bus stops with their names, the stop object chosen put at the kerb of the nearest lane;
  * bus lines: each route through two or more of the map's stops a trip of its line (variants
    stopping alike once, a route leaving the map cut to its longest run within it), the way
    found along the roads, a tour from 06:00 to 22:00 for each new line, a depot if none;
  * trees, rows of trees and woods planted with the plant mix, none on a road.
  * water: lakes and rivers (with their islands) dug below their banks, sloping down from them,
    and OMSI's water put on their tiles a little under the banks.
  * buildings: each outline filled with the model of a catalogue folder (Buildings_MC, …)
    whose ground plan and height (OSM's height or storeys, else what its kind usually is)
    come nearest, its front to the nearest road; an outline no model fits is left out.

  A map without a start point gets one on a lane some 40 m before a bus stop, so the town
  can be driven at once. All a build brings is one undo step. © OpenStreetMap contributors
  (ODbL).
* **Tiles from another map** (Map menu): another map's tiles (ground, painting, water,
  objects, roads, start points) put beside or into this one: its tiles shown over this
  map's on a small plan, picked by a click and moved into place, raised if wanted, the
  ground eased where the maps meet. IDCodes are given anew, roads running from one tile
  taken into another stay joined, its ground textures are found among this map's or added
  in the same order. One undo step; lines, timetable and AI lists stay with the other map.
* **GPS tracks to trace** (Map menu, or drop the file on the window): a route recorded on a
  phone or drawn in another program (GPX tracks and routes, KML, GeoJSON) laid over a map
  placed on the Earth, drawn in magenta in the view to draw roads along, or built at once
  into a road of the Roads tool's type, as if drawn (corners rounded, junctions where it
  meets the map's roads, only its runs over the map's tiles), one undo step.
* **Own pictures** on tiles (a scan, a screenshot, an old plan), no place on the Earth needed:
  "Load image file…" for a tile (or drop the picture on the window with the Tiles tool and a tile
  clicked), or one picture over a rectangle of tiles, cut into their pieces; turned (90° steps or
  any angle), mirrored, moved and scaled by eye with the 3D view following.
* **Names in the view**: bus stops and start points near the camera carry their names over
  them (View menu to switch off).
* **Time of day** (View menu): the view lit at any hour, the sun where it stands then at the
  map's place today (Calendar and place, summer time counted), the sky and light going through
  dusk to a moonlit night; shadows follow the sun, and by night the objects' night textures
  and light maps glow (lit windows, street lamps, signs) as in the game.
* **Season** (View menu): the view in summer, dry summer, spring, autumn, winter or snow,
  with the seasons' own textures as OMSI keeps them (`Fall`, `Winter`, `WinterSnow`…), the
  ground's too.
* **Search for a command** (`Ctrl+K`, Edit menu): every command, tool and window found by
  its name or a word about it in either language ("timetable", "lake", "уклон"); for what
  lives in a panel (a road's height profile, a junction's lights, planting a wood) it opens
  the tool and says where to look. The arrows choose, `Enter` runs it.
* **Saved views** (View menu): places of the map to come back to (a busy junction, the
  depot, a stop), named and kept with the map; `Ctrl+1…9` flies to the first nine,
  `Ctrl+Shift+1…9` keeps the view there.
* **Layers** (View > Show): trees, other objects, roads and splines each hidden or shown;
  what is hidden is not picked or box-selected either, handy in a dense town.
* **Copies in a row** (an object's or a selection's properties): a fence, a row of houses, an
  avenue: N copies a step apart, each turned a few degrees more if wanted, and in several
  rows for a grid (an orchard, a car park), in one undo step.
* **Right-click menu** in the view (Select tool, and Game logic on one of its things): what
  is under the cursor is selected and the menu offers copy, cut, paste there, duplicate,
  delete, group and ungroup, select all like it, go to, keep for other maps, select all and
  deselect.
* **Groups on the map** (`Ctrl+G`, `Ctrl+Shift+G` to undo, or the right-click menu, or the
  selection's properties): things kept together under a name (a stop with its shelter and
  signs); a click on one selects all of it and a drag moves all of it, `Ctrl` + click takes
  one alone. They are listed in the Select tool's Objects tab and kept beside the map in
  `openMapper/groups.json` (the game ignores it).
* **Groups for other maps** (Edit menu): a selection kept under a name (a stop with its shelter and signs, a
  crossing with its lights, rows and what hangs on them included), to put down again in any
  map with `Ctrl+V`; kept as tile files in `<content>/openMapper/groups`.
* **Hanging objects** (an object's properties, Hanging on it): what hangs on its attachment
  points (`[new_attachment]`), and the catalogue's object hung on one, as OMSI 2's editor does
  it (signs on a post, a timetable on a stop).
* **Linked objects** (an object's properties), OMSI 2's `[varparent]`: what an object takes
  its state from, picked with a click in the view and drawn as a line to it: a signal head
  bound to a crossing's light program with the light it shows chosen by name, a stop's
  display to its stop; a crossing lists the heads that take their lights from it, and a signal head put within 40 m of a crossing with a light program is linked to it at once. The map
  check names objects linked to one that is gone.
* **Path rules** (a road's or an object's properties), as in OMSI 2's editor: a speed limit,
  traffic density, priority, trucks, no cars or buses only, each for one path (lane), the
  paths' numbers shown on the road or object in the view while its rules are open.
* **Signal routes** (Map menu), as OMSI 2's editor keeps them in `signalroutes.cfg`: for each
  railway signal the track pieces it protects (taken from the tracks and switches selected in
  the view), its distant signal, the next signal, its aspect and a speed limit; each piece's
  place among its tile's paths is counted again on saving, as the game counts it (all 649
  pieces of the stock maps come out the same). The map check names routes whose signal or
  track is gone.
* **Chrono scenarios** (Map menu): the map's changes over the years listed with their dates,
  tiles, the lines they take off and their descriptions; the view shows the map as on any day
  chosen, every scenario active then laid over it as the game does. A scenario's dates, the
  lines it takes off, its name and description are edited there, and new scenarios made (the
  installation's scenarios are brought into the map's own copy first). What is selected in the
  view is put into a scenario (it appears from its first day), taken away from that day on, or
  given the catalogue's type then, written into the scenario's tile patches as OMSI 2's
  editor does (`[selobject]`, `[delete]`, `[typ]`). What a scenario changes is listed tile by
  tile (what appears, goes, changes type, texts or rules), each to go to, take out of the
  scenario or put back into the map itself (every stock patch is written back unchanged).
  "Edit its own things on the map" brings what a scenario adds onto the map to select, move
  and change like anything else, what is placed meanwhile included; Done (or saving, a test
  drive, another map) writes it back into the scenario. "Edit its timetable" puts the map's
  timetable with the scenario's laid over it into the Lines window: lines, trips and tours
  changed there are the scenario's, a line of the map taken away is taken off from its day,
  and only what differs from the map's is kept in its `TTData`. "Edit its AI lists" does the
  same for the Traffic window: vehicles and type groups added to a group or depot, and a
  depot's own depot file, go into its `ailists_#upd.cfg`. A scenario may also bring its own
  ticket pack and money, other texts on a map object and other path rules on a road or object
  from its day.
* **Map settings** (Map menu), as in OMSI 2's editor: money system, ticket pack, depot,
  years and year offset, repair time, real rail and the seasons of the year.
* **Calendar and place** (Map menu), the map's own files as OMSI 2's editor keeps them:
  where it lies and its time zone (the sun), summer time (EU rules filled in for a span of
  years), public and school holidays by year (the timetable's day types) and the AI cars'
  number plates (made from a pattern such as `B-XX 9999`).
* **Objects on the map** (the Select tool's Objects tab): everything on the map as a tree of
  the folders its files lie in (Sceneryobjects › Buildings_MC › …), each kind with how many
  there are and each one a click away (selected and flown to); a search by name, file or
  IDCode; a right-click selects every one of a kind or everything in a folder.
* **Find in map** (Edit menu or `Ctrl+F`): stops and start points by name and a point by its coordinates (`x, y`) to go to; every kind of object and spline the map uses,
  with how many, by name, file or IDCode; select them all, go from one to the next, or replace
  them all with the object (road type) chosen in the catalogue in one undo step.
* **Texts on objects**: street signs, stop plates and line numbers show their labels in the
  view, drawn with OMSI's `.oft` fonts as the game draws them (`[texttexture]`); an object's
  texts are listed by name in its properties (`@` starts a new line).
* **Open any map**: OMSI 2's own (saved as a copy into the content folder) and mod maps.
  Right-click a map where maps are chosen: make a copy of it under a new name (to try
  something out), show it in the file manager, or move a content folder map into
  openMapper's bin (`openMapper\bin\maps`, from where it can be put back by hand).
* **New map** from OMSI 2's own template (`template/NewMap`): name and description, size, the
  ground's height above sea level and its texture, keep right or left, how busy the streets
  and stops are, years and year offset, money and tickets, seasons (four with a snowy winter,
  a mild winter, or summer only), real rail, and "Start from a real place" (the satellite,
  terrain and OpenStreetMap window opens at once); the car groups its traffic names are copied
  from the stock maps (the template has none, so its streets would stay empty).
* **Getting started** (Help menu, opens by itself for a new map): a checklist that ticks
  itself as the map grows (road, stop, start point, line, tour, buses, checked, saved, test drive),
  with a "Show me" for each step.
* **Test drive**: the Test drive button (`F5`) asks where to start (where the camera looks, a start point, a bus stop, or just this view with a free camera), the vehicle or none (on foot), the time of day, the weather, the traffic and the passengers, saves the map and starts openOMSI on it. One session at a time: while it runs the button reads Close test drive and ends it. From the start point nearest the view without asking: `Shift+F5` (Map menu, or a right-click on Test drive).
* **Export a model** (right-click an object or a road > Export…): O3D, OBJ, FBX, GLB or X (as text, or binary), written where the file dialog says; a single object comes out as the model is made, several things (roads too) as they stand together.
* **A bridge to other programs (MCP)**: `openmapper --mcp path/to/global.cfg` is a [Model Context Protocol](https://modelcontextprotocol.io) server on stdin / stdout that reads the map once, with no window, and answers until it is closed. Tools: `map_info` (tiles, bounds, counts), `convert` (a point as latitude / longitude, world metres and tile + local metres), `check` (the map check as data, nothing cut short), `query` (objects, roads, stops and start points near a point), `terrain` (heights at points or along a line) and `reload`. Read-only; with `--allow-edit` it also has `add_object`, `delete_object` and `save`. For an agent: `claude mcp add openmapper -- openmapper --mcp /path/to/global.cfg`.
* **Check in openOMSI** (Map menu): the map saved and run by the game itself, unseen, with
  its own checks (lanes with no road surface under them, roads buried in the ground, steps
  a bus wheel cannot climb, chained roads apart in height, start points off the road, every
  trip's way on the lanes), the game's picture of the map from your view beside what it
  found, the problems first; a place it names is a click away. A few seconds on a small map.
* **Map check** (Map > Check map…): missing objects, roads and textures, road ends lying on
  one another but not joined, links to splines that are gone, bus stops with no lane near,
  start points away from a road, IDCodes used twice, a too low NextIDCode, objects off their
  tile, tiles without a file or ground, street lamps on a tile the game lights no ground from, tiles far fuller than the stock maps' (a wood planted too densely); trips no way is found for, tours without departures or
  a depot, traffic groups missing from the AI lists (copied from OMSI 2's own maps with one click where they have them), objects linked to one that is gone, stops no trip calls at. Every issue is explained, a click flies there, many
  are put right with one more click (and can be undone). `openmapper --check path/to/global.cfg`
  prints the same without a window.
* **Safe saving**: every file is written under a temporary name and put in place only once
  all of them are written, so a full disk or a crash leaves the map as it was. The files a save
  replaces are kept (the last 10 saves, in `<content>/openMapper/backups/<map>`); File > Earlier
  versions… puts the map back as it was before any of them. The changes are autosaved every
  3 minutes to `<content>/openMapper/autosave` (never into the map) and offered back at the next
  start if openMapper ended without saving (the timetable and the traffic too).
* **Map picture** (`picture.jpg`, shown where maps are chosen) taken from the view, and the
  view the map opens with (`[mapcam]`), both in the map's properties. A map opened again
  opens where it was left (kept in openMapper's settings, not in the map).
* **Favourites and recently used** at the top of the catalogue: right-click an entry to star
  it.
* **Overview map** in the corner of the view (`M`): tiles, roads, tracks and the camera;
  click or drag on it to move there, double-click for a larger one.
* **Asset import**: drop onto the window or use the button:
  * a mod (a folder or `.zip` laid out like OMSI 2) is merged into the content folder;
  * a folder of `.sco` files (an object's folder with its `model` and `texture`, or several)
    becomes a folder under `Sceneryobjects`, its objects ready to place; one of `.sli` files a
    folder under `Splines`; models anywhere in a folder or `.zip` (not only at its top) become
    objects each, and a folder with only a `texture` folder beside its models is not taken for a mod;
  * a 3D model (`.obj`, `.gltf` / `.glb`, `.fbx` from Blender, 3ds Max, Maya and asset stores, `.x`, `.o3d`) becomes a new object, with every node's transform, its materials' colours and textures (embedded ones too; any picture format is turned into one OMSI reads), and their normal and roughness / metalness / occlusion maps as `_nn` / `_orm`: a `.sco` is
    written for it, the textures are copied, and so are their **PBR maps** (`_nn`, `_orm`,
    `_rr`, `_mm`, `_aa` …, the way openOMSI finds them).
* **Meshes** (openMapper's own objects, kept as `<name>.openmapper.json` beside the `.sco`
  and `.x` files written from it, so OMSI 2 and openOMSI read them as any other object):
  * **Shapes** (Objects tool > Shapes and lamps, the Assets panel's Shapes…): cube, plane,
    grid, circle, triangle, triangular prism, wedge, corner wedge, pyramid, sphere,
    icosphere, cylinder, cone, torus, tube, capsule, stairs, arch; textured in metres, put on
    the map with a click, their sizes and segments changed in the properties until edited.
  * **Polygons** (Objects tool > Shapes and lamps > Polygon): click the corners on the map (the
    first corner or `Enter` closes it, `Esc` takes the last corner back; with Snap to road edges
    and surfaces on, a corner takes the height of the face or edge under the cursor), or fill the
    outline that the selected roads and splines make end to end (kerbs round an island): a flat
    sheet, or a slab of a chosen thickness, of a plain colour or a picture repeating every so many
    metres, at the corners' heights; an object like any other (`Tab` edits its mesh).
  * **Edit the mesh** (`Tab`, or Edit the mesh in an object's properties; any object, a copy of
    its type of its own made first, its textures and lamps carried over): vertices, edges or
    faces (`1 2 3`) picked with a click, Shift + click, a box, a second click or Alt + click
    for a loop, `L` / `Ctrl+L` linked, `A`, `Alt+A`, `Ctrl+I`; moved, turned and scaled with the
    handles; extrude (`E`), inset (`I`), bevel (`Ctrl+B`), loop cut (`Ctrl+R`), their size
    following the mouse until a click (the wheel: segments or cuts; Adjust the last operation
    after), subdivide, fill (`F`), bridge loops, connect (`J`), poke, merge (`M`), dissolve,
    delete (`X`), duplicate (`Shift+D`), split (`Y`), separate into an object (`P`), spin,
    mirror, smooth, triangulate (`Ctrl+T`) and back to quads (`Alt+J`), normals flipped or set
    outward, smooth or flat shading, seams and sharp edges, materials per face; its own undo
    (`Ctrl+Z`); x-ray picking.
  * **Parts**: an object's meshes listed (shown or not, renamed, duplicated, deleted, a shape
    added as a part); Separate parts into objects makes each one an object standing where it
    stood; several objects selected: **Join** (`Ctrl+J`) makes them one.
  * **Booleans (CSG)** of any objects (two or more selected; the one in the properties against
    the others): union, difference, intersection and slice (both pieces kept), the cutters
    kept or not; and a Boolean modifier with another part.
  * **Modifiers**, as Blender's stack (on, off, order, apply): array, mirror, solidify, bevel,
    subdivision surface, decimate, triangulate, smooth, displace, wireframe, screw, simple
    deform (twist, bend, taper, stretch), cast, weld, edge split, boolean, follow the ground.
  * **Materials**: colour, texture (any picture; OMSI's formats kept, others made PNG), its
    transparency, glow, shine, normal and ORM maps (openOMSI's `_nn`, `_orm`), OMSI's night
    texture and light map; Import materials from an `.mtl`, a glTF / GLB / FBX, or the
    pictures of one PBR material (base colour, normal, roughness, metalness, occlusion,
    opacity, emission: packed as openOMSI reads them); a texture saved as PNG, JPEG, BMP, TGA,
    TIFF, WebP or DDS.
  * **UV maps**: unwrap by seams (conformal), smart project, cube, cylinder, sphere, from the
    view, lightmap pack; the UV editor window with the texture under the faces (pick, move,
    turn, flip, scale, fit, pack, even out islands, seams from islands).
  * **Bake** (as Blender's): the object's textures and colours painted into one picture on a
    new UV map, with ambient occlusion and its lamps' night light (a light map) if chosen, so
    the whole object is one material.
  * **Lamps**: glows (OMSI's `[light_enh_2]`: colour, size, a cone or all round, always, at
    night or by a script's variable) and light on the ground (`[maplight]`); a **street lamp**
    and a **signal head** of chosen lamps made in one click (the head lit by its own script
    from the phase of the crossing it is linked to, as OMSI 2's `Ampel_Kfz_1`; it takes its
    light from a crossing with a light program within 40 m).
  * **Export** (an object's or the selection's Export…, objects and roads): OBJ + MTL, glTF,
    GLB, FBX (binary, as Blender writes it), Collada, USD, STL, PLY, 3MF, OFF, DirectX `.x`
    and OMSI `.o3d`, textures copied beside (or packed in), metres with z up.
  * A road or spline's shape made an object (Copy as an object, Replace by an object), to edit
    as a mesh.
  * Imported models come in as these objects, a part for each node.
* **PBR** in the viewport: normal, roughness, metalness and AO maps are found by
  openOMSI's rules (`omsi-texture`) and drawn with the same derivative-based tangent frame.
* Undo / redo for every action (Edit > Edit history… lists the steps: a click goes back or forward to one), twenty-eight languages, those of openOMSI (View > Language), interface size 80–150 % (View menu).
* **Multiplayer** (File > Multiplayer…): one map edited by several people at once. One of them
  hosts the map open in their openMapper, the others join with an address: on the same network
  (`192.168.0.10:27560`, port 27560 by default) or from anywhere through a free Cloudflare quick
  tunnel (`https://….trycloudflare.com`, openOMSI's own: nothing to set up on the router;
  `cloudflared` is fetched and checked the first time). A password is needed (hosting
  without one only on the local network, chosen and confirmed) and proved without being sent;
  over the internet it must have 8 letters or more. What to know before hosting, and a host's
  limits: [COLLABORATION.md](COLLABORATION.md).
  * Shared as they are made: every tile (objects, roads and splines, ground, paint, water),
    the global settings and the tile list, and the groups. Each edit goes to the host with the
    versions of the tiles it changed; when somebody else changed one of them first, the edit
    is put back and its author told, so nobody's work is written over unseen.
  * Undo stays each person's own and leaves the others' edits alone: the steps on a tile
    somebody else changed since are forgotten.
  * Every editor hands out IDCodes of its own block, so two people placing things at once
    never make two with the same number.
  * The window shows who is in (each in their colour), the addresses to give out, whether
    one's edits have reached the others, and a chat.
  * The others' cursors are drawn in the view with their names and the tool in their hand,
    and what they have selected is ringed (objects) or traced (roads) in their colour.
  * Not shared yet: the timetable, the traffic and AI and the descriptions stay the host's (a
    guest's change of them is put back). The host saves the map; a guest's copy goes into
    `maps/<folder>_multiplayer`, saved once they leave. Everybody needs the same objects and
    splines installed. On the LAN the connection is not encrypted (the tunnel is), and
    everybody who joins can change the whole map.

## Controls

* **Mouse (as in Unity's scene view):** hold the right button to look round and fly with
  `W A S D` (`Q / E` down / up, `Shift` faster, the wheel sets the speed meanwhile), the wheel
  zooms, the middle button pans, `Alt` + left / right / middle drag orbits / zooms / pans, a
  left drag on nothing selects with a box. View > Camera > Map editor: right drag turns round
  the cursor and dragging the ground moves the map.
* **Trackpad:** two fingers move the map, pinch zooms, twist turns; Option + two fingers or
  Option + drag turns and tilts.
* **Keyboard:** `W A S D` / arrows move, `Q / E` turn, `1–7` tools, `V / C / X` move, rotate and scale handles, `Tab` edit the selected object's mesh, `Ctrl+J` join objects, `M` overview map, `B` assets, `L` lines, `T` traffic, `I` satellite and terrain, `R` turn object, `Del`,
  `F` go to selection, `[ ]` brush size, `G` tile grid, `Ctrl+Z`, `Ctrl+Shift+Z`, `Ctrl+S`,
  `Ctrl+C / X / V / D` copy, cut, paste, duplicate, `Ctrl+A` select all, `Ctrl+F` find in map, `Ctrl+K` search for a command, `Alt + arrows` nudge the selection a grid step (`Shift`: 10 cm), `Alt + Page Up / Down` raise or lower it, `U` ruler, `Home` look straight down, `F5` test drive (`Shift+F5`: from the start point nearest the view), `F12` save a picture of the view (into `~/Pictures/openMapper`), `Ctrl+1…9` go to a saved view (`Ctrl+Shift`: keep the view there).
* **OMSI 2's editor (View > Preset Key Shortcut (from other applications) > "OMSI 2's editor: mouse and keys"):** the mouse
  and the keys of OMSI 2's own map editor in one switch: the middle button turns the view, the
  right button held and moved up and down zooms, a right click on the ground goes there
  (`Shift` + right click: the menu), the wheel zooms. `G` moves the selection (the tile grid
  moves to `Shift+G`), `R` turns it, `N` puts the object chosen in the catalogue at the cursor
  and goes into move mode for it, `Enter` confirms what is being drawn (the road is built on
  `Ctrl+Enter`), `Del` deletes. View > Controls shows the set and every key is changeable
  there; one's own keys are kept when the set is changed.
* **Blender (View > Preset Key Shortcut (from other applications) > "Blender's keys"):** Blender's own keys over the map:
  `G` moves the selection (the tile grid moves to `Shift+G`), `Shift+S` scales it, `R` turns it,
  `X` deletes (as `Del` does), `Shift+A` puts a new object at the cursor, `Shift+D` duplicates,
  `Tab` edits the mesh (its keys were Blender's already: `E`, `I`, `Ctrl+B`, `Ctrl+R`, `M`, `F`,
  `P`, `Alt+Z`…), `F3` searches for a command, `Numpad 7` looks straight down. `W A S D` and
  `Q / E` move the camera here as they do in every other set (the camera is first, as it is
  everywhere in openMapper), so Blender's scale is on `Shift+S` and its select all is left to
  the menu; the mouse stays openMapper's own. Every key is changeable in View > Controls.

