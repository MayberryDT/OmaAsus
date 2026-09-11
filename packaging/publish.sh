#!/usr/bin/env bash
# Publishes the GitHub release for a tag from the built files:
#   publish.sh <tag> <dir>
# Adds SHA256SUMS and the install notes. Needs GH_TOKEN, GITHUB_REPOSITORY
# and GLIBC (the portable build's glibc floor). A re-run finishes a draft an
# interrupted run left, and refuses to touch a published release.
set -euo pipefail
tag="${1:?tag}"
dir="${2:?directory with the release files}"
src="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$("$src/packaging/version.sh" "$tag")"
repo=(--repo "$GITHUB_REPOSITORY")
cd "$dir"

pkg=(omaasus-"$version"-*-x86_64.pkg.tar.zst)
for f in "${pkg[0]}" "omaasus-$version-x86_64-linux.tar.gz" "omaasus-$version-source.tar.gz" PKGBUILD omaasus.install; do
    if [[ ! -f $f ]]; then
        echo "missing $f in $dir" >&2
        exit 1
    fi
done
rm -f SHA256SUMS
sha256sum -- * > SHA256SUMS

notes="$(mktemp)"
sed -e "s|@VERSION@|$version|g" -e "s|@PKG@|${pkg[0]}|g" -e "s|@GLIBC@|${GLIBC:?}|g" "$src/packaging/release-notes.md" > "$notes"
echo >> "$notes"
gh api "repos/$GITHUB_REPOSITORY/releases/generate-notes" -f tag_name="$tag" --jq .body >> "$notes"

# true (a draft), false (published), or missing; any other failure stops here
# rather than passing for "missing".
if state="$(gh release view "$tag" "${repo[@]}" --json isDraft -q .isDraft 2>&1)"; then
    :
elif [[ $state == *"release not found"* ]]; then
    state=missing
else
    echo "cannot tell whether release $tag exists: $state" >&2
    exit 1
fi

case $state in
    true)
        gh release upload "$tag" "${repo[@]}" --clobber -- *
        gh release edit "$tag" "${repo[@]}" --title "OmaAsus $version" --notes-file "$notes" --draft=false
        ;;
    false)
        # Rebuilt files would change the checksums people already verified.
        echo "release $tag is already published; delete it first to publish it again" >&2
        exit 1
        ;;
    missing)
        gh release create "$tag" "${repo[@]}" --verify-tag --title "OmaAsus $version" --notes-file "$notes" -- *
        ;;
    *)
        echo "unexpected answer about release $tag: $state" >&2
        exit 1
        ;;
esac
