import sys
import tempfile
import tracemalloc
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from fingerprint import embed, matches


class FingerprintTests(unittest.TestCase):
    identifier = "fixture-opaque-buyer-7e8d"

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="openmapper-fingerprint-")
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def test_obj_marker_survives_text_and_filename_edits(self):
        original = "# owned geometry fixture\nmtllib old.mtl\n" + "".join(
            f"v {i}.125 0.0 1.0\n" for i in range(64)
        ) + "f 1 2 3\n"
        source = self.root / "source.obj"
        marked = self.root / "marked.obj"
        source.write_text(original, encoding="utf-8")
        self.assertFalse(matches(source, self.identifier, "obj"))

        embed(source, marked, self.identifier, "obj")

        self.assertEqual(source.read_text(encoding="utf-8"), original)
        self.assertTrue(matches(marked, self.identifier, "obj"))
        self.assertFalse(matches(marked, "different-opaque-id", "obj"))
        renamed = self.root / "renamed.obj"
        renamed.write_text(marked.read_text(encoding="utf-8").replace("old.mtl", "new.mtl").replace("f 1 2 3", "# new note\nf 1 2 3"), encoding="utf-8")
        self.assertTrue(matches(renamed, self.identifier, "obj"))
        xs = [float(line.split()[1]) for line in marked.read_text(encoding="utf-8").splitlines() if line.startswith("v ")]
        self.assertEqual(len(xs), 64)
        self.assertLessEqual(max(abs(a - b) for a, b in zip(xs, [i + 0.125 for i in range(64)])), 0.000025)

    def test_ppm_marker_changes_only_one_bit_per_pixel_and_survives_header_rewrite(self):
        pixels = bytes((i * 6) % 256 for i in range(64 * 3))
        source = self.root / "source.ppm"
        marked = self.root / "marked.ppm"
        source.write_bytes(b"P6\n8 8\n255\n" + pixels)
        self.assertFalse(matches(source, self.identifier, "ppm"))

        embed(source, marked, self.identifier, "ppm")

        result = marked.read_bytes()
        raster = result.index(b"255\n") + len(b"255\n")
        marked_pixels = result[raster:]
        self.assertTrue(matches(marked, self.identifier, "ppm"))
        self.assertFalse(matches(marked, "another-opaque-id", "ppm"))
        self.assertTrue(all(abs(a - b) <= 1 for a, b in zip(pixels, marked_pixels)))
        rewritten = self.root / "rewritten.ppm"
        rewritten.write_bytes(b"P6\n# editor comment\n8 8\n255\n" + marked_pixels)
        self.assertTrue(matches(rewritten, self.identifier, "ppm"))

    def test_bad_inputs_and_existing_outputs_fail_without_partial_files(self):
        source = self.root / "short.obj"
        output = self.root / "out.obj"
        source.write_text("v 0 0 0\n", encoding="utf-8")
        with self.assertRaises(ValueError):
            embed(source, output, self.identifier, "obj")
        self.assertFalse(output.exists())

        short = self.root / "short.ppm"
        short.write_bytes(b"P6\n8 8\n255\n" + b"\0" * 20)
        with self.assertRaises(ValueError):
            embed(short, self.root / "short-marked.ppm", self.identifier, "ppm")
        self.assertFalse((self.root / "short-marked.ppm").exists())

        existing = self.root / "existing.obj"
        existing.write_text("keep", encoding="utf-8")
        with self.assertRaises(FileExistsError):
            embed(source, existing, self.identifier, "obj")
        self.assertEqual(existing.read_text(encoding="utf-8"), "keep")

    def test_large_finite_coordinates_fail_with_a_user_facing_value_error(self):
        source = self.root / "huge.obj"
        output = self.root / "huge-marked.obj"
        source.write_text("v 1e308 0 0\n" + "".join(f"v {i} 0 0\n" for i in range(63)), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "coordinate"):
            embed(source, output, self.identifier, "obj")
        self.assertFalse(output.exists())


    def test_overlong_coordinate_token_is_rejected(self):
        source = self.root / "long-token.obj"
        output = self.root / "long-token-marked.obj"
        long_token = "0." + "0" * 70
        source.write_text(f"v {long_token} 0 0\n" + "".join(f"v {i} 0 0\n" for i in range(63)), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "coordinate"):
            embed(source, output, self.identifier, "obj")
        self.assertFalse(output.exists())

    def test_dense_obj_line_does_not_build_a_token_list(self):
        source = self.root / "dense.obj"
        output = self.root / "dense-marked.obj"
        dense_line = "v 0 0 0 " + " ".join("0" for _ in range(180_000)) + "\n"
        source.write_text(dense_line + "".join(f"v {i} 0 0\n" for i in range(64)), encoding="utf-8")

        tracemalloc.start()
        try:
            embed(source, output, self.identifier, "obj")
            _, peak = tracemalloc.get_traced_memory()
        finally:
            tracemalloc.stop()

        self.assertLess(peak, 8 * 1024 * 1024)
        self.assertTrue(matches(output, self.identifier, "obj"))

    @unittest.skipUnless(__import__("os").name == "posix", "POSIX file permission bits")
    def test_created_asset_does_not_grant_group_or_world_write_access(self):
        source = self.root / "permissions.obj"
        output = self.root / "permissions-marked.obj"
        source.write_text("".join(f"v {i} 0 0\n" for i in range(64)), encoding="utf-8")
        source.chmod(0o666)

        embed(source, output, self.identifier, "obj")

        self.assertEqual(output.stat().st_mode & 0o022, 0)


if __name__ == "__main__":
    unittest.main()
