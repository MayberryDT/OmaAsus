#!/usr/bin/env bash
# Builds the portable release archive from release binaries:
#   make-tarball.sh <bindir> <outdir>
# The binaries may link only glibc, libgcc and libudev; Wayland, xkbcommon,
# Vulkan and NVML are loaded at runtime. Prints `glibc=<oldest version the
# binaries run on>` for $GITHUB_OUTPUT.
set -euo pipefail
bin="${1:?directory with omaasus, oma and oma-helper}"
out="${2:?output directory}"
src="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$("$src/packaging/version.sh")"
name="omaasus-$version-x86_64-linux"
binaries=("$bin/omaasus" "$bin/oma" "$bin/oma-helper")

allowed='^(linux-vdso\.so\.1|libudev\.so\.1|libgcc_s\.so\.1|libm\.so\.6|libc\.so\.6|/lib64/ld-linux-x86-64\.so\.2)$'
for b in "${binaries[@]}"; do
    extra="$(ldd "$b" | awk '{print $1}' | grep -Ev "$allowed" || true)"
    if [[ -n $extra ]]; then
        echo "$b links libraries the archive can't count on:" $extra >&2
        exit 1
    fi
done
glibc="$(objdump -T "${binaries[@]}" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
"$src/packaging/stage.sh" "$stage/$name/root" "$bin"
install -m755 "$src/packaging/portable/install.sh" "$stage/$name/install.sh"
install -m644 "$src/LICENSE" "$stage/$name/LICENSE"
mkdir -p "$out"
tar --sort=name --owner=0 --group=0 --numeric-owner -C "$stage" -czf "$out/$name.tar.gz" "$name"

echo "$out/$name.tar.gz: needs ${glibc/_/ } or newer" >&2
echo "glibc=${glibc#GLIBC_}"
