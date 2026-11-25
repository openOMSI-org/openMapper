# Optional placement watermarks

`tools/placement_watermark.py` gives each buyer's copy of a map its own object placements.

- **What moves.** Every eligible scenery object in the map's tile files moves horizontally
  by a fixed offset, 0.3 mm by default. The direction differs per object and per buyer.
- **Who can read it.** The directions come from the creator's secret key and an opaque
  buyer ID, so only someone holding that key and the unmarked original can tell which
  buyer a copy belongs to.
- **What it doesn't do.** The tool contacts no service, stores no buyer data and writes no
  ID into the files. It is optional and changes nothing for maps that don't use it.

```sh
python3 tools/placement_watermark.py keygen watermark.key
python3 tools/placement_watermark.py embed --key watermark.key --buyer lic-0001 \
    --content /path/to/content maps/MyMap /path/to/builds/lic-0001
python3 tools/placement_watermark.py inspect --key watermark.key --candidates buyer-ids.txt \
    --content /path/to/content maps/MyMap /path/to/suspect/MyMap
```

## What is marked

- **Marked:** `[object]` records in the map folder's top-level `tile_X_Y.map` files. Only
  the x and y lines change. Height, rotation, labels and every other byte stay as they
  were, including line endings.
- **Not marked:**
  - attached objects (`[attachObj]`), which hang on their parent's attachment points;
  - splines;
  - Chrono tiles;
  - every other file in the map folder, which is copied unchanged.
- **Never moved:**
  - objects whose `.sco` file has `[path]`, `[pathlink]`, `[splinehelper]`,
    `[crossingproblem]`, `[crossing_heightdeformation]`, `[entrypoint]`, `[switch]` or
    `[switchdir]`, because roads, rails and AI paths connect to them;
  - objects whose `.sco` file can't be found under `--content`.
- **Tile layouts:** the tool follows each tile's `[version]` layout, as the engine's tile
  reader does. Labels that look like record keywords are skipped by their count.
- **Offsets:** from 0.1 to 0.5 mm (`--offset-mm`). Use the same value for `embed` and
  `inspect`.

## How a copy is inspected

1. The suspect tile files are matched to the original by tile name, and their objects by
   ID and model. Tiles older than version 6 have no object IDs and are matched in file
   order.
2. Each object's movement from the original is measured in units of the offset. Values
   are capped at three offsets, so a few objects moved on purpose can't swamp the result.
3. That movement is correlated with each candidate buyer's directions.

The report gives, for each candidate:
- **Score:** about 1 for an exact copy, about 0.5 each for two buyers' copies averaged,
  and about 0 for anyone else.
- **Bound:** an upper bound on the chance that a wrong buyer scores as high.

For a wrong buyer the directions behave as independent fair signs, so the chance is at
most `exp(-z²/2)`, where `z` is the correlation divided by the square root of the total
squared movement. A candidate counts as a match when that bound is at most `1e-9`
(`--max-false-match`). With many candidates, multiply the bound by their number for the
chance of any false match. The command exits with 0 when at least one candidate matches.

## Measured by the tests

`tools/test_placement_watermark.py` uses a fixture of 10 tiles with 1,510 movable objects.

| Case | Result |
|---|---|
| Exact copy | Buyer found (score 1.0); 58 other IDs are not matched |
| Unmarked original | Nobody matched |
| Wrong key | Nobody matched |
| One tile in ten (151 objects) | Buyer found |
| Objects reordered, numbers rewritten to 6 decimals, CRLF line endings | Buyer found |
| 15 objects of one tile moved by 10 cm | Buyer found |
| Positions rounded to 1 mm | Buyer found (score about 0.97) |
| Two buyers' copies averaged, or tiles mixed from both | Both found; no one else |
| 10,000 wrong IDs against one tile | None matched |
| Random jitter of up to ±5 mm on every number | Not found (see limits) |
| Version 5 tiles without object IDs | Buyer found |

The tests also check that:
- the original folder is never changed, and the destination is never overwritten;
- a malformed tile, a symbolic link, a bad buyer ID, a bad key or an offset out of range
  leaves no partial output;
- the same buyer always gets the same copy, and different buyers get different copies.

## Limits

- **Deliberate jitter.** Someone who knows the marks exist can drown them by moving every
  object at random by a few millimetres, which is just as invisible. In the tests, jitter
  of ±5 mm hides the buyer on a 1,510-object map. Detection gets stronger with the square
  root of the number of objects. The tests' numbers suggest that roughly 10,000 movable
  objects are needed to survive ±5 mm jitter, and larger jitter needs more. That estimate
  has not been measured on a real map.
- **Renamed tiles, or a map moved as a whole.** These are not matched.
- **Re-saving in another editor.** This keeps the mark only if positions keep at least
  millimetre precision.
- **Proof.** A match is evidence for the creator to weigh, not proof of who shared a copy.
  Copies can be passed on.
- **Speed.** Inspection checks every candidate ID against every matched object, so very
  large candidate lists take longer.
- **What it can't replace.** This does not replace encryption, signatures or licences, and
  it does not stop copying. It only helps trace a copy after the fact.
