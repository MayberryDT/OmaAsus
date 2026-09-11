#!/usr/bin/env bash
# Prints the release version, after checking that Cargo.toml, the PKGBUILD
# and (when given) the tag agree:
#   version.sh [vX.Y.Z]
set -euo pipefail
src="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p;}' "$src/Cargo.toml")"
pkgver="$(sed -n 's/^pkgver=//p' "$src/packaging/arch/PKGBUILD")"
if [[ -z $version ]]; then
    echo "Cargo.toml has no [workspace.package] version" >&2
    exit 1
fi
if [[ $pkgver != "$version" ]]; then
    echo "packaging/arch/PKGBUILD has pkgver=$pkgver but Cargo.toml has $version" >&2
    exit 1
fi
if [[ $# -gt 0 && $1 != "v$version" ]]; then
    echo "tag $1 doesn't match the version in Cargo.toml ($version)" >&2
    exit 1
fi
echo "$version"
