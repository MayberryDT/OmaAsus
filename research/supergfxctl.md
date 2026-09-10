# supergfxctl / supergfxd — technical reference for D-Bus GUI integration

Target: **supergfxctl 5.2.7** (tag `5.2.7`, 2025-02-16), which is what Arch/AUR and the Omarchy repo package (`supergfxctl 5.2.7-2`). Where `main` (last commit 2025-11-08) differs from the tag, it is called out. Everything below was verified against the actual source at `https://gitlab.com/asus-linux/supergfxctl/-/raw/{main,5.2.7}/...` unless marked otherwise.

Researched 2026-09-09.

---

## 0. Status warning (read first)

- **The upstream project is archived.** The GitLab project page shows "This project is archived. Its data is read-only." Last tag is 5.2.7; last commit on `main` is 2025-11-08 ("asusmuxdgpu-fix"). Open issues (#170–#181, 2025-09 → 2026-04) have no maintainer resolution.
- **ArchWiki:** "supergfxctl is being phased out and its use is unadvised … unless you require vfio for virtual machines or have problems turning off your dGPU, don't install it. See cardwire for the successor project." — https://wiki.archlinux.org/title/Supergfxctl
- **Successor:** [cardwire](https://opengamingcollective.github.io/cardwire/) (OpenGamingCollective, same author lineage). It blocks GPU access with eBPF/LSM instead of unbinding PCI devices, needs no logout/reboot, requires Wayland, and is "in an early development stage, expect breaking changes." There is an Omarchy discussion proposing migration: https://github.com/omacom/omarchy/discussions/5982 (no decision recorded).
- **Kernel ≥ 6.19 breaks the ASUS-specific modes.** supergfxd 5.2.7 hard-codes `/sys/devices/platform/asus-nb-wmi/{gpu_mux_mode,dgpu_disable,egpu_enable}`. Since 6.19 these live in the `asus-armoury` firmware-attributes driver at `/sys/class/firmware-attributes/asus-armoury/attributes/<attr>/current_value`, and the old `asus-nb-wmi` attributes are deprecated / compiled out (`CONFIG_ASUS_WMI_DEPRECATED_ATTRS=n`). Result: `AsusMuxDgpu`/`AsusEgpu` disappear from `Supported`, and forcing them wedges the daemon. Issues #178 (MUX) and #177 (eGPU); CachyOS report https://github.com/CachyOS/linux-cachyos/issues/711. Hybrid/Integrated/Vfio still work.
- **Not on crates.io.** `https://crates.io/api/v1/crates/supergfxctl` → `{"errors":[{"detail":"crate `supergfxctl` does not exist"}]}`. Depend on it via git (`git = "https://gitlab.com/asus-linux/supergfxctl", tag = "5.2.7"`) or write your own proxy (recommended; see §2.6 and §6).

A GUI should therefore treat supergfxd as an optional, legacy backend: feature-detect it on the bus, expose only what `Supported` returns, and be prepared for `cardwire` later.

---

## 1. Components, files, packaging

| Item | Value | Source |
|---|---|---|
| Daemon binary | `/usr/bin/supergfxd` (`src/daemon.rs`, feature `daemon`) | Cargo.toml, Makefile |
| CLI binary | `/usr/bin/supergfxctl` (`src/cli.rs`, feature `cli`) | Cargo.toml |
| Library crate | `supergfxctl` (`src/lib.rs`), edition 2021, `rust-version = "1.64"`, license MPL-2.0 | Cargo.toml |
| systemd unit | `/usr/lib/systemd/system/supergfxd.service` — `Type=dbus`, `BusName=org.supergfxctl.Daemon`, `Environment=IS_SERVICE=1 RUST_LOG=debug`, `Restart=always RestartSec=1`, `Before=display-manager.service nvidia-powerd.service graphical.target multi-user.target`, `WantedBy=getty.target` | data/supergfxd.service |
| systemd preset | `enable supergfxd.service` | data/supergfxd.preset |
| D-Bus policy | `/usr/share/dbus-1/system.d/org.supergfxctl.Daemon.conf` — root may `own`; groups **adm, sudo, users, wheel** may `send_destination` / `receive_sender`. Any other user gets `org.freedesktop.DBus.Error.AccessDenied`. | data/org.supergfxctl.Daemon.conf |
| udev rule | `/usr/lib/udev/rules.d/90-supergfxd-nvidia-pm.rules` — on `bind` of NVIDIA class 0x030000/0x030200 PCI devices set `power/control=auto`; on `unbind` set `on`. | data/90-supergfxd-nvidia-pm.rules |
| optional udev rule (not installed by Makefile) | `99-nvidia-ac.rules` — stop `nvidia-powerd.service` on battery, start on AC. | data/99-nvidia-ac.rules |
| Xorg snippet | `/usr/share/X11/xorg.conf.d/90-nvidia-screen-G05.conf` — `Option "AllowNVIDIAGPUScreens"`. (Xorg "no longer supported" per README.) | data/90-nvidia-screen-G05.conf |
| Config file | `/etc/supergfxd.conf` (JSON) | `CONFIG_PATH` in lib.rs |
| Generated modprobe file | `/etc/modprobe.d/supergfxd.conf` (rewritten on every switch/boot when the dGPU is NVIDIA) | `MODPROBE_PATH` in lib.rs |
| Generated modules-load file | `/etc/modules-load.d/asus.conf` (`asus-wmi`, `asus-nb-wmi`) — only when `hotplug_type = Asus` and `dgpu_disable` is missing | special_asus.rs |
| Vulkan ICD toggle | renames `/usr/share/vulkan/icd.d/nvidia_icd.json` ↔ `nvidia_icd.json_inactive` (inactive in Integrated/Vfio) | config.rs `check_vulkan_icd` |
| Runtime deps | `lsof` (used to kill `/dev/nvidia0` holders), `lspci`, `systemctl`, `modprobe`/`rmmod`, libudev | lib.rs, pci_device.rs |
| Rust deps (5.2.7 lock) | `zbus 5.5.0`, `zvariant 5.1.0`, `logind-zbus 5.2.0`, `tokio`, `udev 0.9`, `serde`, `serde_json`, `gumdrop` (cli), `env_logger` (daemon) | Cargo.toml / Cargo.lock |

`supergfxd` refuses to run unless `IS_SERVICE=1` is in its environment (prints "supergfxd schould be only run from the right systemd service" and exits 0). Logs: `journalctl -b -u supergfxd` (debug level by default).

Packaging seen on this machine (Omarchy): `supergfxctl 5.2.7-2` in the `omarchy` repo, built from the unmodified AUR PKGBUILD (source tarball of tag 5.2.7, no patches), `depends=(gcc-libs systemd lsof)`, `conflicts=(supergfxctl-git optimus-manager)`. `asusctl 6.4.0` is in `extra`.

---

## 2. D-Bus surface

### 2.1 Addresses

| | |
|---|---|
| Bus | **system** bus |
| Well-known name | `org.supergfxctl.Daemon` (`DBUS_DEST_NAME`) |
| Object path | `/org/supergfxctl/Gfx` (`DBUS_IFACE_PATH`) |
| Interface | `org.supergfxctl.Daemon` |
| Also implemented | `org.freedesktop.DBus.Introspectable`, `.Peer`, `.Properties` (zbus defaults; **there are no properties**, so `PropertiesChanged` never fires) |

The name is **not activatable** (no `/usr/share/dbus-1/system-services/*.service`); if the daemon is not running you get `org.freedesktop.DBus.Error.ServiceUnknown` ("The name is not activatable"). `daemon.rs` calls `connection.request_name()` **before** building the controller and registering the object, then again after. So there is a window during boot (while `CtrlGraphics::new` rescans PCI and `reload()` runs the boot action list — modprobe, systemctl, etc.) where the **name is owned but the object does not exist** → calls fail with `org.freedesktop.DBus.Error.UnknownObject` / `UnknownMethod`. Treat that as "starting", not "unsupported".

### 2.2 Wire encoding of the enums

All enums are `#[derive(zvariant::Type, Serialize, Deserialize)]` unit-variant enums **without** `#[zvariant(signature = "s")]`, so zvariant encodes them as **`u` (u32 variant index)**, not strings. This is confirmed by the introspection XML captured by supergfxctl-gex (`resources/dbus/org-supergfxctl-gfx-5.xml`) and by the plasmoid reading `quint32`. Indices (declaration order in `src/pci_device.rs` / `src/actions.rs`):

**GfxMode** (`u`)

| value | variant | notes |
|---|---|---|
| 0 | `Hybrid` | dGPU offload (PRIME) |
| 1 | `Integrated` | dGPU unbound/removed, drivers blacklisted |
| 2 | `NvidiaNoModeset` | only offered when kernel cmdline has `nvidia-drm.modeset=0`; for GA401I-class laptops |
| 3 | `Vfio` | dGPU bound to `vfio-pci` |
| 4 | `AsusEgpu` | ASUS Flow eGPU (`egpu_enable`) |
| 5 | `AsusMuxDgpu` | ASUS MUX set to dGPU (`gpu_mux_mode=0`); always reboot |
| 6 | `None` | "unknown / nothing"; `Display` prints **"Unknown"**; is the `#[default]` |

**GfxPower** (`u`)

| value | variant | `-S` string | meaning |
|---|---|---|---|
| 0 | `Active` | `active` | `power/runtime_status == active` |
| 1 | `Suspended` | `suspended` | runtime-PM suspended (D3) |
| 2 | `Off` | `off` | sysfs read failed (device removed/unbound) → treated as Off |
| 3 | `AsusDisabled` | `dgpu_disabled` | `dgpu_disable=1` |
| 4 | `AsusMuxDiscreet` | `asus_mux_discreet` | MUX in dGPU mode |
| 5 | `Unknown` | `unknown` | default; also what the status thread uses when `Power` would error |

**UserActionRequired** (`u`) — the XML comment calls it `GfxRequiredUserAction`

| value | variant | CLI verbose string |
|---|---|---|
| 0 | `Logout` | "Logout required to complete mode change" |
| 1 | `Reboot` | "Reboot required to complete mode change" |
| 2 | `SwitchToIntegrated` | "You must switch to Integrated first" |
| 3 | `AsusEgpuDisable` | "The mode must be switched to Integrated or Hybrid first" |
| 4 | `Nothing` | "No action required" |

**GfxVendor** is *not* sent as an enum; `Vendor` returns a string: `"Nvidia"`, `"AMD"`, `"Intel"`, `"Unknown"`, `"ASUS dGPU disabled"`.

**HotplugType** (`u`, inside the `Config` struct): 0 `Std`, 1 `Asus`, 2 `None`.

### 2.3 Methods

Exact introspection (from gex's captured XML, matches `src/zbus_iface.rs`):

```xml
<interface name="org.supergfxctl.Daemon">
  <method name="Version">           <arg type="s"  direction="out"/></method>
  <method name="Mode">              <arg type="u"  direction="out"/></method>
  <method name="Supported">         <arg type="au" direction="out"/></method>
  <method name="Vendor">            <arg type="s"  direction="out"/></method>
  <method name="Power">             <arg type="u"  direction="out"/></method>
  <method name="SetMode">           <arg name="mode" type="u" direction="in"/>
                                    <arg type="u"  direction="out"/></method>
  <method name="PendingMode">       <arg type="u"  direction="out"/></method>
  <method name="PendingUserAction"> <arg type="u"  direction="out"/></method>
  <method name="Config">            <arg type="(ubbbbtu)" direction="out"/></method>
  <method name="SetConfig">         <arg name="config" type="(ubbbbtu)" direction="in"/></method>
  <signal name="NotifyGfxStatus">   <arg name="status" type="u"/></signal>
  <signal name="NotifyGfx">         <arg name="vendor" type="u"/></signal>
  <signal name="NotifyAction">      <arg name="action" type="u"/></signal>
</interface>
```

Semantics (from `src/zbus_iface.rs` + `src/controller.rs`):

| Method | Returns | Behaviour |
|---|---|---|
| `Version() → s` | `CARGO_PKG_VERSION`, e.g. `"5.2.7"` | constant |
| `Mode() → u` (GfxMode) | If `gpu_mux_mode` sysfs exists **and** reads `0` (Discreet) → `AsusMuxDgpu` (5) regardless of config. Else `config.tmp_mode` if set (never set in 5.x) else **`config.mode`** — i.e. the *last successfully applied / saved* mode, **not** the live hardware state. During an in-flight switch it still returns the old mode. | Errors: `org.freedesktop.DBus.Error.Failed` "GFX fail: …" (practically never) |
| `Supported() → au` | MUX Discreet → 5.2.7: `[AsusMuxDgpu]` only (issue #171); `main`: `[AsusMuxDgpu, Integrated, Hybrid]`. Otherwise: if dGPU vendor is `Unknown` **and** no `dgpu_disable` sysfs → `[Integrated]`; else `[Integrated, Hybrid]` + `Vfio` if `vfio_enable` + `AsusEgpu` if `egpu_enable` sysfs exists + `AsusMuxDgpu` if `gpu_mux_mode` sysfs exists + `NvidiaNoModeset` if cmdline has `nvidia-drm.modeset=0`. | Cheap; locks dgpu + config mutexes. |
| `Vendor() → s` | dGPU vendor string (see §2.2). `Unknown` when no NVIDIA/AMD GPU was classified as dGPU. | |
| `Power() → u` (GfxPower) | MUX Discreet → `AsusMuxDiscreet`. Else reads `<dgpu sysfs>/power/runtime_status`. If no dGPU tracked: `AsusDisabled` if `dgpu_disable=1`, `AsusMuxDiscreet` if MUX discreet, **else D-Bus error** `Failed: "GFX fail: get_runtime_status: Could not find dGPU"`. | **Locks the dGPU mutex** — blocks while a switch action holds it (see §7). |
| `SetMode(u) → u` (UserActionRequired) | See §2.4. Returns quickly (spawns a tokio task) with the *predicted* user action. Also emits `NotifyAction(action)` and `NotifyGfx(requested_mode)` **immediately**, before the switch actually happens. | Errors: `Failed: "GFX fail: Egpu mode requested when either the laptop doesn't support it or the kernel is not recent enough"`; other `GfxError` displays. |
| `PendingMode() → u` | `config.pending_mode` or `None`(6). Set at the start of `SetMode`, cleared when the background task finishes (success or failure). **Not cleared** when `SetMode` short-circuits with a `UserAction` (see §2.4 pitfall). | Only locks config → does not block during switches. |
| `PendingUserAction() → u` | `config.pending_action` or `Nothing`(4). Same lifecycle as `PendingMode`. | |
| `Config() → (ubbbbtu)` | `GfxConfigDbus { mode:u, vfio_enable:b, vfio_save:b, always_reboot:b, no_logind:b, logout_timeout_s:t, hotplug_type:u }`. (The doc comments still list an old 7-field layout with `compute_save`; ignore them — the struct is authoritative.) | |
| `SetConfig((ubbbbtu))` | Copies `vfio_enable, vfio_save, always_reboot, no_logind, logout_timeout_s` into the live config (**does not persist to disk**, **ignores `hotplug_type`**, ignores `mode`). Bug: `do_mode_change = cfg.mode == config.mode` (should be `!=`) — if you pass the *current* mode it re-runs `SetMode(current)`, which is a no-op `Nothing`. Effectively useless; edit `/etc/supergfxd.conf` and `systemctl restart supergfxd` instead. | |

All methods on the daemon side are `async` and take `&self` except `SetMode`/`SetConfig` which take `&mut self` — zbus serialises these with an interface RwLock (see §7.2).

### 2.4 `SetMode` in detail (`CtrlGraphics::set_gfx_mode`)

```
mode_support_check(mode)                 # only rejects AsusEgpu when egpu_enable sysfs is absent
vendor = dgpu.lock().vendor()            # blocks if a switch task currently holds the dgpu lock
from   = config.mode
user_action = always_reboot ? Reboot : UserActionRequired::mode_change_action(mode, from)
actions     = StagedAction::action_list_for_switch(&config, vendor, from, mode)
config.pending_mode = Some(mode); config.pending_action = Some(user_action)
match actions:
  Action::UserAction(u)        => return Ok(u)     # NOTE: pending_* left set, nothing else happens
  Action::StagedActions(list)  => tokio::spawn(run list); return Ok(user_action)
```

The background task, per action: `dgpu.lock()` → `action.perform()`. On `SystemdUnitWaitTimeout` (logout wait > 30 s, or display-manager stop didn't reach `inactive` within 3 s) it **breaks**; on any other error it logs and **continues** with the remaining actions but marks `failed`. At the end: `pending_* = None`; if `!failed` → `config.mode = mode; config.write()` (this is the only place the mode is persisted); if `failed` → best-effort rollback by running `action_list_for_switch(mode → from)` (note: it passes `mode`, i.e. the *target*, as `changing_to` to those rollback actions, so `WriteModprobeConf` during rollback writes the target's modprobe file — a bug; rollback is unreliable).

**Which user action is returned** — `mode_change_action(new, current)`; rows = requested mode, columns = current mode:

| requested ↓ / current → | Hybrid | Integrated | NvidiaNoModeset | Vfio | AsusEgpu | AsusMuxDgpu | None |
|---|---|---|---|---|---|---|---|
| **Hybrid** | Nothing | **Logout** | Nothing | SwitchToIntegrated¹ | Logout | Reboot | Nothing |
| **Integrated** | **Logout** | Nothing | Nothing | Nothing | Logout | Reboot | Nothing |
| **NvidiaNoModeset** | Nothing | Nothing | Nothing | Nothing | Logout | Reboot | Nothing |
| **Vfio** | Logout¹ | Nothing | Nothing | Nothing | Logout¹ | Reboot | Nothing |
| **AsusEgpu** | Logout | Logout | Logout | SwitchToIntegrated¹ | Nothing | Reboot | Nothing |
| **AsusMuxDgpu** | Reboot | Reboot | Reboot | Reboot | Reboot² | Nothing | Nothing |
| **None** | Nothing | Nothing | Nothing | Nothing | Nothing | Nothing | Nothing |

¹ Overridden by `action_list_for_switch`, which returns `Action::UserAction(SwitchToIntegrated)` for Hybrid→Vfio and AsusEgpu→Vfio, so `SetMode` actually **returns `SwitchToIntegrated` (2)** and does nothing. ² AsusEgpu→AsusMuxDgpu returns `Action::UserAction(AsusEgpuDisable)` → **returns `AsusEgpuDisable` (3)**, does nothing.
Also: NvidiaNoModeset→Hybrid and NvidiaNoModeset→AsusEgpu return `UserAction(Nothing)` and **do nothing** (mode is not even saved). If `always_reboot` is set the return value is always `Reboot` but the staged actions still run immediately (with the logind wait/DM stop replaced by no-ops) — i.e. it will try to unload drivers under a live session.

**Which mode changes need a logout** (rebootless path, `no_logind=false`, `always_reboot=false`): Hybrid↔Integrated, Hybrid→AsusEgpu, Integrated→AsusEgpu, Vfio→AsusEgpu, AsusEgpu→Hybrid/Integrated. These action lists begin with `WaitLogout, StopDisplayManager` and end with `StartDisplayManager`.
**Instant (no logout)**: Integrated↔Vfio, Integrated→NvidiaNoModeset, NvidiaNoModeset→Integrated/Vfio, Vfio→Hybrid/NvidiaNoModeset (note Vfio→Hybrid is "instant" and returns Nothing because the dGPU was never owned by a display driver).
**Reboot**: anything to/from `AsusMuxDgpu` (the daemon just flips `gpu_mux_mode`; from AsusMuxDgpu to *anything* runs only `[AsusMuxIgpu]` and then the target mode is applied on the next boot by the boot action list). The README: "A reboot is *always* required due to how this works in ACPI"; the kernel `asus-armoury` driver also sets `pending_reboot` for `gpu_mux_mode`.

Staged action lists (5.2.7 = `main` minus the `Enable/DisableNvidiaPersistenced` steps that `main` inserts next to every powerd step):

```
Hybrid → Integrated:   WaitLogout, StopDisplayManager, DisableNvidiaPowerd, KillNvidia|KillAmd,
                       UnloadGpuDrivers, UnbindRemoveGpu, WriteModprobeConf, CheckVulkanIcd,
                       <hotplug_rm>, StartDisplayManager
Integrated → Hybrid:   WaitLogout, StopDisplayManager, WriteModprobeConf, CheckVulkanIcd,
                       <hotplug_add>, RescanPci, LoadGpuDrivers, EnableNvidiaPowerd, StartDisplayManager
Integrated → Vfio:     WriteModprobeConf, CheckVulkanIcd, <hotplug_add>, RescanPci, DisableNvidiaPowerd,
                       Kill*, UnloadGpuDrivers, UnbindGpu, LoadVfioDrivers
Vfio → Integrated:     Kill*, UnloadVfioDrivers, UnbindRemoveGpu
Vfio → Hybrid:         Kill*, UnloadVfioDrivers, WriteModprobeConf, CheckVulkanIcd, RescanPci, LoadGpuDrivers
Integrated → NvidiaNoModeset: WriteModprobeConf, CheckVulkanIcd, RescanPci, LoadGpuDrivers, EnableNvidiaPowerd
* → AsusMuxDgpu:       [WriteModprobeConf (only from Integrated)], CheckVulkanIcd, [<hotplug_add>], EnableNvidiaPowerd, AsusMuxDgpu
AsusMuxDgpu → *:       AsusMuxIgpu
* → AsusEgpu / AsusEgpu → *: see src/actions.rs lines ~359-546 (logout flows incl. AsusEgpuEnable/Disable, AsusDgpuEnable)
<hotplug_rm/add> = HotplugUnplug/HotplugPlug (Std) | AsusDgpuDisable/AsusDgpuEnable (Asus) | DevTreeManaged no-op (None)
```

What the actions do (`StagedAction::perform`):
- `WaitLogout`: polls `org.freedesktop.login1.Manager.ListSessions` every 100 ms until no session with `Class=user`, `Type∈{x11,wayland,mir}`, `State∈{online,active}` exists. **Timeout is hard-coded to 30 s** (`let logout_timeout_s = 30;` in `src/actions.rs`) — the config's `logout_timeout_s` (default 180) is **never read** in 5.2.7. On timeout → `SystemdUnitWaitTimeout` → abort + rollback. Sessions of `Type=tty` are ignored, so a compositor started from a tty login (no display manager) is *not* waited for.
- `StopDisplayManager`: `systemctl stop display-manager.service`, then poll `systemctl is-active` every 250 ms up to 3 s for `inactive`. If the unit does not exist, `systemctl stop` exits 5 ("Unit … not loaded") → error → the switch is marked **failed** (remaining actions still run, then rollback). So rebootless Hybrid↔Integrated **requires a `display-manager.service` alias** (sddm/gdm/ly/greetd all provide one; this Omarchy box has `sddm.service` aliased).
- `UnloadGpuDrivers`/`LoadGpuDrivers`: `rmmod`/`modprobe` of `nvidia_drm, nvidia_modeset, nvidia_uvm, nvidia` (+ `nvidia_wmi_ec_backlight` on `main`) in that order, up to 7 attempts each with 50 ms sleeps; "is not currently loaded" is fine; "is builtin" → `VfioBuiltin` error; "Module X not found" → `MissingModule`; only for NVIDIA vendor (AMD: no-op).
- `KillNvidia`: `lsof /dev/nvidia0` and `kill -9` every PID listed. `KillAmd`: TODO/no-op.
- `UnbindRemoveGpu`: write PCI address to `<driver>/unbind`, then `1` to `<device>/remove` for each tracked function (GPU + audio etc.) in reverse order. `UnbindGpu`: unbind only. `RescanPci`: `echo 1 > /sys/bus/pci/rescan` (or re-detect devices if none tracked).
- `HotplugUnplug/Plug`: write `0`/`1` to `/sys/bus/pci/slots/<slot>/power` for the dGPU slot (only if a slot whose `address` matches the dGPU exists).
- `AsusDgpuDisable/Enable`, `AsusEgpuEnable/Disable`, `AsusMuxDgpu/Igpu`: write `1`/`0` to the `asus-nb-wmi` sysfs files, with 500 ms sleep before, 50 ms after + PCI rescan when enabling.
- `WriteModprobeConf`: rewrites `/etc/modprobe.d/supergfxd.conf` (NVIDIA only). Hybrid/AsusEgpu/NvidiaNoModeset: `blacklist nouveau`, `alias nouveau off`, `options nvidia-drm modeset=1` (+ `options nvidia-wmi-ec-backlight force=1` on `main`). Integrated: blacklist `nouveau nvidia_drm nvidia_uvm nvidia_modeset nvidia` (+ `nvidia-wmi-ec-backlight`), plus `options nvidia-drm modeset=1`. Vfio: the Integrated blacklist + `options vfio-pci ids=10DE:xxxx,10DE:yyyy,`. AsusMuxDgpu/None: empty file.
- `CheckVulkanIcd`: rename NVIDIA Vulkan ICD JSON away in Integrated/Vfio, restore otherwise (errors ignored).
- `Enable/DisableNvidiaPowerd`: `systemctl start|stop nvidia-powerd.service` (NVIDIA only; failures only warned).

### 2.5 Signals

| Signal | Arg | When |
|---|---|---|
| `NotifyGfxStatus(u status: GfxPower)` | | Emitted by a daemon-side polling task (`start_notify_status`) that calls `get_runtime_status()` **every 1 s** and emits only when the value changed from the last emitted value. Initial `last_status = Unknown`, so no signal until the first non-Unknown reading. If `Power` would error (no dGPU) the task maps it to `Unknown` and stays silent. **This is the only "live" signal.** |
| `NotifyGfx(u vendor: GfxMode)` | arg is misnamed `vendor`; it is the **requested** mode | Emitted inside `SetMode` right after the background task is spawned — i.e. **before** the switch happens, and even when `SetMode` returned `Nothing`/`SwitchToIntegrated` (in the `UserAction` short-circuit path the signals are still emitted because `set_mode` emits after `set_gfx_mode` returns `Ok`). Nothing is emitted when the background switch completes or fails. |
| `NotifyAction(u action: UserActionRequired)` | | Emitted together with `NotifyGfx`, carrying the same value `SetMode` returns. |

There is **no completion signal** and **no `PropertiesChanged`**. To learn that a switch finished you must poll `PendingMode` until it returns `None` (6) and then re-read `Mode`.

### 2.6 Bundled zbus proxy — signature mismatch warning

`src/zbus_proxy.rs` (generated long ago with `zbus-xmlgen 3.0.0`) declares:

```rust
#[proxy(interface = "org.supergfxctl.Daemon",
        default_service = "org.supergfxctl.Daemon",
        default_path = "/org/supergfxctl/Gfx")]
pub trait Daemon {
    fn version(&self) -> zbus::Result<String>;
    fn config(&self) -> zbus::Result<(u32, bool, bool, bool, bool, bool, u64, bool)>;   // (ubbbbbtb) — STALE
    fn set_config(&self, config: &(u32, bool, bool, bool, bool, bool, u64, bool)) -> zbus::Result<()>;
    fn power(&self) -> zbus::Result<GfxPower>;
    fn set_mode(&self, mode: &GfxMode) -> zbus::Result<UserActionRequired>;
    fn pending_mode(&self) -> zbus::Result<GfxMode>;
    fn pending_user_action(&self) -> zbus::Result<UserActionRequired>;
    fn mode(&self) -> zbus::Result<GfxMode>;
    fn supported(&self) -> zbus::Result<Vec<GfxMode>>;
    fn vendor(&self) -> zbus::Result<String>;
    #[zbus(signal)] fn notify_gfx_status(&self, status: GfxPower) -> zbus::Result<()>;
    #[zbus(signal)] fn notify_action(&self, action: UserActionRequired) -> zbus::Result<()>;
    #[zbus(signal)] fn notify_gfx(&self, mode: GfxMode) -> zbus::Result<()>;
}
```

The `config`/`set_config` tuple is `(ubbbbbtb)` (8 fields, old `compute_save` + bool hotplug) while the daemon's `GfxConfigDbus` is `(ubbbbtu)`; calling `config()` through this proxy fails with a signature mismatch. Everything else in the proxy is correct. The macro generates `DaemonProxy` (async) and `DaemonProxyBlocking`, plus `receive_notify_gfx_status()` → `NotifyGfxStatusStream` whose items expose `.args()?.status`, and likewise `receive_notify_gfx()` (`.args()?.mode`) and `receive_notify_action()` (`.args()?.action`).

---

## 3. The Rust crate as a library

Public modules (`src/lib.rs`): `config`, `controller`, `error`, `special_asus`, `zbus_iface`, `zbus_proxy`, `pci_device`, `systemd`, `actions`. Constants: `VERSION`, `CONFIG_PATH = "/etc/supergfxd.conf"`, `DBUS_DEST_NAME = "org.supergfxctl.Daemon"`, `DBUS_IFACE_PATH = "/org/supergfxctl/Gfx"`, `CONFIG_NVIDIA_VKICD`, `KERNEL_CMDLINE`.

Cargo features: `default = ["daemon", "cli", "zbus_tokio"]`; `daemon` pulls `env_logger`, `cli` pulls `gumdrop`, `zbus_tokio` enables `zbus/tokio`. For a client you would want `default-features = false, features = ["zbus_tokio"]` — but note the crate unconditionally depends on `udev`, `logind-zbus`, `tokio` and its metadata is copy-pasted from asusctl (`repository = ".../asusctl"`, `documentation = "https://docs.rs/rog-anime"`, description "Types useful for fancy keyboards"). It is not on crates.io; only a git dependency works. Given the small surface, **re-declaring the three enums and a `#[proxy]` in your own crate is the pragmatic choice** (see §6).

Types (all `Copy + Clone + Debug + PartialEq + Eq + Serialize + Deserialize + zvariant::Type`):

```rust
// src/pci_device.rs
#[derive(Default)] pub enum GfxMode { Hybrid, Integrated, NvidiaNoModeset, Vfio, AsusEgpu, AsusMuxDgpu, #[default] None }
impl Display for GfxMode   // Debug name, except None → "Unknown"
impl FromStr for GfxMode   // exact, case-sensitive: "Hybrid" | "Integrated" | "NvidiaNoModeset" | "Vfio" | "AsusEgpu" | "AsusMuxDgpu"; else GfxError::ParseMode ("None" is NOT parseable)

#[derive(Default)] pub enum GfxPower { Active, Suspended, Off, AsusDisabled, AsusMuxDiscreet, #[default] Unknown }
impl FromStr for GfxPower  // "active"|"suspended"|"off"|"dgpu_disabled"|"asus_mux_discreet"|_ → Unknown (lower-cased)
impl From<&GfxPower> for &str  // the strings above / "unknown"

pub enum GfxVendor { Nvidia, Amd, Intel, Unknown, AsusDgpuDisabled }
impl From<u16>/From<&str> for GfxVendor   // 0x1002 Amd, 0x10DE Nvidia, 0x8086 Intel
impl From<GfxVendor> for &str             // "Nvidia" | "AMD" | "Intel" | "Unknown" | "ASUS dGPU disabled"

pub enum HotplugType { Std, Asus, None }
pub enum HotplugState { On, Off }
pub enum RuntimePowerManagement { Auto, On, Off }   // "auto"/"on"/"off" written to power/control
pub struct Device { .. }        // one PCI function; dev_path(), vendor(), is_dgpu(), pci_id()
pub struct DiscreetGpu { .. }   // DiscreetGpu::new() rescans PCI and detects the dGPU; vendor(), devices(), get_runtime_status(), set_runtime_pm(), unbind(), remove(), ...

// src/actions.rs
pub enum UserActionRequired { Logout, Reboot, SwitchToIntegrated, AsusEgpuDisable, Nothing }
impl UserActionRequired { pub fn mode_change_action(new_mode: GfxMode, current_mode: GfxMode) -> Self }
impl Display for UserActionRequired            // variant name
impl From<UserActionRequired> for &str         // verbose sentences (see §2.2)
pub enum StagedAction { WaitLogout, StopDisplayManager, StartDisplayManager, NoLogind, LoadGpuDrivers, UnloadGpuDrivers, KillNvidia, KillAmd, EnableNvidiaPowerd, DisableNvidiaPowerd, LoadVfioDrivers, UnloadVfioDrivers, DevTreeManaged, RescanPci, UnbindRemoveGpu, UnbindGpu, HotplugUnplug, HotplugPlug, AsusDgpuDisable, AsusDgpuEnable, AsusEgpuDisable, AsusEgpuEnable, AsusMuxIgpu, AsusMuxDgpu, WriteModprobeConf, CheckVulkanIcd, NotNvidia, None }  // main adds Enable/DisableNvidiaPersistenced
pub enum Action { UserAction(UserActionRequired), StagedActions(Vec<StagedAction>) }

// src/config.rs
pub struct GfxConfigDbus { pub mode: GfxMode, pub vfio_enable: bool, pub vfio_save: bool, pub always_reboot: bool, pub no_logind: bool, pub logout_timeout_s: u64, pub hotplug_type: HotplugType }   // signature (ubbbbtu)
pub struct GfxConfig { config_path (skip), mode, tmp_mode (skip), pending_mode (skip), pending_action (skip), vfio_enable, vfio_save, always_reboot, no_logind, logout_timeout_s, hotplug_type }

// src/error.rs
pub enum GfxError { ParseVendor, ParseMode, DgpuNotFound, Udev(String, io::Error), SystemdUnitAction(String), SystemdUnitWaitTimeout(String), AsusGpuMuxModeDiscreet, VfioBuiltin, VfioDisabled, MissingModule(String), Modprobe(String), Command(String, io::Error), Path(String, io::Error), Read(..), Write(..), NotSupported(String), Io(PathBuf, io::Error), Zbus(zbus::Error), ZbusFdo(zbus::fdo::Error), IncorrectActionOrder(StagedAction, StagedAction) }

// src/special_asus.rs
pub enum AsusGpuMuxMode { Discreet, Optimus }  // sysfs '0' → Discreet, anything else → Optimus
pub fn asus_gpu_mux_exists() / asus_gpu_mux_mode() / asus_gpu_mux_set_igpu(bool)
pub fn asus_dgpu_disable_exists() / asus_dgpu_disabled() / asus_dgpu_set_disabled(bool)
pub fn asus_egpu_enable_exists() / asus_egpu_enabled() / asus_egpu_set_enabled(bool)
pub async fn asus_boot_safety_check(mode, asus_use_dgpu_disable) -> Result<GfxMode>
```

Serde: serialised with **serde default representation** (unit variants as strings, e.g. `"mode": "Hybrid"`, `"hotplug_type": "None"`) in JSON; as **u32** on D-Bus (zvariant). Both from the same derives.

---

## 4. `/etc/supergfxd.conf`

JSON, written pretty-printed by the daemon at every load (`GfxConfig::load` → `write()`), so comments/extra keys are lost. Defaults (`GfxConfig::new`):

```json
{
  "mode": "Hybrid",
  "vfio_enable": false,
  "vfio_save": false,
  "always_reboot": false,
  "no_logind": false,
  "logout_timeout_s": 180,
  "hotplug_type": "None"
}
```

| Key | Type | Semantics in 5.2.7 |
|---|---|---|
| `mode` | `"Hybrid" \| "Integrated" \| "NvidiaNoModeset" \| "Vfio" \| "AsusEgpu" \| "AsusMuxDgpu" \| "None"` (serde name, case-sensitive) | The mode applied at boot (`reload()` → `do_boot_tasks`) and reported by `Mode`. Updated only after a successful switch. Overridden at boot by kernel cmdline `supergfxd.mode=<Mode>` (also case-sensitive via `FromStr`, despite README), which is then written back to the file. `asus_boot_safety_check` may override it based on `gpu_mux_mode`/`dgpu_disable`/`egpu_enable` state (e.g. MUX discreet → `AsusMuxDgpu`; `dgpu_disable=1` → `Integrated`; `egpu_enable=1` → `AsusEgpu`; MUX optimus but mode `AsusMuxDgpu` → `Hybrid`). |
| `vfio_enable` | bool | Adds `Vfio` to `Supported`; at boot, `mode: "Vfio"` is refused ("Tried to set vfio mode but it is not enabled") if false. **`SetMode(Vfio)` is *not* gated by it** (only `mode_support_check` runs, which checks eGPU only) — the GUI should gate on `Supported`. Requires vfio modules built as modules, not built-in (`VfioBuiltin` error otherwise). |
| `vfio_save` | bool | **Dead in 5.2.7**: read/written and exposed via `Config`, but never consulted (grep shows no reader). A successful `SetMode(Vfio)` always persists `mode: "Vfio"`. README claims it controls persistence; it does not. |
| `always_reboot` | bool | `SetMode` always returns `Reboot`; the logind wait and display-manager stop/start are replaced by no-ops (`NoLogind` marker) but **the remaining actions still run immediately**. README: "helps some laptops". |
| `no_logind` | bool | Same effect on the action list as `always_reboot` (no `WaitLogout`/`Stop|StartDisplayManager`) but the returned action still says `Logout`. Also disables the daemon's logind `PrepareForSleep` listener (used only with `hotplug_type: Asus` to re-assert `dgpu_disable` after resume). README: for people without a login manager. |
| `logout_timeout_s` | u64 | **Ignored in 5.2.7** — the wait is hard-coded to 30 s (`src/actions.rs` `let logout_timeout_s = 30;`). README/ArchWiki say default 3 min, 0 = infinite; that only describes the (unused) config field. |
| `hotplug_type` | `"None" \| "Std" \| "Asus"` | `None`: manage the dGPU purely via sysfs unbind/remove (`DevTreeManaged`). `Std`: additionally write `0`/`1` to `/sys/bus/pci/slots/<slot>/power` if a hotplug slot matching the dGPU exists. `Asus`: use `/sys/devices/platform/asus-nb-wmi/dgpu_disable` (hard ACPI disable; device vanishes from the bus; re-asserted after resume; if the sysfs file is missing the daemon writes `/etc/modules-load.d/asus.conf` and waits up to 2 s for it). **Changing it requires a reboot** (README) and a daemon restart. ArchWiki recommends `Asus` for VFIO users on ASUS. |

Loading is strict: if the JSON does not deserialise as the current struct (e.g. a key is missing — no `#[serde(default)]`), the daemon tries the 3.0.0/4.0.5/5.0.0 legacy layouts and otherwise **recreates the file with defaults** ("Could not deserialise …, recreating"). Any manual edit needs `systemctl restart supergfxd` (config is read once at startup).

Omarchy's `omarchy-toggle-hybrid-gpu` writes this file (`vfio_enable: true`, everything else default) before first starting the service "to prevent hang on first boot".

---

## 5. Requirements and runtime behaviour

### 5.1 Daemon start-up sequence (`daemon.rs` + `controller.rs`)

1. Connect to system bus, `request_name`.
2. Load config (creating it if absent; `/etc` must exist or it panics).
3. If `!no_logind`: spawn logind `PrepareForSleep` watcher.
4. `CtrlGraphics::new` → `DiscreetGpu::new()`: `echo 1 > /sys/bus/pci/rescan`, then enumerate PCI via udev looking for vendor `10DE` (NVIDIA) or `1002` (AMD) with `PCI_CLASS` starting `30`. A device is the dGPU if its DRM connectors do **not** include a connected `eDP-1` (5.2.7 "new method via checking the GPU port connection names"; the old `boot_vga` method was removed). Fallbacks: AMD `hwmon/*/in1_input` absence, then `ID_MODEL_FROM_DATABASE` / `lspci -d` label matching `Radeon RX|AMD/ATI|GeForce|Geforce|Quadro|T1200` (issue #179: RTX A1000 not matched). Sibling functions (audio, USB-C…) with the same parent are tracked as "additional devices" for unbind/VFIO. If nothing is found: `vendor = Unknown`, or `AsusDgpuDisabled` if `dgpu_disable=1`, or `Nvidia` if MUX is discreet. `CtrlGraphics::new` only fails if the PCI rescan write fails (root-only) — then the object is **never registered** but the daemon keeps running and owning the name.
5. `reload()`: read `supergfxd.mode=` from `/proc/cmdline`; refuse Vfio if `!vfio_enable`, refuse AsusEgpu if no `egpu_enable`; `asus_boot_safety_check`; run `action_list_for_boot(mode)`: Hybrid = `WriteModprobeConf, CheckVulkanIcd, <hotplug_add>, RescanPci, LoadGpuDrivers, EnableNvidiaPowerd`; Integrated/NvidiaNoModeset = `DisableNvidiaPowerd, Kill*, UnloadGpuDrivers, UnbindRemoveGpu, WriteModprobeConf, CheckVulkanIcd, <hotplug_rm>`; Vfio = `DisableNvidiaPowerd, Kill*, UnloadGpuDrivers, WriteModprobeConf, CheckVulkanIcd, LoadVfioDrivers`; AsusEgpu/AsusMuxDgpu = `WriteModprobeConf, CheckVulkanIcd, LoadGpuDrivers, EnableNvidiaPowerd`. Then `power/control = auto` on all tracked devices. Errors are logged, not fatal.
6. Start the 1 s `NotifyGfxStatus` poller, register the object, `request_name` again, then sleep forever.

The unit is `Before=display-manager.service`, so on an Integrated boot the dGPU is unbound/removed before the DM starts. Omarchy adds `ExecStartPre=/bin/sleep 5` (drop-in `delay-start.conf`) in Integrated mode because "the system can freeze on boot because supergfxd tries to disable the dGPU while the display subsystem is still initializing"; issue #181 is the mirror case at shutdown (KWin holding `nvidia_drm`).

### 5.2 Desktops / machines without hybrid graphics

- The daemon **starts and stays running** regardless of hardware; `Version`, `Mode`, `Supported`, `Vendor`, `PendingMode`, `PendingUserAction`, `Config` always answer.
- Intel-only or no NVIDIA/AMD GPU: `Vendor = "Unknown"`, `Supported = [Integrated]` (1 entry), `Mode = Hybrid` (config default — nonsense but harmless), `Power` → D-Bus error `Failed "GFX fail: get_runtime_status: Could not find dGPU"`, `NotifyGfxStatus` never emitted. `SetMode(Integrated)` "succeeds" and merely writes the config.
- **Desktop with a single NVIDIA/AMD card or any desktop with two GPUs (e.g. this machine: AMD Raphael iGPU + RTX 4090)**: there is no `eDP-1`, so *every* NVIDIA/AMD display device is classified as a dGPU; `Vendor` will be `Nvidia`/`AMD`, `Supported = [Integrated, Hybrid]`, and on boot in Hybrid the daemon writes `/etc/modprobe.d/supergfxd.conf` (blacklists nouveau, forces `nvidia-drm modeset=1`). **`SetMode(Integrated)` would stop the DM, kill every `/dev/nvidia0` user, rmmod nvidia, unbind and remove the primary card.** The daemon has no laptop/desktop guard. A GUI must not offer Integrated on such systems: check for an `eDP-*`/`LVDS-*` connector (`/sys/class/drm/card*-eDP-1`), chassis type (`/sys/class/dmi/id/chassis_type` 8/9/10/14/31/32), or `lspci` count as Omarchy's `omarchy-hw-hybrid-gpu` does (`supergfxctl -s` must contain `Hybrid`, or `lspci | grep -cE 'VGA|3D|Display' >= 2`).
- ASUS laptop with `dgpu_disable=1` at boot: `Vendor = "ASUS dGPU disabled"`, `Power = AsusDisabled`, `Supported` still lists `Integrated, Hybrid` (+ASUS modes) because `asus_dgpu_disable_exists()`.

### 5.3 NVIDIA requirements

- Proprietary `nvidia` driver modules (`nvidia`, `nvidia_modeset`, `nvidia_uvm`, `nvidia_drm`) as **loadable modules**; the daemon `rmmod`s/`modprobe`s them. Built-in modules cannot be switched. Nouveau is blacklisted by the generated modprobe file in all NVIDIA modes; issue #175 reports nouveau still loading in Integrated on some setups.
- `nvidia-drm.modeset=1` is forced via `/etc/modprobe.d/supergfxd.conf` (`options nvidia-drm modeset=1`) in Hybrid/Integrated/AsusEgpu/NvidiaNoModeset. Since 5.0.0 "nvidia.modeset=0 not required for rebootless switching". If the kernel cmdline says `nvidia-drm.modeset=0`, `Supported` additionally offers `NvidiaNoModeset` (hot-unload-capable mode for GA401I-era hardware). ArchWiki: for NVIDIA laptops follow NVIDIA#DRM kernel mode setting (early KMS / initramfs) — the modprobe file must be included in the initramfs or the boot-time blacklist will not take effect until late.
- Runtime PM: the shipped udev rule sets `power/control=auto` on driver bind (and `on` on unbind); the daemon also writes `auto` after boot tasks. Fine-grained D3 (`options nvidia NVreg_DynamicPowerManagement=0x02`) is **optional** and must be put in a modprobe file *other than* `supergfxd.conf` (which is overwritten). `nvidia-powerd.service` is started in Hybrid and stopped in Integrated/Vfio (`main` does the same for `nvidia-persistenced.service`).
- `lsof` must be installed (else "hogging" processes are not killed — daemon only warns).
- Rebootless switching may need `KillUserProcesses=yes` in `/etc/systemd/logind.conf` (README), because lingering user processes keep `/dev/nvidia*` open after logout.
- Conflicts: optimus-manager, suse-prime, ubuntu-prime, system76-power, bbswitch, envycontrol-style modprobe/xorg leftovers in `/etc/modprobe.d`, `/usr/lib/modprobe.d`, `/etc/X11/xorg.conf.d`, `/etc/udev/rules.d`. Stray nvidia blacklists make supergfxd "always default to integrated".
- AMD dGPU: only Hybrid/Integrated/Vfio; no modprobe file is written; `KillAmd` is a no-op; issue #170/#180 report amdgpu/ttm crashes and RCU stalls on unbind.
- Backlight: AMD iGPU + NVIDIA with `hotplug_type: Asus` can lose backlight control after switching (issue #158); README suggests `acpi_backlight=native`; `main` adds `nvidia_wmi_ec_backlight` handling.
- ASUS laptops need kernel ≥ 6.1 (for `gpu_mux_mode`/`dgpu_disable`), and **< 6.19** for the ASUS-specific modes with 5.2.7 (see §0).

### 5.4 Interplay with asusd (asusctl) — MUX and dgpu_disable

- Both daemons touch the same firmware knobs. supergfxd 5.2.7 reads/writes `/sys/devices/platform/asus-nb-wmi/{gpu_mux_mode,dgpu_disable,egpu_enable}` directly. asusctl ≥ 6.x exposes them via the `asus-armoury` firmware-attributes interface: D-Bus `xyz.ljones.AsusArmoury` objects at `/xyz/ljones/asus_armoury/<attr>` (`gpu_mux_mode`, `dgpu_disable`, `egpu_enable`, `egpu_connected`, `pending_reboot`, …) with properties `CurrentValue` (i32, `PropertiesChanged` emitted), `PossibleValues`, `MinValue`/`MaxValue`, `Name`, and reads sysfs `/sys/class/firmware-attributes/asus-armoury/attributes/<attr>/current_value` with a fallback to the legacy `asus-nb-wmi` path (rog-platform `gpu_pci.rs`). asusd treats the GPU attributes as **queued**: `SetCurrentValue` on `gpu_mux_mode`/`dgpu_disable`/`egpu_enable` stores the value and applies it at shutdown (`queued_gpu` in `asusd/src/asus_armoury.rs`), so `CurrentValue` does not change until the next boot. The old asusctl 5.x/6.x interface `org.asuslinux.Platform`/`/org/asuslinux/Platform` `GpuMuxMode` property is gone in 6.4.
- `gpu_mux_mode` semantics (both tools): `0` = dGPU/discrete ("Ultimate"), `1` = Optimus/iGPU. supergfxd: `AsusGpuMuxMode::from(c)`: `'0'` → Discreet, anything else → Optimus.
- When the MUX is discrete, supergfxd's `Mode` reports `AsusMuxDgpu`, `Power` reports `AsusMuxDiscreet`, and 5.2.7's `Supported` reports **only** `[AsusMuxDgpu]` (issue #171, fixed on `main` to `[AsusMuxDgpu, Integrated, Hybrid]`); `SetMode(anything else)` runs only `AsusMuxIgpu` (writes `1`) and returns `Reboot`; the requested mode is saved to config immediately (the background task marks success) and is applied by the boot action list after reboot. Every mode other than `AsusMuxDgpu` "should assume that either the ASUS specific gpu_mux_mode sysfs entry is not available or is set to iGPU mode" (pci_device.rs).
- Boot sanity (`asus_boot_safety_check`): MUX discrete + `dgpu_disable=1` → it re-enables the dGPU ("can't continue safely"); `dgpu_disable=1` while `hotplug_type != Asus` → it clears `dgpu_disable` and forces Hybrid (or Integrated on failure); `egpu_enable=1` → forces `AsusEgpu`. Windows dual-boot changes to these BIOS values are picked up this way.
- README's advice: if you only ever need Hybrid + the MUX toggle, **use asusctl instead of supergfxctl**. ArchWiki: "Using the MUX switch requires that you are running asusctl … AsusMuxDgpu mode can be switched with asusctl". For a GUI on ASUS hardware the robust path for the MUX is asusd's armoury property, and supergfxd only for Integrated/Hybrid/Vfio.
- Both daemons start/stop `nvidia-powerd.service` (asusd restarts it when `nv_*` TDP attributes change) — harmless overlap.

### 5.5 Hotplug and eGPU handling

- `hotplug_type: Std` uses the PCIe hotplug slot (`/sys/bus/pci/slots/*/address` matching the dGPU address; `power` file written `0`/`1`). Only some laptops expose a slot; otherwise the action is silently skipped.
- `hotplug_type: Asus` uses `dgpu_disable` (ACPI hard-disable); the device disappears from the bus, `Vendor` becomes "ASUS dGPU disabled" on next start, and the daemon re-writes `dgpu_disable=1` after resume (logind `PrepareForSleep(false)`) when the mode is Integrated. Issue #174: some laptops re-enumerate the dGPU after resume anyway (Omarchy's `force-igpu` sleep hook works around this by doing `Vfio` → `Integrated` after resume, and `Vfio` before hibernate).
- eGPU (`AsusEgpu`): only on ASUS Flow models with `/sys/devices/platform/asus-nb-wmi/egpu_enable` (or `/sys/bus/platform/devices/asus-nb-wmi/egpu_enable`). The user must plug in and flip the switch **before** `SetMode(AsusEgpu)`; enabling the eGPU also disables the internal dGPU (ACPI), so leaving `AsusEgpu` runs `AsusEgpuDisable` then `AsusDgpuEnable`. Kernel 6.19 moved `egpu_enable` too (issue #177). The kernel driver logs "a reboot is strongly advised" after toggling.

---

## 6. Suggested client definition for a Rust GUI (zbus 5)

```rust
use serde::{Deserialize, Serialize};
use zbus::{proxy, zvariant::Type};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[repr(u32)]
pub enum GfxMode { Hybrid, Integrated, NvidiaNoModeset, Vfio, AsusEgpu, AsusMuxDgpu, None }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum GfxPower { Active, Suspended, Off, AsusDisabled, AsusMuxDiscreet, Unknown }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum UserActionRequired { Logout, Reboot, SwitchToIntegrated, AsusEgpuDisable, Nothing }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum HotplugType { Std, Asus, None }
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GfxConfigDbus { pub mode: GfxMode, pub vfio_enable: bool, pub vfio_save: bool,
    pub always_reboot: bool, pub no_logind: bool, pub logout_timeout_s: u64, pub hotplug_type: HotplugType }

#[proxy(interface = "org.supergfxctl.Daemon",
        default_service = "org.supergfxctl.Daemon",
        default_path = "/org/supergfxctl/Gfx")]
pub trait SuperGfx {
    fn version(&self) -> zbus::Result<String>;
    fn mode(&self) -> zbus::Result<GfxMode>;
    fn supported(&self) -> zbus::Result<Vec<GfxMode>>;
    fn vendor(&self) -> zbus::Result<String>;
    fn power(&self) -> zbus::Result<GfxPower>;
    fn set_mode(&self, mode: GfxMode) -> zbus::Result<UserActionRequired>;
    fn pending_mode(&self) -> zbus::Result<GfxMode>;
    fn pending_user_action(&self) -> zbus::Result<UserActionRequired>;
    fn config(&self) -> zbus::Result<GfxConfigDbus>;
    #[zbus(signal)] fn notify_gfx_status(&self, status: GfxPower) -> zbus::Result<()>;
    #[zbus(signal)] fn notify_gfx(&self, vendor: GfxMode) -> zbus::Result<()>;
    #[zbus(signal)] fn notify_action(&self, action: UserActionRequired) -> zbus::Result<()>;
}
// Build with .cache_properties(CacheProperties::No) (there are no properties) and
// consider zbus::Connection::system() + proxy.inner().connection().  Use tokio timeouts on every call.
```

Unit-variant enums with `#[derive(Type)]` and no `signature` attribute serialise as `u`, matching the daemon. Unknown future values (e.g. if the daemon ever adds a variant) would fail to deserialise; decode as `u32` and map manually if you want to be defensive.

Manual checks from a shell:

```
busctl call org.supergfxctl.Daemon /org/supergfxctl/Gfx org.supergfxctl.Daemon Supported      # → au 2 1 0
busctl call org.supergfxctl.Daemon /org/supergfxctl/Gfx org.supergfxctl.Daemon SetMode u 1     # → u 0 (Logout)
busctl monitor org.supergfxctl.Daemon                                                          # watch NotifyGfxStatus etc.
```

---

## 7. CLI (`supergfxctl`) as a fallback

`src/cli.rs`, gumdrop options; it is a thin `DaemonProxyBlocking` client with `CacheProperties::No`.

```
supergfxctl --help
Optional arguments:
  -h, --help         print help message
  -m, --mode         Set graphics mode           (value parsed with GfxMode::from_str: Hybrid|Integrated|NvidiaNoModeset|Vfio|AsusEgpu|AsusMuxDgpu, case-sensitive)
  -v, --version      Get supergfxd version       → "5.2.7"
  -g, --get          Get the current mode        → Display of GfxMode: "Hybrid" … / "Unknown" for None
  -s, --supported    Get the supported modes     → Debug of Vec: e.g. "[Integrated, Hybrid, Vfio]"
  -V, --vendor       Get the dGPU vendor name    → "Nvidia" | "AMD" | "Intel" | "Unknown" | "ASUS dGPU disabled"
  -S, --status       Get the current power status → "active" | "suspended" | "off" | "dgpu_disabled" | "asus_mux_discreet" | "unknown"
  -p, --pend-action  Get the pending user action if any → verbose sentence, e.g. "No action required"
  -P, --pend-mode    Get the pending mode change if any → mode name or "Unknown"
```

Flags can be combined; they are executed in the order `-m, -v, -g, -s, -V, -S, -p, -P`. With no flags (or `-h`) it prints usage and still connects to the bus. `-m` output: `Graphics mode changed to X. Required user action is: Logout required to complete mode change` / `Graphics mode changed to X` (Nothing) / `A reboot is required to complete the mode change` / `AsusEgpuDisable`; for `SwitchToIntegrated` it prints `You must change to Integrated before you can change to X` to stderr and **exits 1**. On any D-Bus error it prints "Graphics mode change error." and a hint (`supergfxd is not enabled…` / `…not running…` / `Please check journalctl -b -u supergfxd`), the daemon's error text in red, then exits 1. Note `-m` returns as soon as the daemon accepted the request — the switch continues in the background; Omarchy's hook polls `supergfxctl -g` afterwards to confirm. All CLI calls block for as long as the D-Bus call blocks (see §8.2) — wrap in `timeout`, as Omarchy does (`timeout --kill-after=1s 3s supergfxctl -g`).

---

## 8. Pitfalls for GUI integration

1. **No properties, no `PropertiesChanged`.** Everything is a method call. The only spontaneous signal is `NotifyGfxStatus` (dGPU power, 1 s daemon-side poll, only on change). Subscribe to it for the power indicator; for mode/pending state you must poll (`Mode`, `PendingMode`, `PendingUserAction`) — the KDE plasmoid polls `Version`, `Mode`, `Power`, `PendingUserAction`, `PendingMode` every 1 s and refreshes `Supported` on daemon state changes; the GNOME extension (`supergfxctl-gex`) instead re-reads `Mode` inside its `NotifyAction` handler and listens to `NotifyGfxStatus`.
2. **`NotifyGfx`/`NotifyAction` mean "requested", not "done".** They are emitted synchronously inside `SetMode` before any work happens, with the requested mode. There is **no completion event**: poll `PendingMode` until it returns `None` (6), then read `Mode`; if `Mode` still equals the old mode the switch failed (check `journalctl -u supergfxd`).
3. **Stale pending state.** If `SetMode` short-circuits (returns `SwitchToIntegrated`, `AsusEgpuDisable`, or `Nothing` for a same-mode/no-op request), `PendingMode`/`PendingUserAction` keep the just-set values forever (until the next real switch finishes). Do not treat `PendingMode != None` alone as "switch in progress" — combine it with the `SetMode` return value and/or the fact that `Mode` changed.
4. **`SetMode` is quick, but the daemon can wedge every other call afterwards.** The background task holds the `dgpu` mutex while performing each action, including `WaitLogout` (up to 30 s) and `StopDisplayManager` (up to 3 s + systemctl). `Power` locks that same mutex and **hangs** meanwhile; the 1 s status poller stalls too. A second `SetMode` needs the interface write-lock **and** the dgpu lock, so it — and, because of the write lock, *all* other methods — block until the pending switch finishes. Always call with a timeout (Omarchy uses 1–3 s and treats a timeout as "daemon wedged"). Prefer `Mode`/`PendingMode` (config-lock only) for polling during a switch; avoid `Power` until `PendingMode` is `None`.
5. **The 30 s logout window is real and hard-coded** (config `logout_timeout_s` is ignored). After `SetMode` returns `Logout`, the user must fully log out within 30 s or the switch aborts (`SystemdUnitWaitTimeout`, "Time (30 seconds) for logout exceeded") and a buggy rollback runs. A GUI should trigger the logout itself (e.g. `loginctl terminate-session` / compositor exit) immediately after confirming with the user — and remember lingering processes holding `/dev/nvidia0` get `kill -9`'d.
6. **Sessions of type `tty` are not waited for.** Compositors launched from a tty login (no display manager) are invisible to `WaitLogout`; the daemon will proceed immediately, `systemctl stop display-manager.service` fails (exit 5 if the alias does not exist → switch marked failed → rollback), and the drivers are unloaded under a live session. Verify with `loginctl show-session <id> -p Type` before offering rebootless switching; if `Type=tty` or there is no `display-manager.service`, use the Omarchy strategy: edit `/etc/supergfxd.conf` `mode` and reboot.
7. **`Mode` is config, not hardware.** It reports the persisted mode (or `AsusMuxDgpu` when the MUX is discrete). After a failed switch or a boot sanity override it may not match what the drivers are doing; the modprobe file, `lsmod`, and `Power` are the ground truth.
8. **`Supported` is the authority for what to show**, but `SetMode(Vfio)` is accepted even when `vfio_enable` is false, and Integrated/Hybrid are offered on desktops (see §5.2). Add your own guards. Under a discrete MUX, 5.2.7 offers only `AsusMuxDgpu` — still allow the user to pick Integrated/Hybrid (they work, reboot required), as `main` now does.
9. **Enum values are `u32`, and the value set differs from older majors** (4.x had `Dedicated`, `Compute`, `Egpu`, and different indices; 5.0.x had `Compute`). Gate on `Version() >= "5.1.0"` before trusting the tables in §2.2 (gex does the same: `sgfxLastState = 7` for ≥5.1.1).
10. **Bus policy.** Non-root users must be in `adm`, `sudo`, `users` or `wheel`, otherwise every call returns `AccessDenied`; surface this distinctly from "daemon not running" (`ServiceUnknown`) and "daemon starting" (`UnknownObject`/`UnknownMethod`).
11. **Timing expectations.** Integrated↔Vfio: ~1–3 s (modprobe retries, 500 ms ASUS sleeps, PCI rescan). Hybrid↔Integrated: dominated by the user's logout + DM restart; the daemon's own work is a few seconds; DM stop is polled for ≤3 s. MUX changes: instant write + reboot. Boot-time actions in Integrated can take several seconds (plus Omarchy's 5 s delay) before the object appears on the bus.
12. **Side effects to warn about.** Switching rewrites `/etc/modprobe.d/supergfxd.conf`, renames the NVIDIA Vulkan ICD, starts/stops `nvidia-powerd`, `kill -9`s `/dev/nvidia0` users, and unbinds PCI functions; hibernate with a powered-off dGPU can fail to resume (Omarchy's `force-igpu` hook switches to `Vfio` before hibernate for this reason).
13. **`Config`/`SetConfig` are effectively read-only/broken** (§2.3); persist config changes by editing the JSON as root and restarting the unit.
14. **Deprecated upstream; kernel 6.19+ regressions for ASUS modes.** Design the backend behind a trait so `cardwire` (or asusd's armoury properties for the MUX) can replace it.

---

## 9. Omarchy-specific notes (this machine)

- Omarchy ships `supergfxctl 5.2.7-2` in its own repo and two scripts: `omarchy-hw-hybrid-gpu` (hybrid detection: `timeout 1s supergfxctl -s` contains `Hybrid`, else `lspci` count ≥ 2) and `omarchy-toggle-hybrid-gpu` (Hybrid⇄Integrated). The toggle **never calls `SetMode`**; it rewrites `"mode"` in `/etc/supergfxd.conf` with `sed`, installs/removes `/etc/systemd/system/supergfxd.service.d/delay-start.conf` (`ExecStartPre=/bin/sleep 5`) and `/usr/lib/systemd/system-sleep/force-igpu`, then reboots. `vfio_enable` is set to `true` so the sleep hook can use `Vfio` as an intermediate state (`supergfxctl -m Vfio` then `-m Integrated` after resume; `-m Vfio` before hibernate), polling `supergfxctl -g` up to 10 s for confirmation.
- This box is a desktop (AMD Raphael iGPU + RTX 4090, no `asus-nb-wmi`), supergfxctl is **not installed**, `sddm.service` is the `display-manager.service` alias, and the graphical logind session is `Type=wayland Class=user` (so `WaitLogout` would see it; the tty caveat in §8.6 applies only to DM-less setups). Per §5.2, supergfxd would misclassify the 4090 as a laptop dGPU here.

---

## 10. Sources

- Repo (archived): https://gitlab.com/asus-linux/supergfxctl — files read: `Cargo.toml`, `Cargo.lock`, `README.md`, `CHANGELOG.md`, `Makefile`, `src/{lib,daemon,cli,controller,zbus_iface,zbus_proxy,actions,config,pci_device,special_asus,systemd,error}.rs`, `src/tests/actions.rs`, `data/{supergfxd.service,supergfxd.preset,org.supergfxctl.Daemon.conf,90-supergfxd-nvidia-pm.rules,99-nvidia-ac.rules,90-nvidia-screen-G05.conf}` at refs `main` and `5.2.7` (raw: `https://gitlab.com/asus-linux/supergfxctl/-/raw/<ref>/<path>`). Tags via `https://gitlab.com/api/v4/projects/asus-linux%2Fsupergfxctl/repository/tags`.
- Captured introspection XML: https://gitlab.com/asus-linux/supergfxctl-gex/-/raw/main/resources/dbus/org-supergfxctl-gfx-5.xml ; client code https://gitlab.com/asus-linux/supergfxctl-gex/-/raw/main/src/modules/dbus.ts
- KDE plasmoid client: https://raw.githubusercontent.com/Jhyub/supergfxctl-plasmoid/master/src/DaemonController.cpp
- zvariant enum encoding: https://docs.rs/zvariant/latest/zvariant/derive.Type.html ("w/o repr attribute, u32 representation is chosen")
- Issues: #171 https://gitlab.com/asus-linux/supergfxctl/-/issues/171 (MUX → only AsusMuxDgpu supported), #178 (kernel 6.19 gpu_mux_mode path), #177 (6.19 eGPU), #158, #170, #173–#176, #179–#181 via `https://gitlab.com/api/v4/projects/asus-linux%2Fsupergfxctl/issues`
- Kernel 6.19 regression: https://github.com/CachyOS/linux-cachyos/issues/711 ; kernel driver `drivers/platform/x86/asus-armoury.c` (attributes `gpu_mux_mode`, `dgpu_disable`, `egpu_enable`, `pending_reboot`)
- asusctl: https://github.com/flukejones/asusctl — `asusd/src/asus_armoury.rs` (`xyz.ljones.AsusArmoury`, queued GPU values), `rog-platform/src/asus_armoury.rs` (`/sys/class/firmware-attributes/asus-armoury/attributes/`), `rog-platform/src/gpu_pci.rs` (dual-path lookup)
- ArchWiki: https://wiki.archlinux.org/title/Supergfxctl ; manual: https://asus-linux.org/manual/supergfxctl-manual/
- Successor: https://opengamingcollective.github.io/cardwire/ ; Omarchy discussion https://github.com/omacom/omarchy/discussions/5982
- AUR PKGBUILD: https://aur.archlinux.org/cgit/aur.git/plain/PKGBUILD?h=supergfxctl ; crates.io: https://crates.io/api/v1/crates/supergfxctl (404)
- Local: `/usr/share/omarchy/bin/omarchy-toggle-hybrid-gpu`, `/usr/share/omarchy/bin/omarchy-hw-hybrid-gpu`, `/usr/share/omarchy/default/systemd/system/supergfxd.service.d/delay-start.conf`, `/usr/share/omarchy/default/systemd/system-sleep/force-igpu`
