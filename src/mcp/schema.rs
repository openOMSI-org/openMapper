//! The tools the server offers, with what an agent needs to call them well.

use serde_json::{json, Value};

use super::Server;

/// The tools that change the map or write files (refused when the server is read-only).
pub const EDITS: &[&str] = &[
    "new_map", "set_editor", "click", "stroke", "command", "undo", "redo", "draw_road", "roundabout", "bus_pocket", "place_objects", "bus_stop", "start_point", "ai_path", "terrain_brush", "ramp", "water", "paint", "plant", "edit_object", "edit_spline", "delete", "tiles", "ground_textures", "make_line", "fix", "sample", "save", "export",
];

fn point() -> Value {
    json!({
        "description": "A point: [x, y] world metres (x east, y north), or {x, y}, or {lat, lon}, or {tile: [tx, ty], local: [x, y]}.",
        "anyOf": [
            { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 3 },
            { "type": "object", "properties": {
                "x": { "type": "number" }, "y": { "type": "number" },
                "lat": { "type": "number" }, "lon": { "type": "number" },
                "tile": { "type": "array", "items": { "type": "integer" } },
                "local": { "type": "array", "items": { "type": "number" } }
            } }
        ]
    })
}

fn points(what: &str) -> Value {
    json!({ "type": "array", "items": point(), "description": what })
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str], read_only: bool) -> Value {
    let mut schema = json!({ "type": "object", "properties": properties });
    if !required.is_empty() {
        schema["required"] = json!(required);
    }
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema,
        "annotations": { "readOnlyHint": read_only, "destructiveHint": false, "idempotentHint": read_only, "openWorldHint": false }
    })
}

impl Server {
    pub fn tools(&self) -> Vec<Value> {
        let p = point();
        let ids = json!({ "type": "array", "items": { "type": "integer" }, "description": "IDCodes of objects and splines." });
        let mut t = vec![
            // ------------------------------------------------------------------ seeing
            tool("map_info", "The open map: name, folder, tiles and bounds, counts (objects, splines, stops, start points, lines), ground textures, whether it is placed on the Earth, unsaved changes, undo / redo.", json!({}), &[], true),
            tool("list_maps", "The maps of the OMSI 2 installation and the content folder (their global.cfg paths), to open one.", json!({}), &[], true),
            tool("render", "A picture of the map from above (PNG): the ground shaded by height, water, roads and tracks at their widths, AI paths, objects, trees, stops with names, start points, a grid with coordinates, tile edges and the selection. Use it to SEE what is there and what an edit did. Give `center` + `radius`, or `min` + `max`, or nothing for the whole map. The text answer gives the metres per pixel and the corners.", json!({
                "center": p, "radius": { "type": "number", "description": "Metres from the centre to the picture's edge (150 by default)." },
                "min": p, "max": p,
                "size": { "type": "integer", "description": "The longer side in pixels, 768 by default (256 to 2048)." },
                "ids": { "type": "boolean", "description": "Write objects' and splines' IDCodes on them (when there are not too many)." },
                "lanes": { "type": "boolean", "description": "Show AI paths and technical splines (on by default)." },
                "grid": { "type": "boolean", "description": "Grid lines with coordinates (on by default)." }
            }), &[], true),
            tool("query", "What is near a point or in a box: objects, splines, bus stops and start points, nearest first, with IDCodes. Filter by file text.", json!({
                "at": p, "radius": { "type": "number", "description": "Metres, 50 by default." },
                "min": p, "max": p,
                "kinds": { "type": "array", "items": { "type": "string", "enum": ["objects", "splines", "stops", "entries"] } },
                "file": { "type": "string", "description": "Only those whose file contains this (case-insensitive)." },
                "limit": { "type": "integer", "description": "At most this many of each kind, 100 by default." },
                "full": { "type": "boolean", "description": "Each position in every form (tile, local, lat/lon) instead of [x, y]." }
            }), &[], true),
            tool("inspect", "Everything about objects and splines by IDCode: file, position, heading, height, labels (texts, stop settings), links of a spline chain, gradients, cant, radius, length, the catalogue's facts about the file.", json!({ "ids": ids }), &["ids"], true),
            tool("convert", "A point in every form: latitude / longitude, world metres, tile + local metres, the ground's height.", p.clone(), &[], true),
            tool("terrain", "The ground's height (m) at points, or evenly along a line (a profile), or a grid over a box (`min`, `max`, `step`).", json!({
                "points": points("Points."), "from": p, "to": p,
                "samples": { "type": "integer", "description": "Along the line: how many, 21 by default." },
                "min": p, "max": p, "step": { "type": "number", "description": "The grid's spacing (m), 10 by default." }
            }), &[], true),
            tool("catalogue", "Search the catalogue of the installation and the content folder: roads, tracks, footpaths, decoration (fences, walls, kerbs), technical splines (AI paths), objects, trees, stops and start points, junction pieces. Gives each file as the map names it, to build with. `on_map: true` lists the files the map uses already, most used first.", json!({
                "search": { "type": "string", "description": "Words that must all be in the name or folder." },
                "kind": { "type": "string", "enum": ["object", "spline"] },
                "category": { "type": "string", "enum": ["objects", "trees", "stops", "junctions", "roads", "tracks", "paths", "decoration", "technical"] },
                "on_map": { "type": "boolean" },
                "limit": { "type": "integer", "description": "50 by default." }, "offset": { "type": "integer" }
            }), &[], true),
            tool("check", "The map check as data: severity, kind, text, where, the object or spline, and whether `fix` can put it right.", json!({
                "severity": { "type": "string", "enum": ["error", "warning", "info"] },
                "limit": { "type": "integer", "description": "At most this many (all by default)." }
            }), &[], true),
            tool("timetable", "The bus lines: each line's trips (their stops by name and IDCode, terminus, length) and tours (departures), and the map's bus stops.", json!({
                "line": { "type": "string", "description": "Only this line." }
            }), &[], true),
            tool("editor_state", "The editor now: the tool and its mode and settings, the road / object type chosen, the selection, the undo and redo steps.", json!({}), &[], true),
            tool("script", "The map's Lua script (openOMSI plugin: events like on(\"stop_arrive\", fn), every(seconds, fn), omsi.* calls). Without `text`: read it and the templates. With `text`: check it as Lua 5.4 and (with --allow-edit) write it; `delete: true` removes it.", json!({
                "text": { "type": "string" }, "delete": { "type": "boolean" }
            }), &[], true),
            tool("reload", "Read the map from disk again (after another program changed it). Unsaved edits are dropped.", json!({}), &[], false),
            tool("open_map", "Open a map by its global.cfg (see list_maps). The open one is dropped (save it first).", json!({ "path": { "type": "string" } }), &["path"], false),
        ];
        if !self.allow_edit {
            for x in &mut t {
                if x["name"] == "script" {
                    x["description"] = json!("The map's Lua script and its templates; `text` is only checked (the server is read-only).");
                }
            }
            return t;
        }
        let road_opts = json!({
            "type": "object",
            "description": "The road tool's switches for this call: auto_junctions, auto_terrain (the ground follows the road), follow_ground, keep_ground_under_roads, auto_adapt (width adapters), auto_switches, snap_roads (join to ends and roads near), inherit_type (go on in the type of the road started from).",
            "additionalProperties": { "type": "boolean" }
        });
        t.extend([
            // ------------------------------------------------------------------ the editor's tools
            tool("set_editor", "Set the editor's tool and its settings, as a person does in the tool panel and catalogue, before `click` / `stroke`. Keys (all optional): tool (select|roads|objects|logic|terrain|paint|tiles), terrain_mode (raise|lower|smooth|flatten|roughen|ramp|water|along_spline), logic_mode (start|stops|parking|paths|other), water_shape (lake|river|brush), road_type, object_type, path_type (files from catalogue), brush_radius, brush_strength, flatten_height, paint_layer, paint_erase, water_depth, river_width, place_heading, align_to_road, road_setback, auto_junctions, auto_terrain, auto_adapt, auto_switches, follow_ground, keep_ground_under_roads, snap_roads, keep_drawing, inherit_type, roundabout, bus_pocket, pocket_len, pocket_width, plant_brush, plant_erase, plant_area, plant_spacing, plant_mix (list of tree files), stop_both_sides, parking_row, parking_step, snap_grid, grid_step, snap_angle, angle_step, keep_height, straight_lock, show_lanes.", json!({
                "settings": { "type": "object", "description": "Keys and values as listed." }
            }), &["settings"], false),
            tool("click", "Left clicks at points with the tool as set (a road's points, objects placed, the corners of a lake or a plant area...), then optionally `finish`: \"confirm\" (Enter: finish a polygon / road) or \"cancel\" (a right click: stop drawing).", json!({
                "points": points("Where to click, in order."), "shift": { "type": "boolean" },
                "finish": { "type": "string", "enum": ["confirm", "cancel", "none"] }
            }), &["points"], false),
            tool("stroke", "The left button held down along a path for some seconds, with the tool as set: a terrain or paint brush, the plant brush, a drag.", json!({
                "path": points("The cursor's way (one point: held still)."), "seconds": { "type": "number", "description": "How long, 1 by default; the brush works as long as it is held." }
            }), &["path"], false),
            tool("command", "One of the editor's commands (menus, keys) on the selection or the map: undo, redo, cancel, confirm, delete, copy, cut, paste, duplicate, select_all, select_none, rotate {degrees}, move {dx, dy}, raise {dz}, reverse, lay_on_ground, even_chain, continue_spline, parallel_copy {distance}, level_under_objects, align_to_roads, turn_objects {random}, lean_objects {lean}, replace_objects {file}, retype_splines {file}, replace_everywhere {from, to, splines}, cut_spline {id, at}, join_with_next {id}, wire_along {file}, paint_along {layer, width, under}, array {count, step:[dx,dy], turn, rows, row_step:[dx,dy]}, group, ungroup, remake_junctions, flatten_terrain, grow_map, align_ground, place_rows, detach_lanes.", json!({
                "name": { "type": "string" }, "args": { "type": "object" }
            }), &["name"], false),
            tool("select", "Select objects and splines: by `ids`, or what is within `radius` of `at` (kinds, file filter), or everything like an IDCode (`alike`), or a spline's whole chain (`chain`), or `all` / `none`. `add` keeps what is selected. The selection is what `command`, `delete` and `render` act on or show.", json!({
                "ids": ids, "at": p, "radius": { "type": "number" },
                "kinds": { "type": "array", "items": { "type": "string", "enum": ["objects", "splines"] } },
                "file": { "type": "string" }, "alike": { "type": "integer" }, "chain": { "type": "integer" },
                "all": { "type": "boolean" }, "none": { "type": "boolean" }, "add": { "type": "boolean" }
            }), &[], false),
            tool("undo", "Undo the last step(s), as Ctrl+Z.", json!({ "steps": { "type": "integer" } }), &[], false),
            tool("redo", "Redo, as Ctrl+Y.", json!({ "steps": { "type": "integer" } }), &[], false),
            // ------------------------------------------------------------------ in one call
            tool("draw_road", "Draw a road (or track, footpath, wall, fence, any spline type) through points with the road tool: it curves smoothly from point to point, joins to road ends and roads it meets, and makes junctions where roads cross or branch, as for a person. `closed` comes back to the first point; `separate` makes each pair of points its own road.", json!({
                "points": points("Two or more points."), "type": { "type": "string", "description": "The spline file (catalogue); the road type chosen before when left out." },
                "closed": { "type": "boolean" }, "separate": { "type": "boolean" }, "options": road_opts
            }), &["points"], false),
            tool("roundabout", "A roundabout of a road type: the ring, the roads inside cut back to it, junctions where they meet it. Best with a one-way road type.", json!({
                "center": p, "radius": { "type": "number", "description": "Metres to the ring's middle line (at least 10)." }, "type": { "type": "string" }
            }), &["center", "radius"], false),
            tool("bus_pocket", "A bus bay on a road at a point: the road widened there, with tapers.", json!({
                "at": p, "length": { "type": "number", "description": "Full-width length (m)." }, "width": { "type": "number", "description": "How much wider (m), 3 by default." }, "type": { "type": "string", "description": "The pocket's road type (default: found)." }
            }), &["at"], false),
            tool("place_objects", "Place objects (buildings, lamps, signs, benches, trees...) in one call, each standing on the ground. Each item: file, at, heading (deg, clockwise from north), height (m above the ground), align_to_road (turn to face the nearest road and stand at its edge), labels (its text strings). Up to 5000.", json!({
                "items": { "type": "array", "items": { "type": "object", "properties": {
                    "file": { "type": "string" }, "at": p, "heading": { "type": "number" }, "height": { "type": "number" },
                    "align_to_road": { "type": "boolean" }, "labels": { "type": "array", "items": { "type": "string" } }
                }, "required": ["file", "at"] } },
                "file": { "type": "string", "description": "The file for items that name none." }
            }), &["items"], false),
            tool("bus_stop", "A bus stop by a road, at the kerb facing along the lane, and (by default) its pair on the other side; named. The road must be there first.", json!({
                "at": p, "name": { "type": "string" }, "both_sides": { "type": "boolean" },
                "kind": { "type": "string", "enum": ["regular", "request", "terminus"] }, "file": { "type": "string", "description": "The stop object (default: the catalogue's first)." }
            }), &["at", "name"], false),
            tool("start_point", "A start point (where the bus appears when a drive begins), facing `heading`; on a lane by default it is found on the road nearest.", json!({
                "at": p, "heading": { "type": "number" }, "name": { "type": "string" }
            }), &["at"], false),
            tool("ai_path", "An AI path (lanes for cars or people the road types lack: a car park, a depot, a turn) drawn through points; it snaps to lane ends. A car path runs the way it is drawn.", json!({
                "points": points("Two or more points."), "type": { "type": "string", "description": "A technical spline with lanes (default: an invisible street)." }
            }), &["points"], false),
            tool("terrain_brush", "Shape the ground with the brush along a path (or at one point): raise, lower, smooth, flatten (to `height`, or the height where it starts), roughen.", json!({
                "mode": { "type": "string", "enum": ["raise", "lower", "smooth", "flatten", "roughen"] },
                "path": points("Where the brush goes (one point: held there)."), "radius": { "type": "number" }, "strength": { "type": "number" },
                "seconds": { "type": "number", "description": "How long it is held, 1 by default: longer is higher / deeper." }, "height": { "type": "number", "description": "For flatten." }
            }), &["mode", "path"], false),
            tool("ramp", "An even slope of the ground from one point to another, as wide as `width` (m).", json!({ "from": p, "to": p, "width": { "type": "number" } }), &["from", "to"], false),
            tool("water", "A lake (its shore round, `shape: lake`) or a river (its middle line from the source, `shape: river`, `width`): the ground dug to a bed `depth` under the water, the water laid.", json!({
                "shape": { "type": "string", "enum": ["lake", "river"] }, "points": points("The shore, or the river's line."),
                "depth": { "type": "number" }, "width": { "type": "number" }
            }), &["shape", "points"], false),
            tool("paint", "Paint the ground with a texture along a path with the brush (or erase it). `texture` adds the ground texture to the map when it lacks it; or name the `layer`.", json!({
                "path": points("Where the brush goes."), "layer": { "type": "integer" }, "texture": { "type": "string" },
                "radius": { "type": "number" }, "strength": { "type": "number" }, "seconds": { "type": "number" }, "erase": { "type": "boolean" }
            }), &["path"], false),
            tool("plant", "Plant trees: in an area (its outline, `area`), or along a path with the brush; a mix of tree files (default: the catalogue's first trees), `spacing` metres apart. `erase` clears plants along the path.", json!({
                "area": points("The area's outline."), "path": points("The brush's way."), "mix": { "type": "array", "items": { "type": "string" } },
                "spacing": { "type": "number" }, "radius": { "type": "number" }, "seconds": { "type": "number" }, "erase": { "type": "boolean" }
            }), &[], false),
            tool("edit_object", "Change an object: where it stands, heading, height above the ground, its file, its text labels, and for a bus stop its name and kind.", json!({
                "id": { "type": "integer" }, "at": p, "heading": { "type": "number" }, "height": { "type": "number" }, "file": { "type": "string" },
                "labels": { "type": "array", "items": { "type": "string" } }, "stop_name": { "type": "string" }, "stop_kind": { "type": "string", "enum": ["regular", "request", "terminus"] }
            }), &["id"], false),
            tool("edit_spline", "Change a spline: its type (file), the height or gradient (%) of its start or end, cant, or cut it at a distance along it.", json!({
                "id": { "type": "integer" }, "file": { "type": "string" },
                "start_height": { "type": "number" }, "end_height": { "type": "number" }, "start_grade": { "type": "number" }, "end_grade": { "type": "number" },
                "start_cant": { "type": "number" }, "end_cant": { "type": "number" }, "cut_at": { "type": "number" }
            }), &["id"], false),
            tool("delete", "Delete objects and splines by IDCode (junctions and links are put right as for a person).", json!({ "ids": ids }), &["ids"], false),
            tool("tiles", "Add tiles ([[tx, ty], ...], next to the map), remove tiles, set a tile's water level (null: none), or grow the map by a ring of tiles.", json!({
                "add": { "type": "array", "items": { "type": "array", "items": { "type": "integer" } } },
                "remove": { "type": "array", "items": { "type": "array", "items": { "type": "integer" } } },
                "water": { "type": "array", "items": { "type": "object", "properties": { "tile": { "type": "array", "items": { "type": "integer" } }, "level": { "type": ["number", "null"] } } } },
                "grow": { "type": "boolean" }
            }), &[], false),
            tool("ground_textures", "The map's ground textures (the paint layers): list them, or add one (a texture file, e.g. Texture\\\\...bmp/.dds).", json!({ "add": { "type": "string" } }), &[], false),
            tool("make_line", "A bus line in one call: the trip from the first stop to the last through the stops given (IDCodes, in order; see query / timetable), its way found on the lanes, the trip back (through `back`, or the same stops' pairs), and a tour running all day from `first` to `last` departure every `every` minutes; a depot and cars as needed. The map then drives in service.", json!({
                "name": { "type": "string", "description": "The line's number or name." },
                "stops": ids, "back": ids,
                "first": { "type": "string", "description": "First departure, \"06:00\"." }, "last": { "type": "string", "description": "Last departure, \"22:00\"." },
                "every": { "type": "number", "description": "Minutes between departures (default: as soon as the bus is back)." },
                "speed": { "type": "number", "description": "Mean speed (km/h) the times are reckoned with, 22 by default." }
            }), &["name", "stops"], false),
            tool("fix", "Put right what the check found that one click fixes: by `kinds` (from check), or all that can be fixed.", json!({
                "kinds": { "type": "array", "items": { "type": "string" } }, "limit": { "type": "integer" }
            }), &[], false),
            tool("sample", "The sample on the map: a ring road round its middle with stops, a start point and line 1 with a timetable (the quickest map that can be driven in service).", json!({ "center": p }), &[], false),
            tool("batch", "Many tool calls in one request (much quicker than one by one), in order; all of them are one undo step. Each: {tool, arguments}. Stops at the first error unless stop_on_error is false.", json!({
                "calls": { "type": "array", "items": { "type": "object", "properties": { "tool": { "type": "string" }, "arguments": { "type": "object" } }, "required": ["tool"] } },
                "label": { "type": "string" }, "stop_on_error": { "type": "boolean" }
            }), &["calls"], false),
            tool("new_map", "Make a new map in the content folder (and open it): folder name, the name shown, size in tiles (side of a square, 1 to 10), the ground's height, left-hand traffic.", json!({
                "folder": { "type": "string" }, "name": { "type": "string" }, "size": { "type": "integer" },
                "height": { "type": "number" }, "left_hand": { "type": "boolean" }, "description": { "type": "string" }
            }), &["folder"], false),
            tool("save", "Save the map into its folder (the replaced files are kept in a backup, as for every save).", json!({}), &[], false),
            tool("export", "Save the map and pack it with every file it needs that is not in OMSI 2 into a .zip to share.", json!({ "path": { "type": "string", "description": "Where the .zip goes." } }), &["path"], false),
        ]);
        t
    }
}
