# Optional geometry and texture fingerprints

`tools/fingerprint.py` adds a small mark derived from a publisher-supplied opaque identifier to a new copy of an owned asset. The identifier itself is not written into the asset. The tool does not contact a service, collect buyer data, or monitor uploads.

```sh
python3 tools/fingerprint.py embed --kind obj --id 7e8d4f01 owned.obj marked.obj
python3 tools/fingerprint.py inspect --kind obj --id 7e8d4f01 marked.obj
python3 tools/fingerprint.py embed --kind ppm --id 7e8d4f01 owned.ppm marked.ppm
python3 tools/fingerprint.py inspect --kind ppm --id 7e8d4f01 marked.ppm
```

The identifier is reduced to a domain-separated 64-bit SHA-256 value. OBJ marks use the parity of a five-decimal grid for the first 64 vertex X coordinates; an edited coordinate changes by at most 0.000015 model units. To keep the grid exactly representable, OBJ coordinates must be within 2^51 grid units (about 22.5 billion model units). PPM marks use the low bit of the red channel for the first 64 pixels; each changed channel differs by at most one level. The output is created as a new file and never replaces an existing file.

The current checks demonstrate detection after OBJ comments/material references change and after a PPM header comment is added, while pixel order is preserved. They also check an unmarked sample and reject a wrong identifier. They do not measure a broad corpus or a buyer-scale false-positive rate. For uniformly random 64-bit samples the chance of a single accidental match is 1 in 2^64; deliberate edits and large populations have different risks.

Detection depends on the marked coordinate or pixel order surviving. Vertex reordering, decimation, coordinate transforms, lossy image conversion, resizing, cropping, or edits to marked coordinates/pixels can remove the mark. This is a bounded inspection aid, not durable watermarking or proof of ownership.

Inputs are limited to UTF-8 Wavefront OBJ files with at least 64 and at most 1,000,000 vertices. An OBJ X coordinate token can contain at most 64 bytes and its rounded grid coordinate must fit in 2^51 units. PPM images must be binary P6 with 8-bit RGB pixels and at least 64 pixels. Each input is limited to 64 MiB; other model and image formats are unsupported.
