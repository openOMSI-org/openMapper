# Street model: building OMSI roads from OpenStreetMap correctly

OpenStreetMap describes a street network for navigation, not geometry for a driving simulator. Taken
one way = one road, it gives broken streets, kinks, wobbling roads, paths running into each other and
objects at wrong angles. Typical defects in Moscow data (our reference area, 2018 and now):

- a divided street drawn as two one-way ways; parking aisles and drives as one-way `service` ways;
- sidewalks drawn as separate `footway` lines beside the road (mostly untagged), crossings as short footways
  through the carriageway, yard paths overlapping each other and the sidewalks;
- node positions with 1–5 m noise, extra nodes along straights, short zig-zags, sharp corners where the
  real kerb is a 10–20 m curve; lanes and widths rarely tagged; whole offsets of 5–10 m against imagery.

So OSM (and the 2018 imagery) are **evidence**, and the map is built from a **street model** that a road
designer would draw from that evidence. Every layer is deterministic and tested.

## Layers

**A. Normalise** (raw graph). Planarise ways at shared nodes; merge nodes closer than 1.5 m; drop pieces
shorter than 2 m; classify every way:
carriageway (`highway` road classes) · sidewalk (footway running beside a carriageway, 2–14 m off its axis
for most of its length) · crossing (footway crossing a carriageway) · yard path · parking aisle / drive.

**B. Streets.** Chain carriageway edges into streets through degree-2 nodes (same name/class). Pair one-way
ways running opposite within 6–30 m into one divided street (median = their distance). Cross-section per
street and side: lanes (tag, else class default, else imagery width), carriageway width, sidewalk width
(from the detected sidewalk lines, else class default). Sidewalk lines are absorbed into the street and are
not built as paths.

**C. Alignment.** Each street axis is fitted as straights and arcs (OMSI splines), tangent-continuous at every
joint, with the class's minimum radius, smoothing node noise (least squares within a tolerance); no corner is
ever left sharp, no wobble of tiny arcs. Heights from the ground, smoothed, with a maximum grade.

**D. Junctions** at street nodes of degree ≥ 3 only, with their approaches given explicitly from the graph
(never found by searching for road ends nearby): plate from the approaches' cross-sections and corner radii,
priority from class (stop lines only on minor approaches), zebras from crossings.

**E. Types.** The spline type for a street is chosen from its cross-section (catalogue of types with
measured profiles); where none fits, a type is made with the road maker.

**F. Build.** Splines, plates, paths, then objects bound to the street model: a stop, sign, lamp is placed at
(street, s, side, offset) and takes the street's tangent there, so it is always at the kerb and at the right
angle. Paths join the sidewalks at the kerb, never the carriageway.

**G. Check.** The map check reports what the model forbids: heading break or gap at a link, free road end
that is not a dead end, 2-arm plate, object in a carriageway, roadside object not aligned with its road, path
ending inside a road or on another path's middle. A build that fails the check is not accepted.

## Imagery

The 2018 imagery (chosen by capture date) decides existence (built by 2018 or not), widths and the street
axis where OSM is offset; features OSM lacks are traced from it into the same model.
