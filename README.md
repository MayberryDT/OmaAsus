# OmaAsus

A high-design control center and Hyprland overlay for ASUS gaming hardware,
written in Rust. It detects what the machine actually has — a ROG desktop
board with a Ryujin AIO and an RTX 4090, or a ROG laptop with `asusd` — and
exposes the right controls: **profiles** (manual and automatic), **fans** with
an interactive curve editor, **CPU** governor/EPP/boost/SMT/frequency limits,
**GPU** power limit / clock lock / offsets / fans, **lighting** through
OpenRGB, ASUS platform profiles and Armoury attributes, supergfxd graphics
modes, and the LiveDash OLED.

Two surfaces from one process:

* `omaasus` / `omaasus window` — the full control-center window.
* `omaasus --overlay` + `omaasus toggle` — a layer-shell panel with
  compositor blur that slides in over any game.

## Architecture

| crate | role |
|---|---|
| `oma-hw` | unprivileged hardware layer: hwmon (nct6775, rog_ryujin, asus-ec), cpufreq/amd-pstate, NVML (incl. driver-555+ clock offsets), amdgpu, Lian Li UNI hub HID, LiveDash OLED HID, CoolerControl REST, OpenRGB SDK, asusd / supergfxd / power-profiles-daemon / GameMode D-Bus proxies, Hyprland IPC, profile model, software fan engine |
| `oma-helper` | the only root component: system D-Bus service `com.omaasus.Helper1` gated by polkit (`com.omaasus.helper.control` for routine tuning, `com.omaasus.helper.advanced` for offsets and raw HID), sysfs allow-list, NVML apply |
| `oma-gui` | iced 0.14 + `iced_exwlshell` application (`omaasus`): pages, curve editor, gauges, overlay, IPC (`com.omaasus.App` on the session bus), automation engine |
| `oma-cli` | `oma inventory / sensors / watch / nvidia / cpu / daemons` diagnostics |

Research that informed the design lives in `research/` (asusctl D-Bus API,
supergfxctl, the desktop control stack, GUI stack evaluation).

## Build

```
cargo build --release
```

Requires Rust 1.98+, Wayland, Vulkan-capable GPU. Runtime optional deps:
CoolerControl, OpenRGB (`openrgb --server`), GameMode, power-profiles-daemon,
asusctl, supergfxctl, nvidia-utils.

## Install the helper

Settings → "Install helper" runs `scripts/install-helper.sh` through
`pkexec`; or by hand:

```
sudo scripts/install-helper.sh target/release/oma-helper crates/oma-helper/data
```

It installs the binary, D-Bus policy, polkit actions, the systemd unit and
udev rules (user access to Aura, Ryujin, Lian Li and LiveDash hidraw nodes).

## Hyprland

Hyprland ≥ 0.56 (Lua config): see `packaging/hyprland.lua`. Older releases:
`packaging/hyprland.conf`. Enable the overlay daemon with
`systemctl --user enable --now omaasus` (unit in `packaging/`).

## CLI

```
omaasus toggle | show | hide | window
omaasus profile <name>
omaasus page <dashboard|cpu|gpu|cooling|lighting|profiles|automation|asus|settings>
oma inventory [--json] | sensors | watch [secs] | nvidia | cpu | daemons
```

## Fan ownership

If CoolerControl is running, OmaAsus defaults to delegating fans to it and
activating a CoolerControl *Mode* per profile (configure the CCAdmin password
under Settings). Choose "OmaAsus" in Cooling to run the built-in engine:
per-target curves with hysteresis and ramp limiting, hardware Smart Fan IV
programming for board headers, Ryujin pump/fans via the kernel driver,
Lian Li hub via HID, GPU fans via NVML.

## Safety

* The helper only writes allow-listed sysfs attributes and ASUS/ENE HID
  devices; everything else is refused before polkit is even consulted.
* GPU clock offsets require admin authentication and are applied to the P0
  VF curve; start small and validate stability.
* On desktops without an internal panel, supergfxd's `Integrated` mode is
  never offered (it would unbind the display GPU).
* Releasing a fan target restores the saved `pwm_enable` mode for board
  headers, returns GPU fans to automatic, and re-enables PWM sync on Lian Li
  channels. The Ryujin falls back to a safe fixed duty.

## License

MIT.
