#!/usr/bin/env bash
# Publishes the GitHub release for a tag from the built files:
#   publish.sh <tag> <dir>
# Adds SHA256SUMS and the install notes. Needs GH_TOKEN, GITHUB_REPOSITORY
# and GLIBC (the portable build's glibc floor).
set -euo pipefail
tag="${1:?tag}"
dir="${2:?directory with the release files}"
src="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$("$src/packaging/version.sh" "$tag")"
cd "$dir"

pkg=(omaasus-"$version"-*-x86_64.pkg.tar.zst)
for f in "${pkg[0]}" "omaasus-$version-x86_64-linux.tar.gz" "omaasus-$version-source.tar.gz" PKGBUILD omaasus.install; do
    if [[ ! -f $f ]]; then
        echo "missing $f in $dir" >&2
        exit 1
    fi
done
sha256sum -- * > SHA256SUMS

notes="$(mktemp)"
sed -e "s|@VERSION@|$version|g" -e "s|@PKG@|${pkg[0]}|g" -e "s|@GLIBC@|${GLIBC:?}|g" "$src/packaging/release-notes.md" > "$notes"
echo >> "$notes"
gh api "repos/$GITHUB_REPOSITORY/releases/generate-notes" -f tag_name="$tag" --jq .body >> "$notes"
gh release create "$tag" --repo "$GITHUB_REPOSITORY" --verify-tag --title "OmaAsus $version" --notes-file "$notes" -- *
