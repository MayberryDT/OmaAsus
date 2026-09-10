#!/usr/bin/env bash
# Installs the OmaAsus privileged helper: binary, D-Bus policy, polkit
# actions, systemd unit and udev rules. Run as root (the GUI invokes it via
# pkexec). Usage: install-helper.sh <path-to-oma-helper-binary> <data-dir>
set -euo pipefail
BIN="${1:?helper binary path}"
DATA="${2:?data dir}"
install -Dm755 "$BIN" /usr/bin/oma-helper
install -Dm644 "$DATA/com.omaasus.Helper1.conf" /usr/share/dbus-1/system.d/com.omaasus.Helper1.conf
install -Dm644 "$DATA/com.omaasus.Helper1.service" /usr/share/dbus-1/system-services/com.omaasus.Helper1.service
install -Dm644 "$DATA/com.omaasus.helper.policy" /usr/share/polkit-1/actions/com.omaasus.helper.policy
install -Dm644 "$DATA/oma-helper.service" /usr/lib/systemd/system/oma-helper.service
install -Dm644 "$DATA/70-omaasus.rules" /usr/lib/udev/rules.d/70-omaasus.rules
udevadm control --reload-rules && udevadm trigger --subsystem-match=hidraw --action=add || true
systemctl daemon-reload
# dbus-broker picks up new policy files on reload.
systemctl reload dbus.service 2>/dev/null || busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig 2>/dev/null || true
# Activation-only: the unit has no [Install] section. Drop any boot-time
# enablement from earlier versions and (re)start so the new binary and device
# rules take effect.
systemctl disable oma-helper.service 2>/dev/null || true
systemctl restart oma-helper.service
echo "oma-helper installed and running"
