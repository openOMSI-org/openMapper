<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logos/openmapper-wordmark-light.svg">
    <img alt="openMapper" src="assets/logos/openmapper-wordmark-dark.svg" width="420">
  </picture>
</p>

<p align="center">
  <a href="https://github.com/openOMSI-org/openMapper/releases/latest"><img alt="Version" src="https://img.shields.io/github/v/release/openOMSI-org/openMapper?label=version&color=2e86ff&style=for-the-badge"></a>
  <img alt="Platforms" src="https://img.shields.io/badge/windows%20%C2%B7%20linux%20%C2%B7%20macos-x64%20%7C%20arm64-2d3138?style=for-the-badge">
  <img alt="Rust" src="https://img.shields.io/badge/rust-2024-2d3138?style=for-the-badge&logo=rust&logoColor=white">
  <a href="https://openomsi.org/"><img alt="Docs" src="https://img.shields.io/badge/openOMSI-website-2d3138?style=for-the-badge"></a>
  <a href="https://discord.gg/openomsi"><img alt="Discord" src="https://img.shields.io/badge/discord-join%20us-5865F2?style=for-the-badge&logo=discord&logoColor=white"></a>
  <a href="https://buymeacoffee.com/usonskyyy"><img alt="Buy me a coffee" src="https://img.shields.io/badge/buy%20me%20a%20coffee-support-ffdd00?style=for-the-badge&logo=buymeacoffee&logoColor=black"></a>
  <a href="https://ko-fi.com/usonance"><img alt="Ko-fi" src="https://img.shields.io/badge/ko--fi-support-29abe0?style=for-the-badge&logo=kofi&logoColor=white"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-AGPL--3.0--only-2e86ff?style=for-the-badge"></a>
</p>

> [!WARNING]
> **Early release. Expect bugs.** openMapper is in an early stage of development: things may be
> missing, broken or change between versions. Please report problems in
> [Issues](https://github.com/openOMSI-org/openMapper/issues) or on our
> [Discord server](https://discord.gg/openomsi). Keep a copy of a map you care about: saving
> keeps the last ten versions, but a copy is cheaper than a regret.

**openMapper** is a simple map editor for [openOMSI](https://openomsi.org/). Draw roads with a
couple of clicks, place buildings and trees from a visual catalogue, shape and paint the ground,
and drive your map in openOMSI with a single button. It's built for anyone to pick up, not just
experienced map makers.

> [!IMPORTANT]
> **openMapper needs an installed OMSI 2** (Steam or retail): maps are built from its
> content. The original folder is never written to. Maps and imported assets go into
> openOMSI's content folder, which the game reads before the installation.

## Download

Every release is on the [**Releases**](https://github.com/openOMSI-org/openMapper/releases)
page, built by GitHub Actions for each system and processor:

| Platform | Intel / AMD (x64) | ARM64 |
| --- | --- | --- |
| Windows 10 / 11 | `openMapper-0.1.18-windows-x64-setup.exe` or `-windows-x64.zip` | `openMapper-0.1.18-windows-arm64-setup.exe` or `-windows-arm64.zip` |
| macOS 11 or newer | `openMapper-0.1.18-macos-x64.dmg` or `-macos-x64.zip` | `openMapper-0.1.18-macos-arm64.dmg` or `-macos-arm64.zip` (Apple silicon) |
| Linux | `openMapper-0.1.18-linux-x64.tar.gz` | `openMapper-0.1.18-linux-arm64.tar.gz` |

* **Windows:** run the `-setup.exe`: it installs for you alone (no administrator rights), adds
  a Start menu entry and a desktop shortcut, and `Settings > Apps` removes it again. Or unpack the zip
  and run `openmapper.exe`: nothing else is needed either way.
* **macOS:** open the `.dmg` and drag `openMapper` onto Applications. The app is not notarised: the
  first time, right-click it and choose Open (or System Settings > Privacy & Security > Open Anyway).
  The zip holds the same `openMapper.app`.
* **Linux:** unpack the archive and run `./openmapper`; `openmapper.desktop` and the icon are in
  the folder for a menu entry. It needs GTK 3 for the file dialogs and a Vulkan or OpenGL driver.

New to it? [Your first map](docs/TUTORIAL.md) walks you from an empty window to a map you
can drive in service.

## What it can do

* **Roads and junctions**: two clicks make a road; junctions, traffic lights, crossings, switches and roundabouts come by themselves. Bus pockets with one tick and a click on the road; complex junctions drawn from above out of small splines, put on the map as they are or saved as custom junctions the automatic ones use.
* **Objects and plants**: a visual catalogue of the installation's buildings, trees and splines, lining up with roads, a plant brush.
* **Terrain and paint**: sculpting brushes, ramps, height maps, ground texture painting, tiles and water.
* **Real places**: satellite imagery, terrain heights and OpenStreetMap (roads, stops, lines, buildings, trees, water).
* **Lines and traffic**: timetable, tours, depot files, AI traffic and paths.
* **Your own models**: import glTF, FBX, OBJ and more, edit meshes, make materials and PBR maps, export.
* **Check and drive**: a map check, the game's own checks, safe saving with earlier versions, and a test drive in openOMSI on one key (`F5`).

The full list is in [docs/FEATURES.md](docs/FEATURES.md), the controls in its last section, and
the changes of each version in the [changelog](CHANGELOG.md).

## How it's built

Rust, written from scratch for openOMSI:

* **wgpu** from the org's fork [`wgpu-openomsi`](https://github.com/openOMSI-org/wgpu-openomsi)
  (the same revision openOMSI uses), Metal / Vulkan / Direct3D 12 / GL;
* the interface is [`egui-openomsi`](https://github.com/openOMSI-org/egui-openomsi), laid out
  like a classic desktop editor: menu bar, toolbar, docked panels (tool/catalogue on the left,
  properties on the right), status bar with help and cursor coordinates;
* icons are Google's [Material Symbols](https://fonts.google.com/icons) (Apache 2.0), stored in
  the repository: the full font is in `assets/fonts`, and the binary embeds a ~10 KB subset of
  the icons it uses (`tools/subset-icons.py`, add a name there to get a new icon). Nothing is
  downloaded at run time;
* reading and building content uses openOMSI's own crates: `omsi-map` (global.cfg, tiles,
  terrain), `omsi-scenery` (.sco/.sli), `omsi-model`, `omsi-o3d`, `omsi-geometry` (spline
  meshes exactly as in the game), `omsi-texture` (DDS/BMP/TGA, PBR), `omsi-cfg` (paths, VFS).

Saving writes OMSI 2's format: UTF-16 with CR LF, tiles at `[version] 14`,
`.map.terrain`, A8 DDS painting masks `texture/map/tile_x_y.map.N.dds`. Tests check
that the written tiles read back through openOMSI's own parser.

```bash
cargo run --release
```

`tools/package-macos.sh` makes `openMapper.app`, a zip of it and a `.dmg` in `target/package`,
`tools/package-linux.sh` a tar.gz with a desktop entry and `tools/package-windows.ps1` a zip and,
with NSIS installed, a `-setup.exe` (no console window, nothing else to install); a `v*` tag builds all three on GitHub and
publishes them as a release (`.github/workflows/release.yml`; Windows and macOS for x64 and ARM64,
Linux on a runner of each). The logo and icons are drawn by `tools/make-icons.py` from Inter
(`assets/logos`, `assets/icons/app`; the installer's and the disk image's pictures by
`tools/make-installer-art.sh`); the program embeds its icon (`openmapper --icon out.png 512`).

`openmapper path/to/global.cfg` opens a map straight away. `OPENMAPPER_SHOT=out.png` saves
a picture of the window and exits (for checks without a person at the screen).

## Contributing

Pull requests and issues are welcome: see [CONTRIBUTING.md](CONTRIBUTING.md). Security problems
are reported privately, see [SECURITY.md](SECURITY.md). What is planned is in
[docs/ROADMAP.md](docs/ROADMAP.md); what changed in each version is in the
[changelog](CHANGELOG.md).

## Support

openMapper is made by volunteers. If it helps you, you can support the work on
[Buy Me a Coffee](https://buymeacoffee.com/usonskyyy) or [Ko-fi](https://ko-fi.com/usonance);
questions and chat are on [Discord](https://discord.gg/openomsi).

## License

[GNU AGPL v3 only](LICENSE) (`AGPL-3.0-only`), matching openOMSI.
Copyright (c) 2026 openOMSI contributors. Files carrying their own license notices retain those licenses.

The logo is set in [Inter](https://rsms.me/inter/) (SIL Open Font License, in
`assets/fonts`); the icons are Google's [Material Symbols](https://fonts.google.com/icons)
(Apache 2.0).
