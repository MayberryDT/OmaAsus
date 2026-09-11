#!/usr/bin/env bash
# Installs OmaAsus from this release archive into /usr, or removes it.
#   sudo ./install.sh              install or update
#   sudo ./install.sh --uninstall  remove what an earlier run installed
# On Arch, install the release's pacman package instead.
# DESTDIR=<dir> installs under <dir> and leaves the running system alone.
set -euo pipefail
export LC_ALL=C
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
dest="${DESTDIR:+$(realpath -m -- "$DESTDIR")}"
manifest="$dest/usr/share/omaasus/MANIFEST"

if [[ -z $dest && $EUID -ne 0 ]]; then
    echo "run as root: sudo $0 $*" >&2
    exit 1
fi
# Once the pacman package is installed, pacman owns these files, even ones an
# earlier run of this script listed.
if [[ -z $dest ]] && command -v pacman >/dev/null && pacman -Q omaasus >/dev/null 2>&1; then
    echo "the omaasus pacman package is installed; manage it with pacman" >&2
    exit 1
fi

# Let systemd, udev and the system bus pick up the changed files.
reload() {
    [[ -z $dest ]] || return 0
    systemctl daemon-reload
    udevadm control --reload-rules
    udevadm trigger --action=change --subsystem-match=hidraw --subsystem-match=usb || true
    systemctl reload dbus.service 2>/dev/null ||
        busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig >/dev/null 2>&1 || true
}

# Removes the files named on stdin (paths relative to /).
remove() {
    local f
    while IFS= read -r f; do
        rm -f -- "$dest/$f"
    done
}

# Fixed modes: the extracted tree's modes come from the user's umask.
mode() {
    case $1 in
        usr/bin/* | usr/share/omaasus/install-helper.sh) echo 755 ;;
        *) echo 644 ;;
    esac
}

if [[ ${1:-} == --uninstall ]]; then
    if [[ ! -f $manifest ]]; then
        echo "no OmaAsus installed from a release archive ($manifest is missing)" >&2
        exit 1
    fi
    # The helper hands back the fans it guards as it stops.
    [[ -n $dest ]] || systemctl stop oma-helper.service 2>/dev/null || true
    remove < "$manifest"
    rm -f -- "$manifest"
    rmdir -- "$dest/usr/share/omaasus/helper" "$dest/usr/share/omaasus" "$dest/usr/share/licenses/omaasus" 2>/dev/null || true
    reload
    echo "OmaAsus removed"
    exit 0
fi

cd "$here/root"
files="$(mktemp)"
trap 'rm -f "$files"' EXIT
find . -type f -printf '%P\n' | sort > "$files"
while IFS= read -r f; do
    install -D -m "$(mode "$f")" -- "$f" "$dest/$f"
done < "$files"
# Files an earlier version installed that this one no longer ships.
if [[ -f $manifest ]]; then
    comm -23 <(sort "$manifest") "$files" | remove
fi
install -Dm644 "$files" "$manifest"
reload
# A running helper hands back the fans it guards as it stops; the next call
# starts the new binary.
if [[ -z $dest ]]; then
    systemctl try-restart oma-helper.service || echo "warning: couldn't restart oma-helper" >&2
fi

cat <<'EOF'
OmaAsus installed. The helper starts on demand; there is nothing to enable.
To start the overlay and automation at login:
  systemctl --user enable --now omaasus.service
EOF
