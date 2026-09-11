#!/usr/bin/env bash
# Lays out every file OmaAsus installs under a root directory, the way a
# package manager would:
#   stage.sh <root> <bindir>
# <bindir> holds the release binaries (omaasus, oma, oma-helper). The PKGBUILD
# and the portable archive both call this, so they never disagree on paths.
set -euo pipefail
root="${1:?root directory}"
bin="${2:?directory with omaasus, oma and oma-helper}"
src="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
data="$src/crates/oma-helper/data"
icons="$src/crates/oma-gui/assets/icons"

install -Dm755 -t "$root/usr/bin" "$bin/omaasus" "$bin/oma" "$bin/oma-helper"

# The helper: D-Bus policy and activation, polkit actions, the systemd unit
# D-Bus activates, and udev rules for the HID controllers.
install -Dm644 "$data/com.omaasus.Helper1.conf" "$root/usr/share/dbus-1/system.d/com.omaasus.Helper1.conf"
install -Dm644 "$data/com.omaasus.Helper1.service" "$root/usr/share/dbus-1/system-services/com.omaasus.Helper1.service"
install -Dm644 "$data/com.omaasus.helper.policy" "$root/usr/share/polkit-1/actions/com.omaasus.helper.policy"
install -Dm644 "$data/oma-helper.service" "$root/usr/lib/systemd/system/oma-helper.service"
install -Dm644 "$data/70-omaasus.rules" "$root/usr/lib/udev/rules.d/70-omaasus.rules"

# Desktop integration.
install -Dm644 "$src/packaging/omaasus.desktop" "$root/usr/share/applications/omaasus.desktop"
install -Dm644 "$src/packaging/omaasus.service" "$root/usr/lib/systemd/user/omaasus.service"
install -Dm644 -t "$root/usr/share/icons/hicolor/scalable/apps" "$icons/omaasus.svg" "$icons/omaasus-symbolic.svg"
install -Dm644 -t "$root/usr/share/omaasus" "$src/packaging/hyprland.lua" "$src/packaging/hyprland.conf"

# What Settings → Reinstall helper runs (crates/oma-gui/src/install.rs).
install -Dm755 "$src/scripts/install-helper.sh" "$root/usr/share/omaasus/install-helper.sh"
install -Dm644 -t "$root/usr/share/omaasus/helper" "$data"/*

install -Dm644 "$src/LICENSE" "$root/usr/share/licenses/omaasus/LICENSE"
