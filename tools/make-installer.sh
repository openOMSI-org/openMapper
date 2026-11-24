#!/bin/sh
# Makes the Windows installer (openMapper-<version>-windows-<arch>-setup.exe) from the folder
# tools/package-windows.ps1 fills (openmapper.exe, README.md, docs/, LICENSE). Needs NSIS (makensis).
#   sh tools/make-installer.sh target/package/openMapper-0.1.4-windows-x64 out/openMapper-setup.exe
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
dist="${1:?folder with openmapper.exe}"
out="${2:-target/package/openMapper-setup.exe}"
version="${OPENMAPPER_VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
case "$dist" in *arm64*) arch=arm64 ;; *) arch=x64 ;; esac
mkdir -p "$(dirname "$out")"
makensis -V2 -NOCD -DDIST="$dist" -DOUT="$out" -DVERSION="$version" -DARCH="$arch" tools/windows/installer.nsi
echo "$out"
