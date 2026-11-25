#!/usr/bin/env python3
"""Add or inspect a small, opaque fingerprint in an owned OBJ or PPM asset."""

import argparse
import hashlib
import io
import os
from pathlib import Path
import re
import stat
import tempfile
from decimal import Decimal, InvalidOperation, ROUND_HALF_EVEN, localcontext


MAX_FILE_BYTES = 64 * 1024 * 1024
MAX_OBJ_VERTICES = 1_000_000
MAX_PPM_DIMENSION = 8192
MARK_BITS = 64
GRID_DECIMAL = Decimal("0.00001")
MAX_COORDINATE_UNITS = 2**51
ID_DOMAIN = b"openMapper content fingerprint v1\0"
_TOKEN = re.compile(r"\S+")
_PPM_SPACE = b" \t\r\n\v\f"


def _fingerprint(identifier: str) -> tuple[int, ...]:
    if not isinstance(identifier, str) or not identifier or len(identifier.encode("utf-8")) > 256:
        raise ValueError("identifier must contain 1 to 256 UTF-8 bytes")
    digest = hashlib.sha256(ID_DOMAIN + identifier.encode("utf-8")).digest()[: MARK_BITS // 8]
    return tuple((byte >> shift) & 1 for byte in digest for shift in range(7, -1, -1))


def _read(path: Path) -> bytes:
    with path.open("rb") as source:
        data = source.read(MAX_FILE_BYTES + 1)
    if len(data) > MAX_FILE_BYTES:
        raise ValueError(f"input exceeds {MAX_FILE_BYTES} bytes")
    return data


def _obj_vertex(line: str, line_number: int, *, marked: bool = False):
    token = _TOKEN.search(line)
    if token is None or token.group() != "v":
        return None
    coordinate = _TOKEN.search(line, token.end())
    if coordinate is None:
        raise ValueError(f"OBJ vertex on line {line_number} has fewer than three coordinates")
    if coordinate.end() - coordinate.start() > 64:
        raise ValueError(f"OBJ x coordinate on line {line_number} exceeds the supported token length")
    try:
        x = Decimal(line[coordinate.start():coordinate.end()])
    except InvalidOperation as error:
        raise ValueError(f"invalid OBJ x coordinate on line {line_number}") from error
    if not x.is_finite():
        raise ValueError(f"non-finite OBJ x coordinate on line {line_number}")
    limit = MAX_COORDINATE_UNITS if marked else MAX_COORDINATE_UNITS - 1
    if abs(x) > GRID_DECIMAL * limit:
        raise ValueError(f"OBJ x coordinate on line {line_number} exceeds the fingerprint precision range")
    with localcontext() as context:
        context.prec = 80
        units = int((x / GRID_DECIMAL).to_integral_value(rounding=ROUND_HALF_EVEN))
    if abs(units) > limit:
        raise ValueError(f"OBJ x coordinate on line {line_number} exceeds the fingerprint precision range")
    y = _TOKEN.search(line, coordinate.end())
    z = _TOKEN.search(line, y.end()) if y else None
    if y is None or z is None:
        raise ValueError(f"OBJ vertex on line {line_number} has fewer than three coordinates")
    return coordinate.start(), coordinate.end(), units


def _write_obj(source: Path, output, bits: tuple[int, ...]) -> None:
    count = 0
    size = 0
    with source.open("rb") as original:
        for line_number, raw_line in enumerate(original, start=1):
            size += len(raw_line)
            if size > MAX_FILE_BYTES:
                raise ValueError(f"input exceeds {MAX_FILE_BYTES} bytes")
            try:
                line = raw_line.decode("utf-8")
            except UnicodeDecodeError as error:
                raise ValueError("OBJ must be UTF-8 text") from error
            vertex = _obj_vertex(line, line_number)
            if vertex is not None:
                if count >= MAX_OBJ_VERTICES:
                    raise ValueError(f"OBJ exceeds {MAX_OBJ_VERTICES} vertices")
                if count < MARK_BITS:
                    start, end, quantized = vertex
                    marked = (quantized & ~1) | bits[count]
                    coordinate = format(Decimal(marked) * GRID_DECIMAL, ".5f")
                    line = line[:start] + coordinate + line[end:]
                count += 1
            output.write(line.encode("utf-8"))
    if count < MARK_BITS:
        raise ValueError(f"OBJ needs at least {MARK_BITS} vertices")


def _obj_matches(data: bytes, bits: tuple[int, ...]) -> bool:
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError as error:
        raise ValueError("OBJ must be UTF-8 text") from error
    observed = []
    count = 0
    for line_number, line in enumerate(io.StringIO(text, newline=""), start=1):
        vertex = _obj_vertex(line, line_number, marked=True)
        if vertex is None:
            continue
        _, _, units = vertex
        if count >= MAX_OBJ_VERTICES:
            raise ValueError(f"OBJ exceeds {MAX_OBJ_VERTICES} vertices")
        if count < MARK_BITS:
            observed.append(units & 1)
        count += 1
    if count < MARK_BITS:
        raise ValueError(f"OBJ needs at least {MARK_BITS} vertices")
    return tuple(observed) == bits


def _ppm_header(data: bytes) -> tuple[int, int, int]:
    if not data.startswith(b"P6") or len(data) < 3 or data[2] not in _PPM_SPACE:
        raise ValueError("PPM must use binary P6 format")
    index = 2
    values = []
    for _ in range(3):
        while index < len(data):
            if data[index] in _PPM_SPACE:
                index += 1
            elif data[index] == ord("#"):
                end = data.find(b"\n", index)
                if end < 0:
                    raise ValueError("unterminated PPM header comment")
                index = end + 1
            else:
                break
        start = index
        while index < len(data) and data[index] not in _PPM_SPACE + b"#":
            index += 1
        if start == index:
            raise ValueError("incomplete PPM header")
        token = data[start:index]
        if not token.isdigit():
            raise ValueError("PPM dimensions and maximum value must be integers")
        values.append(int(token))
    width, height, maximum = values
    if not 1 <= width <= MAX_PPM_DIMENSION or not 1 <= height <= MAX_PPM_DIMENSION:
        raise ValueError(f"PPM dimensions must be between 1 and {MAX_PPM_DIMENSION}")
    if maximum != 255:
        raise ValueError("PPM maximum value must be 255")
    if index >= len(data) or data[index] not in _PPM_SPACE:
        raise ValueError("PPM header has no raster separator")
    raster = index + (2 if data[index:index + 2] == b"\r\n" else 1)
    size = width * height * 3
    if raster + size != len(data):
        raise ValueError("PPM raster is truncated or has trailing bytes")
    if width * height < MARK_BITS:
        raise ValueError(f"PPM needs at least {MARK_BITS} pixels")
    return raster, size, width * height


def _embed_ppm(data: bytes, bits: tuple[int, ...]) -> bytes:
    raster, _, _ = _ppm_header(data)
    marked = bytearray(data)
    for index, bit in enumerate(bits):
        pixel = raster + index * 3
        marked[pixel] = (marked[pixel] & 0xFE) | bit
    return bytes(marked)


def _ppm_matches(data: bytes, bits: tuple[int, ...]) -> bool:
    raster, _, _ = _ppm_header(data)
    observed = tuple(data[raster + index * 3] & 1 for index in range(MARK_BITS))
    return observed == bits


def _atomic_new_file(path: Path, data, source_mode: int) -> None:
    if path.exists():
        raise FileExistsError(path)
    if not path.parent.is_dir():
        raise FileNotFoundError(path.parent)
    descriptor, temporary = tempfile.mkstemp(prefix=".fingerprint-", dir=path.parent)
    temp_path = Path(temporary)
    try:
        with os.fdopen(descriptor, "wb") as output:
            data(output) if callable(data) else output.write(data)
            output.flush()
            os.fsync(output.fileno())
            if hasattr(os, "fchmod"):
                os.fchmod(output.fileno(), stat.S_IMODE(source_mode) & ~0o022)
        if not hasattr(os, "fchmod"):
            os.chmod(temp_path, stat.S_IMODE(source_mode) & ~0o022)
        # link() creates the destination atomically and refuses to replace an existing file.
        os.link(temp_path, path)
    finally:
        temp_path.unlink(missing_ok=True)


def embed(source: Path, destination: Path, identifier: str, kind: str) -> None:
    """Write a marked copy without changing or overwriting either input file."""
    source = Path(source)
    destination = Path(destination)
    if source.resolve() == destination.resolve(strict=False):
        raise ValueError("source and destination must be different files")
    if destination.exists():
        raise FileExistsError(destination)
    bits = _fingerprint(identifier)
    match kind:
        case "obj":
            _atomic_new_file(destination, lambda output: _write_obj(source, output, bits), source.stat().st_mode)
        case "ppm":
            original = _read(source)
            result = _embed_ppm(original, bits)
            _atomic_new_file(destination, result, source.stat().st_mode)
        case _:
            raise ValueError("kind must be obj or ppm")


def matches(source: Path, identifier: str, kind: str) -> bool:
    """Check whether the ordered geometry or pixels contain this 64-bit marker."""
    data = _read(Path(source))
    bits = _fingerprint(identifier)
    match kind:
        case "obj":
            return _obj_matches(data, bits)
        case "ppm":
            return _ppm_matches(data, bits)
        case _:
            raise ValueError("kind must be obj or ppm")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("embed", "inspect"):
        command = commands.add_parser(name)
        command.add_argument("--kind", choices=("obj", "ppm"), required=True)
        command.add_argument("--id", required=True, help="opaque publisher-controlled identifier")
        command.add_argument("source", type=Path)
        if name == "embed":
            command.add_argument("destination", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "embed":
            embed(args.source, args.destination, args.id, args.kind)
            print("Fingerprint written to a new file.")
            return 0
        if matches(args.source, args.id, args.kind):
            print("Fingerprint matches.")
            return 0
        print("Fingerprint does not match.")
        return 1
    except (OSError, ValueError) as error:
        parser.error(str(error))
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
