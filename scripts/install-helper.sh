#!/usr/bin/env bash
# Installs the OmaAsus privileged helper: binary, D-Bus policy, polkit
# actions, systemd unit and udev rules. Run as root (the GUI invokes it via
# pkexec). Usage:
#   install-helper.sh <path-to-oma-helper-binary> <data-dir>
#   install-helper.sh --uninstall   remove a hand-installed helper again
set -euo pipefail

FILES=(
    /usr/bin/oma-helper
    /usr/share/dbus-1/system.d/com.omaasus.Helper1.conf
    /usr/share/dbus-1/system-services/com.omaasus.Helper1.service
    /usr/share/polkit-1/actions/com.omaasus.helper.policy
    /usr/lib/systemd/system/oma-helper.service
    /usr/lib/udev/rules.d/70-omaasus.rules
)

if [[ ${1:-} == --uninstall ]]; then
    # A hand install leaves files pacman does not own, and the package then
    # refuses to install over them ("exists in filesystem"). Take them out
    # first; the helper hands its fans back as it stops.
    # Every file, not just the binary: a mixed state (some owned by a
    # package, some by hand) must stop with a clear message rather than leave
    # the package half-removed.
    if command -v pacman >/dev/null; then
        owned=()
        for f in "${FILES[@]}"; do
            if [[ -e $f ]] && pacman -Qo "$f" >/dev/null 2>&1; then
                owned+=("$f")
            fi
        done
        if (( ${#owned[@]} )); then
            echo "these files belong to a package; remove it with pacman instead:" >&2
            printf '  %s\n' "${owned[@]}" >&2
            exit 1
        fi
    fi
    systemctl stop oma-helper.service 2>/dev/null || true
    systemctl disable oma-helper.service 2>/dev/null || true
    rm -f "${FILES[@]}"
    systemctl daemon-reload
    udevadm control --reload-rules || true
    systemctl reload dbus.service 2>/dev/null || busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig 2>/dev/null || true
    echo "oma-helper removed"
    exit 0
fi

BIN="${1:?helper binary path}"
DATA="${2:?data dir}"
# put <mode> <source> <target>; a packaged install passes its own files
# (/usr/bin/oma-helper), which are already in place.
put() {
    [[ $2 -ef $3 ]] || install -Dm"$1" "$2" "$3"
}
put 755 "$BIN" /usr/bin/oma-helper
put 644 "$DATA/com.omaasus.Helper1.conf" /usr/share/dbus-1/system.d/com.omaasus.Helper1.conf
put 644 "$DATA/com.omaasus.Helper1.service" /usr/share/dbus-1/system-services/com.omaasus.Helper1.service
put 644 "$DATA/com.omaasus.helper.policy" /usr/share/polkit-1/actions/com.omaasus.helper.policy
put 644 "$DATA/oma-helper.service" /usr/lib/systemd/system/oma-helper.service
put 644 "$DATA/70-omaasus.rules" /usr/lib/udev/rules.d/70-omaasus.rules
udevadm control --reload-rules && udevadm trigger --subsystem-match=hidraw --action=add || true
systemctl daemon-reload
# dbus-broker picks up new policy files on reload.
systemctl reload dbus.service 2>/dev/null || busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig 2>/dev/null || true
# Activation-only: the unit has no [Install] section. Drop any boot-time
# enablement from earlier versions and (re)start so the new binary and device
# rules take effect. The helper hands back the fans it guards as it stops.
systemctl disable oma-helper.service 2>/dev/null || true
systemctl restart oma-helper.service
echo "oma-helper installed and running"
