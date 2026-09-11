#!/usr/bin/env bash
# Builds the pacman package from HEAD the way the AUR builds it from a
# release, and writes to <outdir>: the package, the source archive it was
# built from, and a PKGBUILD whose checksum matches that archive.
#   build.sh <outdir> [makepkg options]
# makepkg refuses root; in CI (root in a container) it runs as a throwaway user.
set -euo pipefail
out="${1:?output directory}"
shift
if [[ $EUID -eq 0 && ${GITHUB_ACTIONS:-} != true ]]; then
    echo "run this as a normal user; makepkg won't build as root" >&2
    exit 1
fi
src="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
version="$("$src/packaging/version.sh")"
archive="omaasus-$version-source.tar.gz"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
git -C "$src" archive --format=tar.gz --prefix="omaasus-$version/" -o "$work/$archive" HEAD
sum="$(sha256sum "$work/$archive" | cut -d' ' -f1)"
sed "s/^sha256sums=.*/sha256sums=('$sum')/" "$src/packaging/arch/PKGBUILD" > "$work/PKGBUILD"
cp "$src/packaging/arch/omaasus.install" "$work/"

# Keep the source and the package in $work whatever makepkg.conf says.
export PKGDEST="$work" SRCDEST="$work" PKGEXT=.pkg.tar.zst
if [[ $EUID -eq 0 ]]; then
    id builder &>/dev/null || useradd --create-home builder
    chown -R builder "$work"
    runuser -u builder -- bash -c 'cd "$1" && shift && makepkg --noconfirm "$@"' _ "$work" "$@"
else
    (cd "$work" && makepkg --noconfirm "$@")
fi

mkdir -p "$out"
cp "$work"/*.pkg.tar.zst "$work/$archive" "$work/PKGBUILD" "$work/omaasus.install" "$out/"
ls -l "$out"
