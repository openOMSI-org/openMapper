#!/usr/bin/env python3
"""Add or inspect an opaque per-buyer watermark in the object placements of an OMSI map.

Each eligible free-standing scenery object in the map's tile files moves horizontally by a
fixed sub-millimetre offset. The direction of each move comes from a keyed pseudo-random
function of the creator's secret key, the buyer's opaque ID and the object's identity, so
every buyer's copy differs, and only the key holder with the unmarked original can tell
which buyer a copy belongs to. See docs/PLACEMENT_WATERMARKS.md.
"""

import argparse
import hmac
import json
import math
import os
import re
import secrets
import shutil
import stat
import tempfile
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
from pathlib import Path


KEY_BYTES = 32
DEFAULT_OFFSET_MM = Decimal("0.3")
MIN_OFFSET_MM = Decimal("0.1")
MAX_OFFSET_MM = Decimal("0.5")
MAX_TILE_BYTES = 64 * 1024 * 1024
MAX_SCO_BYTES = 16 * 1024 * 1024
MAX_LIST_BYTES = 64 * 1024 * 1024
MAX_MAP_FILES = 200_000
MAX_CANDIDATES = 1_000_000
MAX_LABELS = 4096
MAX_NUMBER_TOKEN = 64
RESIDUAL_LIMIT = 3.0
DEFAULT_MAX_FALSE_MATCH = 1e-9
DOMAIN = b"openMapper placement watermark v1\0"
_BUYER = re.compile(r"[A-Za-z0-9._-]{1,128}")
_TILE = re.compile(r"tile_-?\d+_-?\d+\.map", re.IGNORECASE)
_INTEGER = re.compile(rb"-?\d{1,9}")
# .sco sections of objects that roads, rails or AI paths connect to. Such objects never move.
FIXED_SECTIONS = frozenset(
    b"[" + name + b"]"
    for name in (b"path", b"pathlink", b"splinehelper", b"crossingproblem",
                 b"crossing_heightdeformation", b"entrypoint", b"switch", b"switchdir")
)


@dataclass(frozen=True)
class Placement:
    """One `[object]` record: its identity, model, and where its x and y values are."""

    ident: bytes
    model: bytes
    lines: tuple[int, int]
    position: tuple[Decimal, Decimal]


def _has(version: int, minimum: int) -> bool:
    # Tiles written before `[version]` existed read as the newest layout (omsi-map's `has`).
    return version == 0 or version >= minimum


def _number(raw: bytes, where: str) -> Decimal:
    token = raw.strip()
    if not token or len(token) > MAX_NUMBER_TOKEN:
        raise ValueError(f"{where}: missing or oversized number")
    try:
        value = Decimal(token.decode("ascii"))
    except (InvalidOperation, UnicodeDecodeError) as error:
        raise ValueError(f"{where}: invalid number") from error
    if not value.is_finite():
        raise ValueError(f"{where}: non-finite number")
    return value


def _integer(raw: bytes, where: str) -> int:
    token = raw.strip()
    if not _INTEGER.fullmatch(token):
        raise ValueError(f"{where}: invalid integer")
    return int(token)


def _model_key(model: bytes) -> bytes:
    return model.strip().replace(b"/", b"\\").lower()


def parse_tile(data: bytes, name: str) -> tuple[list[bytes], list[Placement]]:
    """Find the `[object]` records of a tile, following the layout its `[version]` sets."""
    lines = data.splitlines(keepends=True)
    tile = name.lower().encode("ascii")
    placements: list[Placement] = []
    version = 0
    ordinal = 0
    i = 0

    def line(index: int) -> bytes:
        if index >= len(lines):
            raise ValueError(f"{name}: record cut short at line {index + 1}")
        return lines[index]

    while i < len(lines):
        keyword = lines[i].strip().lower()
        if keyword == b"[version]":
            version = _integer(line(i + 1), f"{name} line {i + 2}")
            i += 2
            continue
        if keyword not in (b"[object]", b"[attachobj]"):
            i += 1
            continue
        attach = keyword == b"[attachobj]"
        j = i + 1
        if _has(version, 9):
            j += 1  # detail level
        model = line(j).strip()
        j += 1
        if _has(version, 6):
            ident = line(j).strip()
            j += 1
        else:
            ident = b"#%d" % ordinal
        if attach:
            j += 3  # parent, parent instance, attachment point
            if _has(version, 8):
                j += 1  # heading
        else:
            at = (j, j + 1)
            x = _number(line(j), f"{name} line {j + 1}")
            y = _number(line(j + 1), f"{name} line {j + 2}")
            line(j + 3)
            j += 4  # x, y, height, heading
            ordinal += 1
        if _has(version, 12):
            j += 2  # pitch, bank
        if _has(version, 4):
            count = _integer(line(j), f"{name} line {j + 1}")
            if not 0 <= count <= MAX_LABELS:
                raise ValueError(f"{name} line {j + 1}: label count outside 0 to {MAX_LABELS}")
            j += 1 + count
        if not attach:
            placements.append(Placement(tile + b"\0" + ident + b"\0" + _model_key(model), model, at, (x, y)))
        i = min(j, len(lines))
    return lines, placements


class _Models:
    """Decide which object models may move, from their .sco files under a content folder."""

    def __init__(self, content: Path):
        self.content = Path(content)
        if not self.content.is_dir():
            raise NotADirectoryError(self.content)
        self.movable_cache: dict[bytes, bool] = {}
        self.listings: dict[Path, dict[str, Path]] = {}

    def movable(self, model: bytes) -> bool:
        key = _model_key(model)
        if key not in self.movable_cache:
            self.movable_cache[key] = self._check(key)
        return self.movable_cache[key]

    def _check(self, key: bytes) -> bool:
        path = self._resolve(key)
        if path is None:
            return False  # unknown model: leave it where it is
        with path.open("rb") as source:
            data = source.read(MAX_SCO_BYTES + 1)
        if len(data) > MAX_SCO_BYTES:
            return False
        return not any(line.strip().lower() in FIXED_SECTIONS for line in data.splitlines())

    def _resolve(self, key: bytes):
        try:
            parts = key.decode("cp1252").split("\\")
        except UnicodeDecodeError:
            return None
        if any(part in ("", ".", "..") for part in parts):
            return None
        current = self.content
        for part in parts:
            listing = self.listings.get(current)
            if listing is None:
                try:
                    listing = {child.name.lower(): child for child in current.iterdir()}
                except OSError:
                    return None
                self.listings[current] = listing
            child = listing.get(part)
            if child is None or child.is_symlink():
                return None
            current = child
        return current if current.is_file() else None


def _read_bounded(path: Path, limit: int) -> bytes:
    with Path(path).open("rb") as source:
        data = source.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"{path.name} exceeds {limit} bytes")
    return data


def _read_key(path) -> bytes:
    key = _read_bounded(Path(path), KEY_BYTES)
    if len(key) != KEY_BYTES:
        raise ValueError(f"watermark key must be exactly {KEY_BYTES} bytes")
    return key


def _buyer_key(key: bytes, buyer: str) -> bytes:
    return hmac.digest(key, DOMAIN + b"buyer\0" + buyer.encode("ascii"), "sha256")


def _chips(buyer_key: bytes, ident: bytes) -> tuple[int, int]:
    digest = hmac.digest(buyer_key, DOMAIN + b"chip\0" + ident, "sha256")
    return (1 if digest[0] & 1 else -1, 1 if digest[1] & 1 else -1)


def _check_buyer(buyer) -> str:
    if not isinstance(buyer, str) or not _BUYER.fullmatch(buyer):
        raise ValueError("buyer ID must be 1 to 128 letters, digits, dots, dashes or underscores")
    return buyer


def _offset(offset_mm) -> Decimal:
    value = Decimal(str(offset_mm))
    if not (value.is_finite() and MIN_OFFSET_MM <= value <= MAX_OFFSET_MM):
        raise ValueError(f"offset must be between {MIN_OFFSET_MM} and {MAX_OFFSET_MM} mm")
    return value / 1000


def _map_files(master: Path) -> list[Path]:
    files: list[Path] = []
    for directory, subdirectories, names in os.walk(master):
        for name in subdirectories + names:
            if (Path(directory) / name).is_symlink():
                raise ValueError(f"symbolic link in map folder: {Path(directory, name).relative_to(master)}")
        files += [Path(directory, name).relative_to(master) for name in names]
        if len(files) > MAX_MAP_FILES:
            raise ValueError(f"map folder holds more than {MAX_MAP_FILES} files")
    return sorted(files)


def _tiles(folder: Path) -> dict[str, Path]:
    return {
        path.name.lower(): path
        for path in Path(folder).iterdir()
        if _TILE.fullmatch(path.name) and path.is_file() and not path.is_symlink()
    }


def keygen(path) -> None:
    """Create a new random watermark key file, readable only by its owner."""
    path = Path(path)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0), 0o600)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(secrets.token_bytes(KEY_BYTES))
            output.flush()
            os.fsync(output.fileno())
    except BaseException:
        path.unlink(missing_ok=True)
        raise


def embed(master, content, key_path, buyer: str, destination, offset_mm=DEFAULT_OFFSET_MM) -> dict:
    """Write a marked copy of a map folder for one buyer. Neither input is changed, and the
    destination only appears, complete, once everything has been written."""
    buyer = _check_buyer(buyer)
    offset = _offset(offset_mm)
    key = _read_key(key_path)
    master = Path(master)
    destination = Path(destination)
    if destination.exists() or destination.is_symlink():
        raise FileExistsError(destination)
    if not master.is_dir():
        raise NotADirectoryError(master)
    inside = destination.resolve(strict=False)
    if inside == master.resolve() or master.resolve() in inside.parents:
        raise ValueError("destination must be outside the map folder")
    if not destination.parent.is_dir():
        raise FileNotFoundError(destination.parent)
    files = _map_files(master)
    models = _Models(Path(content))
    buyer_key = _buyer_key(key, buyer)
    marked = unmoved = tiles = 0
    temporary = Path(tempfile.mkdtemp(prefix=".placement-watermark-", dir=destination.parent))
    try:
        for relative in files:
            source = master / relative
            target = temporary / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            if len(relative.parts) != 1 or not _TILE.fullmatch(relative.name):
                shutil.copy2(source, target)
                continue
            lines, placements = parse_tile(_read_bounded(source, MAX_TILE_BYTES), relative.name)
            for placement in placements:
                if not models.movable(placement.model):
                    unmoved += 1
                    continue
                for at, value, chip in zip(placement.lines, placement.position, _chips(buyer_key, placement.ident)):
                    ending = lines[at][len(lines[at].rstrip(b"\r\n")):]
                    lines[at] = format(value + chip * offset, "f").encode("ascii") + ending
                marked += 1
            target.write_bytes(b"".join(lines))
            shutil.copystat(source, target)
            tiles += 1
        if destination.exists() or destination.is_symlink():
            raise FileExistsError(destination)
        os.rename(temporary, destination)
    except BaseException:
        shutil.rmtree(temporary, ignore_errors=True)
        raise
    return {"buyer": buyer, "tiles": tiles, "marked_objects": marked, "unmoved_objects": unmoved,
            "offset_mm": format(offset * 1000, "f")}


def inspect(master, content, key_path, candidates, suspect, offset_mm=DEFAULT_OFFSET_MM,
            max_false_match=DEFAULT_MAX_FALSE_MATCH) -> dict:
    """Compare a suspect copy with the unmarked master and score each candidate buyer ID."""
    offset = _offset(offset_mm)
    key = _read_key(key_path)
    ids = list(dict.fromkeys(_check_buyer(candidate) for candidate in candidates))
    if not ids or len(ids) > MAX_CANDIDATES:
        raise ValueError(f"give 1 to {MAX_CANDIDATES} candidate buyer IDs")
    if not 0 < float(max_false_match) < 1:
        raise ValueError("maximum false-match probability must be between 0 and 1")
    models = _Models(Path(content))
    suspect_tiles = _tiles(Path(suspect))
    residuals: list[tuple[bytes, float, float]] = []
    skipped = 0
    for name, path in sorted(_tiles(Path(master)).items()):
        other = suspect_tiles.get(name)
        if other is None:
            continue
        _, originals = parse_tile(_read_bounded(path, MAX_TILE_BYTES), path.name)
        try:
            _, copies = parse_tile(_read_bounded(other, MAX_TILE_BYTES), path.name)
        except ValueError:
            skipped += 1
            continue
        found: dict[bytes, Placement] = {}
        for placement in copies:
            found.setdefault(placement.ident, placement)
        for original in originals:
            copy = found.get(original.ident)
            if copy is None or not models.movable(original.model):
                continue
            rx, ry = (
                max(-RESIDUAL_LIMIT, min(RESIDUAL_LIMIT, float((after - before) / offset)))
                for before, after in zip(original.position, copy.position)
            )
            residuals.append((original.ident, rx, ry))
    energy = sum(rx * rx + ry * ry for _, rx, ry in residuals)
    chips = 2 * len(residuals)
    rows = []
    for buyer in ids:
        buyer_key = _buyer_key(key, buyer)
        correlation = 0.0
        for ident, rx, ry in residuals:
            cx, cy = _chips(buyer_key, ident)
            correlation += cx * rx + cy * ry
        # For a wrong buyer the chips are independent fair signs, so P(sum >= t) <= exp(-t^2 / 2E).
        z = correlation / math.sqrt(energy) if energy > 0 and correlation > 0 else 0.0
        bound = math.exp(-z * z / 2) if z > 0 else 1.0
        rows.append({"id": buyer, "score": round(correlation / chips, 6) if chips else 0.0,
                     "z": round(z, 3), "false_match_bound": bound, "match": bound <= max_false_match})
    rows.sort(key=lambda row: -row["score"])
    return {
        "matched_objects": len(residuals),
        "chips": chips,
        "mark_energy": round(energy / chips, 6) if chips else 0.0,
        "skipped_tiles": skipped,
        "max_false_match": max_false_match,
        "matches": [row["id"] for row in rows if row["match"]],
        "candidates": rows,
    }


def _candidate_list(path: Path) -> list[str]:
    text = _read_bounded(path, MAX_LIST_BYTES).decode("utf-8")
    return [line.strip() for line in text.splitlines() if line.strip() and not line.lstrip().startswith("#")]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    keygen_command = commands.add_parser("keygen", help="create a new secret watermark key")
    keygen_command.add_argument("key", type=Path)
    for name in ("embed", "inspect"):
        command = commands.add_parser(name)
        command.add_argument("--key", type=Path, required=True, help="the creator's secret watermark key")
        command.add_argument("--content", type=Path, required=True, help="content folder holding the objects' .sco files")
        command.add_argument("--offset-mm", type=Decimal, default=DEFAULT_OFFSET_MM)
        if name == "embed":
            command.add_argument("--buyer", required=True, help="opaque buyer or licence ID")
        else:
            command.add_argument("--candidates", type=Path, required=True, help="file with one candidate buyer ID per line")
            command.add_argument("--max-false-match", type=float, default=DEFAULT_MAX_FALSE_MATCH)
        command.add_argument("master", type=Path, help="the unmarked original map folder")
        command.add_argument("destination" if name == "embed" else "suspect", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "keygen":
            keygen(args.key)
            print("Watermark key written. Keep it secret and back it up offline.")
            return 0
        if args.command == "embed":
            report = embed(args.master, args.content, args.key, args.buyer, args.destination, args.offset_mm)
            print(json.dumps(report, indent=1))
            return 0
        report = inspect(args.master, args.content, args.key, _candidate_list(args.candidates), args.suspect,
                         args.offset_mm, args.max_false_match)
        print(json.dumps(report, indent=1))
        return 0 if report["matches"] else 1
    except (OSError, ValueError, InvalidOperation, UnicodeDecodeError) as error:
        parser.error(str(error))
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
