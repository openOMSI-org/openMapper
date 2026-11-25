#!/usr/bin/env python3
import hashlib
import json
import random
import subprocess
import sys
import tempfile
import unittest
from decimal import Decimal
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import placement_watermark as pw

TILES = 10
OBJECTS_PER_TILE = 150
BUYER = "lic-0001"
OTHER = "lic-0002"


def _object(version: int, file: str, ident: int, x: Decimal, y: Decimal, labels=()) -> str:
    lines = ["[object]"]
    if version >= 9:
        lines.append("0")
    lines.append(file)
    if version >= 6:
        lines.append(str(ident))
    lines += [str(x), str(y), "0.25", "90"]
    if version >= 12:
        lines += ["0", "0"]
    if version >= 4:
        lines.append(str(len(labels)))
        lines += list(labels)
    return "\n".join(lines) + "\n\n"


def _attachment(version: int, ident: int, parent: int) -> str:
    # An attached object with a label that looks like a record keyword.
    lines = ["[attachObj]"]
    if version >= 9:
        lines.append("0")
    lines.append("Sceneryobjects\\Fixture\\plate.sco")
    if version >= 6:
        lines.append(str(ident))
    lines += [str(parent) if version >= 10 else "0", "0", "0"]
    if version >= 8:
        lines.append("0")
    if version >= 12:
        lines += ["0", "0"]
    if version >= 4:
        lines += ["1", "[object]"]
    return "\n".join(lines) + "\n\n"


def _tile_text(index: int, version: int = 14, objects: int = OBJECTS_PER_TILE) -> str:
    rng = random.Random(index)
    parts = [f"[version]\n{version}\n\n"]
    for n in range(objects):
        x = Decimal(rng.randrange(0, 30000000)) / Decimal(100000)
        y = Decimal(rng.randrange(0, 30000000)) / Decimal(100000)
        parts.append(_object(version, "Sceneryobjects\\Fixture\\tree.sco", 1000 * index + n, x, y))
    # A junction object roads connect to: must never move.
    parts.append(_object(version, "Sceneryobjects\\Fixture\\crossing.sco", 1000 * index + 900, Decimal("10.5"), Decimal("20.5")))
    # An object whose model is missing: left untouched.
    parts.append(_object(version, "Sceneryobjects\\Fixture\\missing.sco", 1000 * index + 901, Decimal("11.5"), Decimal("21.5")))
    # Labels that look like records must not be read as records.
    parts.append(_object(version, "Sceneryobjects\\Fixture\\sign.sco", 1000 * index + 902, Decimal("12.5"), Decimal("22.5"),
                         labels=("[object]", "0", "Sceneryobjects\\Fixture\\tree.sco", "7")))
    parts.append(_attachment(version, 1000 * index + 903, 1000 * index))
    parts.append("[spline]\n0\nSplines\\Fixture\\road.sli\n" f"{1000 * index + 904}\n0\n0\n1.5\n0\n2.5\n90\n50\n0\n0\n0\n0\n0\n0\n0\n0\n\n\n")
    return "".join(parts)


class PlacementWatermarkTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="openmapper-placement-")
        self.root = Path(self.temp.name)
        self.content = self.root / "content"
        scenery = self.content / "Sceneryobjects" / "Fixture"
        scenery.mkdir(parents=True)
        (scenery / "tree.sco").write_text("[mesh]\ntree.o3d\n", encoding="utf-8")
        (scenery / "sign.sco").write_text("[mesh]\nsign.o3d\n", encoding="utf-8")
        (scenery / "plate.sco").write_text("[mesh]\nplate.o3d\n", encoding="utf-8")
        (scenery / "crossing.sco").write_text("[mesh]\nroad.o3d\n\n[path]\n0\n0\n0\n0\n10\n", encoding="utf-8")
        self.master = self.content / "maps" / "Fixture"
        self.master.mkdir(parents=True)
        (self.master / "global.cfg").write_text("[name]\nFixture\n", encoding="utf-8")
        (self.master / "Chrono").mkdir()
        (self.master / "Chrono" / "tile_0_0.map").write_text(_tile_text(99), encoding="utf-8")
        for index in range(TILES):
            (self.master / f"tile_{index}_0.map").write_text(_tile_text(index), encoding="utf-8")
        self.key = self.root / "watermark.key"
        pw.keygen(self.key)
        self.candidates = [BUYER, OTHER] + [f"lic-{n:04d}" for n in range(3, 60)]

    def tearDown(self):
        self.temp.cleanup()

    def _embed(self, buyer: str, name: str) -> Path:
        destination = self.root / name
        pw.embed(self.master, self.content, self.key, buyer, destination)
        return destination

    def _inspect(self, suspect: Path, candidates=None):
        return pw.inspect(self.master, self.content, self.key, candidates or self.candidates, suspect)

    def _scores(self, report):
        return {row["id"]: row for row in report["candidates"]}

    def test_marked_copy_names_its_buyer_and_no_one_else(self):
        copy = self._embed(BUYER, "buyer")
        report = self._inspect(copy)
        scores = self._scores(report)
        self.assertTrue(scores[BUYER]["match"])
        self.assertGreater(scores[BUYER]["score"], 0.99)
        self.assertEqual(report["matches"], [BUYER])
        for other, row in scores.items():
            if other != BUYER:
                self.assertFalse(row["match"], other)
                self.assertLess(abs(row["score"]), 0.15, other)
        self.assertEqual(report["matched_objects"], TILES * (OBJECTS_PER_TILE + 1))

    def test_only_the_horizontal_position_of_eligible_objects_changes(self):
        copy = self._embed(BUYER, "buyer")
        offset = Decimal("0.0003")
        for index in range(TILES):
            before = (self.master / f"tile_{index}_0.map").read_text(encoding="utf-8").splitlines()
            after = (copy / f"tile_{index}_0.map").read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(before), len(after))
            changed = [(a, b) for a, b in zip(before, after) if a != b]
            self.assertEqual(len(changed), 2 * (OBJECTS_PER_TILE + 1))
            for old, new in changed:
                self.assertEqual(abs(Decimal(new) - Decimal(old)), offset)
            text = "\n".join(after)
            self.assertIn("crossing.sco\n" f"{1000 * index + 900}\n10.5\n20.5\n", text)
            self.assertIn("missing.sco\n" f"{1000 * index + 901}\n11.5\n21.5\n", text)
        for name in ("global.cfg", "Chrono/tile_0_0.map"):
            self.assertEqual((self.master / name).read_bytes(), (copy / name).read_bytes())

    def test_master_is_unchanged_and_copies_differ_per_buyer(self):
        digest = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in self.master.glob("tile_*.map")}
        first = self._embed(BUYER, "first")
        second = self._embed(OTHER, "second")
        again = self._embed(BUYER, "again")
        self.assertEqual(digest, {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in self.master.glob("tile_*.map")})
        self.assertNotEqual((first / "tile_0_0.map").read_bytes(), (second / "tile_0_0.map").read_bytes())
        self.assertEqual((first / "tile_0_0.map").read_bytes(), (again / "tile_0_0.map").read_bytes())

    def test_unmarked_copy_and_wrong_key_find_no_buyer(self):
        report = self._inspect(self.master)
        self.assertEqual(report["matches"], [])
        self.assertEqual(report["mark_energy"], 0.0)
        copy = self._embed(BUYER, "buyer")
        other_key = self.root / "other.key"
        pw.keygen(other_key)
        report = pw.inspect(self.master, self.content, other_key, self.candidates, copy)
        self.assertEqual(report["matches"], [])

    def test_one_tile_in_ten_is_enough(self):
        copy = self._embed(BUYER, "buyer")
        partial = self.root / "partial"
        partial.mkdir()
        (partial / "tile_3_0.map").write_bytes((copy / "tile_3_0.map").read_bytes())
        report = self._inspect(partial)
        self.assertEqual(report["matches"], [BUYER])
        self.assertEqual(report["matched_objects"], OBJECTS_PER_TILE + 1)

    def test_reordered_reformatted_and_crlf_copy_still_matches(self):
        copy = self._embed(BUYER, "buyer")
        altered = self.root / "altered"
        altered.mkdir()
        for path in copy.glob("tile_*.map"):
            records = path.read_text(encoding="utf-8").split("\n\n")
            header, body = records[0], records[1:]
            random.Random(path.name).shuffle(body)
            text = "\n\n".join([header] + body)
            lines = []
            for line in text.split("\n"):
                try:
                    value = Decimal(line)
                    lines.append(f"{value:.6f}" if "." in line else line)
                except Exception:
                    lines.append(line)
            (altered / path.name).write_bytes("\r\n".join(lines).encode("utf-8"))
        self.assertEqual(self._inspect(altered)["matches"], [BUYER])

    def test_some_moved_objects_do_not_hide_the_buyer(self):
        copy = self._embed(BUYER, "buyer")
        path = copy / "tile_0_0.map"
        lines = path.read_text(encoding="utf-8").split("\n")
        moved = 0
        for i, line in enumerate(lines[:-2]):
            if line == "Sceneryobjects\\Fixture\\tree.sco" and moved < 15:
                lines[i + 2] = str(Decimal(lines[i + 2]) + Decimal("0.1"))
                moved += 1
        path.write_text("\n".join(lines), encoding="utf-8")
        self.assertEqual(self._inspect(copy)["matches"], [BUYER])

    def test_two_buyers_averaging_or_mixing_copies_are_both_named(self):
        first = self._embed(BUYER, "first")
        second = self._embed(OTHER, "second")
        averaged = self.root / "averaged"
        mixed = self.root / "mixed"
        averaged.mkdir()
        mixed.mkdir()
        for index in range(TILES):
            name = f"tile_{index}_0.map"
            a = (first / name).read_text(encoding="utf-8").split("\n")
            b = (second / name).read_text(encoding="utf-8").split("\n")
            out = []
            for x, y in zip(a, b):
                out.append(str((Decimal(x) + Decimal(y)) / 2) if x != y else x)
            (averaged / name).write_text("\n".join(out), encoding="utf-8")
            (mixed / name).write_bytes(((first if index % 2 else second) / name).read_bytes())
        for suspect in (averaged, mixed):
            scores = self._scores(self._inspect(suspect))
            self.assertTrue(scores[BUYER]["match"], suspect.name)
            self.assertTrue(scores[OTHER]["match"], suspect.name)
            self.assertFalse(any(row["match"] for key, row in scores.items() if key not in (BUYER, OTHER)))

    def test_ten_thousand_wrong_buyers_never_match_one_tile(self):
        copy = self._embed(BUYER, "buyer")
        partial = self.root / "partial"
        partial.mkdir()
        (partial / "tile_5_0.map").write_bytes((copy / "tile_5_0.map").read_bytes())
        wrong = [f"wrong-{n:05d}" for n in range(10_000)]
        report = self._inspect(partial, [BUYER] + wrong)
        self.assertEqual(report["matches"], [BUYER])
        worst = max(row["score"] for row in report["candidates"] if row["id"] != BUYER)
        self.assertLess(worst, 0.35)

    def _rewrite_numbers(self, source: Path, target: Path, change) -> None:
        target.mkdir()
        for path in source.glob("tile_*.map"):
            lines = []
            for line in path.read_text(encoding="utf-8").split("\n"):
                try:
                    lines.append(change(Decimal(line)) if "." in line else line)
                except Exception:
                    lines.append(line)
            (target / path.name).write_text("\n".join(lines), encoding="utf-8")

    def test_rounding_to_millimetres_keeps_the_mark(self):
        copy = self._embed(BUYER, "buyer")
        rounded = self.root / "rounded"
        self._rewrite_numbers(copy, rounded, lambda value: f"{value:.3f}")
        self.assertEqual(self._inspect(rounded)["matches"], [BUYER])

    def test_millimetre_jitter_on_every_object_defeats_one_tile(self):
        # The documented limit: invisible random moves of a few millimetres drown the
        # mark when only a small part of the map is available.
        copy = self._embed(BUYER, "buyer")
        one = self.root / "one"
        one.mkdir()
        (one / "tile_7_0.map").write_bytes((copy / "tile_7_0.map").read_bytes())
        rng = random.Random(5)
        jittered = self.root / "jittered"
        self._rewrite_numbers(one, jittered, lambda value: str(value + Decimal(rng.randint(-5000, 5000)) / Decimal(1000000)))
        self.assertEqual(self._inspect(jittered)["matches"], [])

    def test_unsafe_inputs_are_refused_without_partial_output(self):
        existing = self.root / "existing"
        existing.mkdir()
        with self.assertRaises(FileExistsError):
            pw.embed(self.master, self.content, self.key, BUYER, existing)
        for bad in ("", "has space", "x" * 129, "slash/inside"):
            with self.assertRaises(ValueError):
                pw.embed(self.master, self.content, self.key, bad, self.root / "bad")
        for offset in ("0.05", "0.6"):
            with self.assertRaises(ValueError):
                pw.embed(self.master, self.content, self.key, BUYER, self.root / "bad", offset_mm=Decimal(offset))
        short_key = self.root / "short.key"
        short_key.write_bytes(b"x" * 16)
        with self.assertRaises(ValueError):
            pw.embed(self.master, self.content, short_key, BUYER, self.root / "bad")
        with self.assertRaises(FileExistsError):
            pw.keygen(self.key)
        broken = self.master / "tile_50_0.map"
        broken.write_text("[version]\n14\n\n[object]\n0\nSceneryobjects\\Fixture\\tree.sco\n1\nnot-a-number\n", encoding="utf-8")
        with self.assertRaises(ValueError):
            pw.embed(self.master, self.content, self.key, BUYER, self.root / "bad")
        broken.unlink()
        link = self.master / "linked.cfg"
        try:
            link.symlink_to(self.master / "global.cfg")
        except OSError:
            link = None  # Windows without the symlink privilege
        if link is not None:
            with self.assertRaises(ValueError):
                pw.embed(self.master, self.content, self.key, BUYER, self.root / "bad")
            link.unlink()
        self.assertFalse((self.root / "bad").exists())
        self.assertEqual([p for p in self.root.iterdir() if p.name.startswith(".placement-watermark-")], [])

    def test_older_tile_versions_are_marked_and_read(self):
        old = self.master / "tile_20_0.map"
        old.write_text(_tile_text(20, version=5, objects=60), encoding="utf-8")
        copy = self._embed(BUYER, "buyer")
        partial = self.root / "partial"
        partial.mkdir()
        (partial / old.name).write_bytes((copy / old.name).read_bytes())
        self.assertNotEqual(old.read_bytes(), (partial / old.name).read_bytes())
        self.assertEqual(self._inspect(partial)["matches"], [BUYER])

    def test_command_line(self):
        tool = Path(pw.__file__)
        key = self.root / "cli.key"
        self.assertEqual(subprocess.run([sys.executable, tool, "keygen", key]).returncode, 0)
        out = self.root / "cli-copy"
        run = subprocess.run([sys.executable, tool, "embed", "--key", key, "--buyer", BUYER,
                              "--content", self.content, self.master, out], capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(json.loads(run.stdout)["marked_objects"], TILES * (OBJECTS_PER_TILE + 1))
        ids = self.root / "ids.txt"
        ids.write_text("\n".join(self.candidates) + "\n", encoding="utf-8")
        run = subprocess.run([sys.executable, tool, "inspect", "--key", key, "--candidates", ids,
                              "--content", self.content, self.master, out], capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(json.loads(run.stdout)["matches"], [BUYER])
        run = subprocess.run([sys.executable, tool, "inspect", "--key", key, "--candidates", ids,
                              "--content", self.content, self.master, self.master], capture_output=True, text=True)
        self.assertEqual(run.returncode, 1)


if __name__ == "__main__":
    unittest.main()
