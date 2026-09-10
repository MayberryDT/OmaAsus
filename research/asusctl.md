# asusctl / asusd — technical reference for a Rust D-Bus client

Researched 2026-09-09 from the actual sources. Two repositories exist:

| Repo | Latest tag | Status |
|---|---|---|
| https://gitlab.com/asus-linux/asusctl | **6.3.8** (2026-06-02, commit `321b1114`) | Frozen. README: "This project has been migrated to the OGC on GitHub and future development will happen there." |
| https://github.com/OpenGamingCollective/asusctl | **6.4.0** (2026-08-14), `main` pushed 2026-09-09 | Active. Tags 6.3.9 (07-18), 6.3.10 (07-23), 6.3.11 (07-29), 6.4.0 (08-14). Arch `extra/asusctl` is **6.4.0-3**. |

Everything below is for the 6.3.8 tree unless marked **[6.4.0]**; every diff between 6.3.8 and GitHub `main` that affects the D-Bus surface is listed in §3.4. Source citations use `gitlab:<path>` (= `https://gitlab.com/asus-linux/asusctl/-/raw/6.3.8/<path>`) and `gh:<path>` (= `https://raw.githubusercontent.com/OpenGamingCollective/asusctl/main/<path>`).

Both trees were cloned and diffed locally; the machine this was researched on (ROG CROSSHAIR X670E EXTREME desktop, kernel 7.2.3) does **not** run asusd, so the introspection XML in §1.9 is reconstructed from the `#[interface]` impls rather than captured with `busctl`. See §5 for what actually happens on a desktop board.

---

## 0. TL;DR for the GUI

* Bus: **system bus**, well-known name **`xyz.ljones.Asusd`** (since 6.1.0-rc2, Jan 2025). Before that: `org.asuslinux.Daemon`.
* Root path `/` implements `org.freedesktop.DBus.ObjectManager` — **use `GetManagedObjects` to discover objects**; the `default_path` values baked into the `rog_dbus` proxies are partly stale.
* Fixed paths: `/xyz/ljones` (Platform, FanCurves, Backlight, XgmLed), `/xyz/ljones/asus_armoury/<sysfs_attr_name>` (one AsusArmoury object per firmware attribute). Hot-plug paths: `/xyz/ljones/aura/<...>` (Aura, Anime, Slash, ScsiAura).
* Enums cross the wire as **`u` (serde positional variant index)** unless the type carries `#[zvariant(signature = "s")]`, in which case they are the **variant name string**. See §2.3 for the exact table — note that `AuraModeNum::Pulse/Comet/Flash` are 9/10/11 on the wire, not their `= 10/11/12` discriminants.
* All "tunables" (ppt_*, nv_*, panel_od, mini_led_mode, boot_sound, gpu_mux_mode, dgpu_disable, egpu_enable, mcu_powersave, charge_mode, screen_auto_brightness, apu_mem, cores_*) are **not** Platform properties any more; each is an `xyz.ljones.AsusArmoury` object with `CurrentValue`/`MinValue`/`MaxValue`/`DefaultValue`/`PossibleValues`/`ScalarIncrement`. PPT-class values are stored **per (profile × AC/DC)** and only written to sysfs when `Platform.EnablePptGroup` is true. GPU-class values are **queued** and applied by `asus-shutdown.service` at shutdown.
* `rog_dbus` is **not on crates.io**; it must be a git dependency and it pulls the whole `asusd` crate (udev/libusb/inotify). Writing your own `#[proxy]` traits from §1 is lighter and lets you support both `org.asuslinux.*` and `xyz.ljones.*`.
* `asusctl` (CLI) hard-refuses to run when its version string differs from `Platform.Version` — a CLI fallback must be the same version as the installed daemon.
* On a desktop (no `asus-nb-wmi` platform device) `asusd` **exits during startup** (`RogPlatform::new()?` in `daemon.rs`) and the udev rule never starts it; there is no D-Bus service to talk to at all. See §5.

---

## 1. D-Bus surface (6.3.8, `xyz.ljones.*`)

### 1.1 Names, policy, activation

| Item | Value | Source |
|---|---|---|
| Bus | system | `gitlab:asusd/src/daemon.rs` (`Connection::system()`) |
| Well-known name | `xyz.ljones.Asusd` (`asusd::DBUS_NAME`) | `gitlab:asusd/src/lib.rs` |
| `DBUS_PATH` const | `/xyz/ljones/Daemon` — **nothing is registered there** (legacy constant) | same |
| `DBUS_IFACE` const | `xyz.ljones.Asusd` — only used by **asusd-user on the session bus** (§4.2) | same |
| Base path const | `ASUS_ZBUS_PATH = "/xyz/ljones"` | same |
| ObjectManager | `org.freedesktop.DBus.ObjectManager` at `/` (registered before any controller) | `daemon.rs` |
| Name request | done **last**, after all controllers are registered, so `NameOwnerChanged` implies a complete object tree | `daemon.rs` |
| D-Bus policy | `/usr/share/dbus-1/system.d/asusd.conf`: groups `adm`, `sudo`, `users`, `wheel` may send/receive; `root` owns | `gitlab:data/asusd.conf` |
| systemd | `asusd.service`: `Type=dbus`, `BusName=xyz.ljones.Asusd`, `Environment=IS_SERVICE=1`, hardened (`ProtectSystem=strict`, `ReadWritePaths=/etc/asusd/ /sys/`), `ConfigurationDirectory=asusd`; **not** enabled directly — pulled in by udev (`SYSTEMD_WANTS`) | `gitlab:data/asusd.service`, `gitlab:data/asusd.rules` |
| udev rule | `99-asusd.rules`: `DMI_VENDOR` must match `ASUS*` **and** `product_family` must match one of `*TUF*`, `*ROG*`, `*Zephyrus*`, `*Strix*`, `*Vivo*ook*`, `*ASUSLaptop*`, `*Zen*ook*`, `*ProArt*`, `*TX Air*`, `*TX Gaming*`, `*EXPERTBOOK*`; then `DRIVER=="asus-nb-wmi"` → wants `asusd.service asus-shutdown.service` | `gitlab:data/asusd.rules` |
| Daemon refuses to run | if env `IS_SERVICE` is set and not `"1"` (prints "asusd should be only run from the right systemd service") | `daemon.rs` |

Startup order in `daemon.rs` (matters for what exists on a given machine):

1. `ObjectManager` at `/`
2. `Config::new().load()` → `/etc/asusd/asusd.ron`
3. `RogPlatform::new()?` — **fatal** if no `asus-nb-wmi` platform device
4. `AsusPower::new()?` — non-fatal if no battery (returns empty battery path)
5. `FirmwareAttributes::new()` → one `xyz.ljones.AsusArmoury` object per entry of `/sys/class/firmware-attributes/asus-armoury/attributes/` (skips `pending_reboot`)
6. `CtrlFanCurveZbus::new()` → `xyz.ljones.FanCurves` only if `platform_profile` exists **and** an hwmon named `asus_custom_fan_curve` exists
7. `CtrlBacklight::new()` → `xyz.ljones.Backlight` if `intel_backlight` **or** `asus_screenpad` backlight exists
8. `CtrlPlatform::new()` → `xyz.ljones.Platform`
9. `DeviceManager::new()` → Aura/Anime/Slash/ScsiAura objects, plus a udev monitor thread for hot-plug
10. **[6.4.0]** `CtrlXgmLed::try_new()` → `xyz.ljones.XgmLed` if `/sys/class/leds/asus:xgm*` exists
11. `request_name("xyz.ljones.Asusd")`

### 1.2 Object paths

| Path | Interfaces | Notes | Source |
|---|---|---|---|
| `/` | `org.freedesktop.DBus.ObjectManager` | discovery | `daemon.rs` |
| `/xyz/ljones` | `xyz.ljones.Platform`, `xyz.ljones.FanCurves`, `xyz.ljones.Backlight`, **[6.4.0]** `xyz.ljones.XgmLed` | all four share one object | `ctrl_platform.rs` (`PLATFORM_ZBUS_PATH`), `ctrl_fancurves.rs` (`FAN_CURVE_ZBUS_PATH`), `ctrl_backlight.rs`, `gh:asusd/src/ctrl_xgm_led.rs` |
| `/xyz/ljones/asus_armoury/<name>` | `xyz.ljones.AsusArmoury` | `<name>` is the **sysfs directory name** (`ppt_pl1_spl`, `nv_temp_target`, `dgpu_disable`, …). Every attribute the kernel exposes gets an object, even ones asusd doesn't know (their `Name` property is `"None"`). | `gitlab:asusd/src/asus_armoury.rs` (`dbus_path_for_attr`) |
| `/xyz/ljones/aura/<idProduct>_<devnum>_<devpath>` | `xyz.ljones.Aura` | USB HID keyboards (idVendor `0b05`). `.` characters in `devpath` are stripped. Example: `/xyz/ljones/aura/19b6_3_1`. | `gitlab:asusd/src/aura_manager.rs` (`filename_partial`, `dbus_path_for_dev`) |
| `/xyz/ljones/aura/tuf` | `xyz.ljones.Aura` | TUF laptops with no USB keyboard controller (sysfs `kbd_rgb_mode`/`kbd_rgb_state` via `asus::kbd_backlight` LED) | `aura_manager.rs` (`dbus_path_for_tuf`) |
| `/xyz/ljones/aura/<idProduct>_<devnum>_<devpath>` or `/xyz/ljones/aura/slash` | `xyz.ljones.Slash` | HID path if detected over hidraw, else the fixed `slash` path when found over raw USB | `aura_manager.rs` |
| `/xyz/ljones/aura/anime` | `xyz.ljones.Anime` | AniMe is always raw USB (`0x193b`) → fixed path (`maybe_anime_hid` is a stub returning Err) | `aura_manager.rs`, `aura_types.rs` |
| `/xyz/ljones/aura/<ID_SERIAL_SHORT>_scsi` | `xyz.ljones.ScsiAura` | ROG Strix Arion external SSD enclosures (`ID_MODEL_ID` 1932) | `aura_manager.rs` (`dbus_path_for_scsi`) |

`rog_dbus` proxy `default_path`s that are **wrong/stale** and must not be relied on: `zbus_aura.rs` says `/xyz/ljones/Aura`; `asus_armoury.rs` says `/xyz/ljones/asus_armoury/nv_temp_target`; `zbus_anime.rs` says `/xyz/ljones/aura/anime` (correct only by coincidence); `zbus_slash.rs` says `/xyz/ljones` (wrong). Always resolve with ObjectManager — that is what `rog_dbus::find_iface_async::<T>(iface_name)` does (`gitlab:rog-dbus/src/lib.rs`), and what `asusctl`/rog-control-center do.

Hot-plug: `DeviceManager` runs a udev monitor (`hidraw` and `block` subsystems). On `remove` it calls `object_server().remove::<AuraZbus|SlashZbus|AniMeZbus|ScsiZbus>(path)`, on `add` it re-initialises. Clients get `InterfacesAdded`/`InterfacesRemoved` from the ObjectManager.

### 1.3 `xyz.ljones.Platform` (at `/xyz/ljones`)

Source of truth: `gitlab:asusd/src/ctrl_platform.rs` (`#[interface(name = "xyz.ljones.Platform")] impl CtrlPlatform`), proxy in `gitlab:rog-dbus/src/zbus_platform.rs`.

| D-Bus member | Kind | Sig | Rust (proxy) | Behaviour / errors |
|---|---|---|---|---|
| `Version` | prop R | `s` | `version() -> String` | `CARGO_PKG_VERSION` of asusd, e.g. `"6.3.8"`. `asusctl` compares this to its own version and bails on mismatch. |
| `SupportedProperties` | method | `() -> as` | `supported_properties() -> Vec<Properties>` | Returns a subset of `["ChargeControlEndThreshold", "ThrottlePolicy"]` (only those two are ever pushed; the other `Properties` variants are dead). `"ThrottlePolicy"` present ⇔ `/sys/firmware/acpi/platform_profile` exists. |
| `NextPlatformProfile` | method | `() -> ()` | `next_platform_profile()` | Cycles Balanced→Performance→(LowPower if in choices else Quiet)→Balanced (`PlatformProfile::next`). Sets EPP if linked. Emits `PropertiesChanged` for `PlatformProfile` and `EnablePptGroup`. Error `org.freedesktop.DBus.Error.NotSupported` if no platform_profile. |
| `PlatformProfileChoices` | prop R | `au` | `platform_profile_choices() -> Vec<PlatformProfile>` | Parsed from `/sys/firmware/acpi/platform_profile_choices` (unknown names → Balanced with a warning). |
| `PlatformProfile` | prop RW | `u` | `platform_profile()` / `set_platform_profile(PlatformProfile)` | Setter: applies EPP for that profile if `PlatformProfileLinkedEpp`, writes config, checks `choices.contains` (else `NotSupported`), writes sysfs, emits `EnablePptGroup` changed. **External changes** (Fn+F5, power-profiles-daemon) are detected with inotify on `platform_profile` and re-emitted as `PropertiesChanged` (`PlatformProfile`, `EnablePptGroup`) and trigger fan-curve + PPT re-apply. |
| `PlatformProfileOnAc` | prop RW | `u` | `platform_profile_on_ac()` / `set_…(PlatformProfile)` | Setter also **applies the profile immediately** (calls `set_platform_profile`). `Quiet` is silently replaced by `LowPower` when Quiet is not in choices. |
| `ChangePlatformProfileOnAc` | prop RW | `b` | `change_platform_profile_on_ac()` / `set_…(bool)` | Whether to auto-switch on AC plug. |
| `PlatformProfileOnBattery` | prop RW | `u` | same pattern | same fallback semantics |
| `ChangePlatformProfileOnBattery` | prop RW | `b` | | |
| `PlatformProfileLinkedEpp` | prop RW | `b` | `platform_profile_linked_epp()` | If true, changing profile sets `energy_performance_preference` on every cpufreq policy (and forces `powersave` governor if needed). |
| `ProfileBalancedEpp` | prop RW | `u` | `profile_balanced_epp() -> CPUEPP` | Setter applies immediately if linked. |
| `ProfilePerformanceEpp` | prop RW | `u` | | |
| `ProfileQuietEpp` | prop RW | `u` | | Also used for `LowPower`. (`profile_custom_epp` exists in config but has **no** D-Bus property.) |
| `ChargeControlEndThreshold` | prop RW | `y` | `charge_control_end_threshold() -> u8` | Range check `20..=100` else `Failed("Charge limit …")`; `NotSupported` if no battery attr. Inotify-watched: external writes to sysfs emit `PropertiesChanged`. |
| `OneShotFullCharge` | method | `() -> ()` | `one_shot_full_charge()` | Sets sysfs to 100 and remembers `base_charge_control_end_threshold`; restored on next unplug or at shutdown. |
| `EnablePptGroup` | prop RW | `b` | `enable_ppt_group()` / `set_enable_ppt_group(bool)` | Getter/setter act on the tuning group for **(current profile, current AC/DC)**. Enabling writes stored PPT values (or attribute defaults) to sysfs; disabling re-writes the platform profile so ACPI resets PPT to firmware defaults. Then re-emits `MinValue/MaxValue/ScalarIncrement/CurrentValue` on every armoury object. **[6.4.0]** returns `Failed("Custom fan curve is required to enable profile tuning")` when fan curves are supported and none is enabled for the current profile. |
| **[6.4.0]** `DisableNvidiaPowerdOnBattery` | prop RW | `b` | `disable_nvidia_powerd_on_battery()` | Stop `nvidia-powerd.service` on battery. |

Signals: only the standard `org.freedesktop.DBus.Properties.PropertiesChanged`. asusd explicitly emits it for `PlatformProfile`, `EnablePptGroup`, `ChargeControlEndThreshold` on external/AC events; zbus emits it automatically for properties set via `Properties.Set`.

`Properties` enum (`gitlab:rog-platform/src/platform.rs`), signature `s`: `ChargeControlEndThreshold`, `DgpuDisable`, `GpuMuxMode`, `PostAnimationSound`, `PanelOd`, `MiniLedMode`, `EgpuEnable`, `ThrottlePolicy`.

Config-side automation done by `CtrlPlatform` without any D-Bus involvement (`create_tasks`): polls logind `OnExternalPower`/`LidClosed` every 2 s; on AC/DC change sets the configured profile + EPP, runs `ac_command`/`bat_command` from `asusd.ron`, re-applies fan curves then PPT (fan curves must be written first because PPT writes need the EC in manual fan mode, comment in `apply_fan_curves_and_ppt`), restores the charge limit; on resume re-applies the charge limit; on shutdown restores `base_charge_control_end_threshold`. **[6.4.0]** replaces the 2 s polling with udev/power-supply event monitors and adds `manage_nvidia_powerd`.

### 1.4 `xyz.ljones.AsusArmoury` (at `/xyz/ljones/asus_armoury/<attr>`)

Source: `gitlab:asusd/src/asus_armoury.rs`; sysfs model in `gitlab:rog-platform/src/asus_armoury.rs`; proxy `gitlab:rog-dbus/src/asus_armoury.rs`. Backed by `/sys/class/firmware-attributes/asus-armoury/attributes/<attr>/{current_value,default_value,min_value,max_value,scalar_increment,possible_values,display_name}` (kernel `asus-armoury` driver, mainline since 6.19 per README).

| Member | Kind | Sig | Semantics |
|---|---|---|---|
| `Name` | prop R | `s` | `FirmwareAttribute` variant name (CamelCase, see §2.2) — **not** the sysfs name. Unknown sysfs names → `"None"`. |
| `AvailableAttrs` | prop R | `as` | Subset of `["default_value","min_value","max_value","scalar_increment","possible_values","current_value"]` that this attribute actually has. Anything not listed reads as `-1` / empty. |
| `CurrentValue` | prop RW | `i` | See per-type rules below. Getter for PPT-type returns the **config-stored** value for (current profile, AC/DC), falling back to `default_value`, never sysfs. Setter errors: `NotSupported` for `ReadOnly`; otherwise sysfs write errors → `Failed`. |
| `DefaultValue` | prop R | `i` | `-1` if none. |
| `MinValue` / `MaxValue` | prop R | `i` | Re-read from sysfs on every call (limits change with profile/AC). `-1` if none. |
| `ScalarIncrement` | prop R | `i` | `-1` if none. |
| `PossibleValues` | prop R | `ai` | Only for enum-int attributes (`possible_values` containing `;`), else empty. |
| `QueuedGpuValue` | prop R | `i` | For GPU-type attrs: value queued for shutdown, or `-1`. Always `-1` for non-GPU. |
| `RestoreDefault` | method | `() -> ()` | Writes `default_value` to sysfs; for PPT type also updates the stored tuning and only writes if the group is enabled. |
| `ApplyQueuedGpuValue` | method | `() -> b` | Writes the queued GPU value now; returns whether anything was applied. Called by `asus-shutdown`. |

`PropertiesChanged` is emitted for `CurrentValue`, `DefaultValue`, `MinValue`, `MaxValue` via inotify on the sysfs directory, and for `MinValue/MaxValue/ScalarIncrement/CurrentValue` on every object after profile change, AC/DC change, `EnablePptGroup` change and `asusd.ron` reload (`ArmouryAttributeRegistry::emit_limits`).

Attribute classes (`FirmwareAttributeType`, `gitlab:rog-platform/src/asus_armoury.rs` `define_attribute_getters!`):

| Type | Attributes | Set behaviour | Persisted in |
|---|---|---|---|
| `Ppt` | `ppt_pl1_spl`, `ppt_pl2_sppt`, `ppt_pl3_fppt`, `ppt_apu_sppt`, `ppt_platform_sppt`, `ppt_fppt`, `nv_dynamic_boost`, `nv_temp_target`, `nv_tgp` | Stored in `asusd.ron` `ac_profile_tunings`/`dc_profile_tunings[profile].group[name]`; written to sysfs **only if** that group's `enabled` is true; otherwise the setter returns Ok after storing. Re-applied on every profile/AC change (`set_config_or_default`). | `asusd.ron` |
| `Gpu` | `gpu_mux_mode`, `dgpu_disable`, `egpu_enable`, `egpu_connected` | **Queued in memory** (`queued_gpu` map), not written; `asus-shutdown.service` applies at `PrepareForShutdown(true)` after waiting for the dGPU to be idle. Reboot required. | not persisted |
| `ReadOnly` | `nv_base_tgp` (**[6.4.0]** also `charge_mode`) | Setter → `NotSupported`. | — |
| `Bios` | `boot_sound` | Written immediately; saved in `armoury_settings`, restored at boot. | `asusd.ron` |
| `Immediate` (default) | `mcu_powersave`, `screen_auto_brightness`, `mini_led_mode`, `panel_hd_mode`, `panel_od`, `charge_mode` (6.3.8), `apu_mem`, `cores_performance`, `cores_efficiency`, anything unknown | Written immediately; saved in `armoury_settings`, restored at boot. | `asusd.ron` |
| **[6.4.0]** `Norestore` | `apu_mem` | Written immediately, never restored/persisted. | — |

Wire values are whatever the kernel exposes: booleans are `0/1`; `gpu_mux_mode` `0` = dGPU only ("Ultimate"), `1` = MUX to iGPU (Optimus/Hybrid); `dgpu_disable` `1` = iGPU only. The GUI's three GPU modes map to (`dgpu_disable`,`gpu_mux_mode`) = Integrated (1,1), Ultimate (0,0), Hybrid (0,1) (`gitlab:GPU_MODE_SWITCHING_SUMMARY.md`, `rog-control-center/src/ui/setup_gpu.rs`). Charge limit is **not** an armoury attribute; it is `Platform.ChargeControlEndThreshold` (power_supply sysfs).

Quirk: the `From<&str>` mapping expects the sysfs name `panel_overdrive` for `FirmwareAttribute::PanelOverdrive`, while the getter list (and the kernel driver, to my knowledge) use `panel_od`. Key your GUI on the object path's last segment (sysfs name) rather than on `Name`.

### 1.5 `xyz.ljones.FanCurves` (at `/xyz/ljones`)

Source: `gitlab:asusd/src/ctrl_fancurves.rs`; types `gitlab:rog-profiles/src/{lib.rs,fan_curve_set.rs}`; proxy `gitlab:rog-dbus/src/zbus_fan_curves.rs`. Backed by hwmon `asus_custom_fan_curve` (`pwm{1,2,3}_auto_point{1..8}_{pwm,temp}`, `pwm{n}_enable`). Present only when both `platform_profile` and that hwmon exist. Methods only — no properties, no signals.

| Method | Sig | Rust |
|---|---|---|
| `FanCurveData` | `(u) -> a(s(yyyyyyyy)(yyyyyyyy)b)` | `fan_curve_data(profile: PlatformProfile) -> Vec<CurveData>` — returns the **stored** curves for that profile (Quiet and LowPower share one set). |
| `SetFanCurve` | `(u (s(yyyyyyyy)(yyyyyyyy)b)) -> ()` | `set_fan_curve(profile, curve: CurveData)` — replaces the curve whose `fan` matches; writes to hardware only if `profile` is the active profile; then re-applies PPT. |
| `SetFanCurvesEnabled` | `(u b) -> ()` | `set_fan_curves_enabled(profile, enabled)` — all fans of a profile. |
| `SetProfileFanCurveEnabled` | `(u s b) -> ()` | `set_profile_fan_curve_enabled(profile, fan: FanCurvePU, enabled)` |
| `SetCurvesToDefaults` | `(u) -> ()` | `set_curves_to_defaults(profile)` — **switches the platform profile** to `profile`, writes `pwmN_enable=3` (reset), reads factory curve back, switches back. Expect a brief profile flip. |

`CurveData { fan: FanCurvePU ("CPU"/"GPU"/"MID" as `s`), pwm: [u8; 8] (0–255), temp: [u8; 8] (°C), enabled: bool }` → signature `(s(yyyyyyyy)(yyyyyyyy)b)`: fixed-size arrays travel as structures, not `ay` (verified against asusd 6.4.0 introspection on a GA403WR, 2026-09-10). `pwmN_enable` is written `1` for enabled curves, `2` for disabled. String form for CLI/RON: `30c:1%,49c:2%,…` (8 points, non-decreasing). Persisted in `/etc/asusd/fan_curves.ron` (`FanCurveConfig { profiles: FanCurveProfiles { balanced, performance, quiet, custom: Vec<CurveData> } }`); on first run every profile is visited to capture factory defaults.

### 1.6 `xyz.ljones.Aura` (at `/xyz/ljones/aura/<dev>` or `/xyz/ljones/aura/tuf`)

Source: `gitlab:asusd/src/aura_laptop/trait_impls.rs`; types `gitlab:rog-aura/src/{lib.rs,builtin_modes.rs,keyboard/power.rs,keyboard/advanced.rs}`; proxy `gitlab:rog-dbus/src/zbus_aura.rs`. Capabilities come from `/usr/share/asusd/aura_support.ron` (+ `/etc/asusd/asusd_user_ledmodes.ron` overrides) keyed on DMI `board_name` or USB product id.

| Member | Kind | Sig | Rust | Notes |
|---|---|---|---|---|
| `DeviceType` | prop R | `u` | `device_type() -> AuraDeviceType` | positional index: `LaptopKeyboard2021`=0 (19b6/1a30), `LaptopKeyboardPre2021`=1 (1866/18c6/1869/1854), `LaptopKeyboardTuf`=2, `ScsiExtDisk`=3, `Ally`=4 (1abe/1b4c), `AnimeOrSlash`=5, `Unknown`=6 |
| `Brightness` | prop RW | `u` | `brightness() -> LedBrightness` | Off=0, Low=1, Med=2, High=3; via `asus::kbd_backlight` sysfs. `Failed` if no sysfs brightness node. |
| `SupportedBrightness` | prop R | `au` | | always `[0,1,2,3]` |
| `SupportedBasicModes` | prop R | `au` | `Vec<AuraModeNum>` | from support DB |
| `SupportedBasicZones` | prop R | `au` | `Vec<AuraZone>` | empty for single-zone keyboards |
| `SupportedPowerZones` | prop R | `au` | `Vec<PowerZones>` | |
| `LedMode` | prop RW | `u` | `led_mode() -> AuraModeNum` | Getter uses `try_lock` and can transiently fail with `Failed("Aura control couldn't lock self")` — retry. Setter applies the stored effect for that mode and bumps brightness to Med if Off. |
| `LedModeData` | prop RW | `(uu(yyy)(yyy)ss)` | `led_mode_data() -> AuraEffect` | Setter validates mode ∈ basic_modes and zone ∈ basic_zones (else `NotSupported`), writes the HID packet (padded to 64 bytes) + SET + APPLY, stores it. Zone ≠ None switches the keyboard to multizone mode. |
| `AllModeData` | method | `() -> a{u(uu(yyy)(yyy)ss)}` | `all_mode_data() -> BTreeMap<AuraModeNum, AuraEffect>` | stored per-mode effects |
| `LedPower` | prop RW | `(a(ubbbb))` | `led_power() -> LaptopAuraPower` | `states: Vec<AuraPowerState { zone: PowerZones, boot, awake, sleep, shutdown }>`. Setter merges by zone then writes the power packet (bit layout in `keyboard/power.rs`). Pre-2021/TUF ignore `shutdown`. |
| `DirectAddressingRaw` | method | `(aay) -> ()` | `direct_addressing_raw(AuraLaptopUsbPackets = Vec<Vec<u8>>)` | Raw per-key/zone HID packets (see `rog_aura::keyboard::{LedUsbPackets, KeyLayout}` for building them). `AuraProxyPerkey` wrapper sleeps 33 ms after each call. |

No custom signals. `PropertiesChanged` only for D-Bus-initiated sets. Hardware keys (Fn+F3/F4) change brightness in sysfs without a signal (there is commented-out watch code).

### 1.7 `xyz.ljones.Anime` (at `/xyz/ljones/aura/anime`)

Source: `gitlab:asusd/src/aura_anime/trait_impls.rs`; types `gitlab:rog-anime/src/{data.rs,usb.rs}`; proxy `gitlab:rog-dbus/src/zbus_anime.rs`. Detected by DMI (`AnimeType`: GA401, GA402, GU604, G635L, G835L) + USB `0b05:193b`.

| Member | Kind | Sig | Notes |
|---|---|---|---|
| `Write` | method | `((ays)) -> ()` | `write(AnimeDataBuffer { data: Vec<u8>, anime: AnimeType })`. Disables builtins if they were on, stops the daemon's own animation thread, writes one frame. Use `rog_anime::{AnimeImage, AnimeGif, AnimeDiagonal}` to produce buffers. |
| `RunMainLoop` | method | `(b) -> ()` | `true` restarts the daemon's `system` sequence from `anime.ron`. |
| `DeviceState` | method | `() -> (bub(ssss)bbbu)` | `DeviceState { display_enabled, display_brightness: Brightness, builtin_anims_enabled, builtin_anims: Animations, off_when_unplugged, off_when_suspended, off_when_lid_closed, brightness_on_battery }` |
| `NotifyDeviceState` | **signal** | `(bub(ssss)bbbu)` | declared in the proxy; the 6.3.8 daemon never emits it. |
| `Brightness` | prop RW | `u` | `Brightness`: Off=0, Low=1, Med=2, High=3. Setter also toggles display enable. |
| `BuiltinsEnabled` | prop RW | `b` | "powersave animations" |
| `BuiltinAnimations` | prop RW | `(ssss)` | `Animations { boot: AnimBooting, awake: AnimAwake, sleep: AnimSleeping, shutdown: AnimShutdown }` — strings: boot `GlitchConstruction|StaticEmergence`, awake `BinaryBannerScroll|RogLogoGlitch`, sleep `BannerSwipe|Starfield`, shutdown `GlitchOut|SeeYa` |
| `EnableDisplay` | prop RW | `b` | |
| `OffWhenUnplugged` / `OffWhenSuspended` / `OffWhenLidClosed` | prop RW | `b` | applied immediately against current logind state |

### 1.8 `xyz.ljones.Slash`, `xyz.ljones.Backlight`, `xyz.ljones.ScsiAura`, **[6.4.0]** `xyz.ljones.XgmLed`

**Slash** (`gitlab:asusd/src/aura_slash/trait_impls.rs`; `gitlab:rog-slash/src/data.rs`) — the diagonal LED bar on Zephyrus GA403/GA605/GU605/G614 (2024/2025). Properties (all RW): `Enabled b`, `Brightness y` (0–255; enabling with 0 sets 0x88), `Interval y`, `Mode` (see below), `ShowOnBoot b`, `ShowOnSleep b`, `ShowOnShutdown b`, `ShowOnBattery b`, `ShowBatteryWarning b`, `ShowOnLidClosed b`. Method `DeviceState() -> (byyu)`. `SlashMode` names: `Static, Bounce, Slash, Loading, BitStream, Transmission, Flow (default), Flux, Phantom, Spectrum, Hazard, Interfacing, Ramp, GameOver, Start, Buzzer`.
* **6.3.8 bug**: the daemon's `Mode` getter is typed `u8` and returns `display_interval`, while the setter takes `SlashMode` (`u`) and the proxy reads `u` → reading `Mode` through the 6.3.8 proxy fails with a signature mismatch.
* **[6.4.0]**: `Mode` is `y` both ways and carries the raw HID mode byte (`display_mode as u8`, e.g. Flow = 0x19).

**Backlight** (`gitlab:asusd/src/ctrl_backlight.rs`) — only laptops with an `intel_backlight` and/or `asus_screenpad` node. Properties: `PrimaryBrightness i` (0–100 %, RW, emits changed), `ScreenpadBrightness i` (0–100 % gamma-corrected, RW), `ScreenpadGamma s` (RW, `"0.1".."2.0"` parsed as f32), `ScreenpadPower b` (RW, `bl_power`), `ScreenpadSyncWithPrimary b` (RW). Setting >100 → `Failed`.

**ScsiAura** (`gitlab:asusd/src/aura_scsi/trait_impls.rs`; `gitlab:rog-scsi/src/builtin_modes.rs`) — ROG Strix Arion. `DeviceType u`, `Enabled b` (RW), `LedMode` (getter `y`, setter takes `AuraMode` as `u` — another getter/setter type mismatch; the proxy reads `u8`), `LedModeData (usu(yyy)(yyy)(yyy)(yyy))` (`AuraEffect { mode: AuraMode(u), speed: Speed(s: Slowest|Slow|Med|Fast|Fastest), direction: Direction(u), colour1..4: Colour }`), method `AllModeData() -> a{u(usu(yyy)(yyy)(yyy)(yyy))}`. `AuraMode`: Off=0, Static, Breathe, Flashing, RainbowCycle, RainbowWave, RainbowCycleBreathe, ChaseFade, RainbowCycleChaseFade, Chase, RainbowCycleChase, RainbowCycleWave, RainbowPulseChase, RandomFlicker, DoubleFade=14.

**XgmLed [6.4.0]** (`gh:asusd/src/ctrl_xgm_led.rs`, `gh:rog-dbus/src/zbus_xgm_led.rs`) at `/xyz/ljones`: single property `XgmLedEnabled b` (RW) driving `/sys/class/leds/asus:xgm*/brightness`; state persisted in `asusd.ron` `xgm_led_enabled: Option<bool>`.

### 1.9 Reconstructed introspection XML

Reconstructed from the `#[interface]` impls (6.3.8; `[6.4.0]` additions marked). Type signatures follow §2.3.

```xml
<node name="/xyz/ljones">
  <interface name="xyz.ljones.Platform">
    <property name="Version" type="s" access="read"/>
    <method name="SupportedProperties"><arg type="as" direction="out"/></method>
    <method name="NextPlatformProfile"/>
    <method name="OneShotFullCharge"/>
    <property name="PlatformProfileChoices" type="au" access="read"/>
    <property name="PlatformProfile" type="u" access="readwrite"/>
    <property name="PlatformProfileOnAc" type="u" access="readwrite"/>
    <property name="ChangePlatformProfileOnAc" type="b" access="readwrite"/>
    <property name="PlatformProfileOnBattery" type="u" access="readwrite"/>
    <property name="ChangePlatformProfileOnBattery" type="b" access="readwrite"/>
    <property name="PlatformProfileLinkedEpp" type="b" access="readwrite"/>
    <property name="ProfileBalancedEpp" type="u" access="readwrite"/>
    <property name="ProfilePerformanceEpp" type="u" access="readwrite"/>
    <property name="ProfileQuietEpp" type="u" access="readwrite"/>
    <property name="ChargeControlEndThreshold" type="y" access="readwrite"/>
    <property name="EnablePptGroup" type="b" access="readwrite"/>
    <!-- [6.4.0] --><property name="DisableNvidiaPowerdOnBattery" type="b" access="readwrite"/>
  </interface>
  <interface name="xyz.ljones.FanCurves">
    <method name="FanCurveData"><arg name="profile" type="u" direction="in"/><arg type="a(s(yyyyyyyy)(yyyyyyyy)b)" direction="out"/></method>
    <method name="SetFanCurve"><arg name="profile" type="u" direction="in"/><arg name="curve" type="(s(yyyyyyyy)(yyyyyyyy)b)" direction="in"/></method>
    <method name="SetFanCurvesEnabled"><arg name="profile" type="u" direction="in"/><arg name="enabled" type="b" direction="in"/></method>
    <method name="SetProfileFanCurveEnabled"><arg name="profile" type="u" direction="in"/><arg name="fan" type="s" direction="in"/><arg name="enabled" type="b" direction="in"/></method>
    <method name="SetCurvesToDefaults"><arg name="profile" type="u" direction="in"/></method>
  </interface>
  <interface name="xyz.ljones.Backlight">
    <property name="PrimaryBrightness" type="i" access="readwrite"/>
    <property name="ScreenpadBrightness" type="i" access="readwrite"/>
    <property name="ScreenpadGamma" type="s" access="readwrite"/>
    <property name="ScreenpadPower" type="b" access="readwrite"/>
    <property name="ScreenpadSyncWithPrimary" type="b" access="readwrite"/>
  </interface>
  <!-- [6.4.0] -->
  <interface name="xyz.ljones.XgmLed">
    <property name="XgmLedEnabled" type="b" access="readwrite"/>
  </interface>
</node>

<node name="/xyz/ljones/asus_armoury/ppt_pl1_spl">   <!-- one per attribute -->
  <interface name="xyz.ljones.AsusArmoury">
    <property name="Name" type="s" access="read"/>
    <property name="AvailableAttrs" type="as" access="read"/>
    <property name="CurrentValue" type="i" access="readwrite"/>
    <property name="DefaultValue" type="i" access="read"/>
    <property name="MinValue" type="i" access="read"/>
    <property name="MaxValue" type="i" access="read"/>
    <property name="ScalarIncrement" type="i" access="read"/>
    <property name="PossibleValues" type="ai" access="read"/>
    <property name="QueuedGpuValue" type="i" access="read"/>
    <method name="RestoreDefault"/>
    <method name="ApplyQueuedGpuValue"><arg type="b" direction="out"/></method>
  </interface>
</node>

<node name="/xyz/ljones/aura/19b6_3_1">   <!-- path varies -->
  <interface name="xyz.ljones.Aura">
    <property name="DeviceType" type="u" access="read"/>
    <property name="Brightness" type="u" access="readwrite"/>
    <property name="SupportedBrightness" type="au" access="read"/>
    <property name="SupportedBasicModes" type="au" access="read"/>
    <property name="SupportedBasicZones" type="au" access="read"/>
    <property name="SupportedPowerZones" type="au" access="read"/>
    <property name="LedMode" type="u" access="readwrite"/>
    <property name="LedModeData" type="(uu(yyy)(yyy)ss)" access="readwrite"/>
    <property name="LedPower" type="(a(ubbbb))" access="readwrite"/>
    <method name="AllModeData"><arg type="a{u(uu(yyy)(yyy)ss)}" direction="out"/></method>
    <method name="DirectAddressingRaw"><arg name="data" type="aay" direction="in"/></method>
  </interface>
</node>

<node name="/xyz/ljones/aura/anime">
  <interface name="xyz.ljones.Anime">
    <method name="Write"><arg name="input" type="(ays)" direction="in"/></method>
    <method name="RunMainLoop"><arg name="start" type="b" direction="in"/></method>
    <method name="DeviceState"><arg type="(bub(ssss)bbbu)" direction="out"/></method>
    <signal name="NotifyDeviceState"><arg name="data" type="(bub(ssss)bbbu)"/></signal>
    <property name="Brightness" type="u" access="readwrite"/>
    <property name="BuiltinsEnabled" type="b" access="readwrite"/>
    <property name="BuiltinAnimations" type="(ssss)" access="readwrite"/>
    <property name="EnableDisplay" type="b" access="readwrite"/>
    <property name="OffWhenUnplugged" type="b" access="readwrite"/>
    <property name="OffWhenSuspended" type="b" access="readwrite"/>
    <property name="OffWhenLidClosed" type="b" access="readwrite"/>
  </interface>
</node>

<node name="/xyz/ljones/aura/slash">   <!-- or /xyz/ljones/aura/<usb id> -->
  <interface name="xyz.ljones.Slash">
    <property name="Enabled" type="b" access="readwrite"/>
    <property name="Brightness" type="y" access="readwrite"/>
    <property name="Interval" type="y" access="readwrite"/>
    <property name="Mode" type="y" access="readwrite"/>   <!-- 6.3.8: getter y (buggy), setter u; 6.4.0: y/y -->
    <property name="ShowOnBoot" type="b" access="readwrite"/>
    <property name="ShowOnSleep" type="b" access="readwrite"/>
    <property name="ShowOnShutdown" type="b" access="readwrite"/>
    <property name="ShowOnBattery" type="b" access="readwrite"/>
    <property name="ShowBatteryWarning" type="b" access="readwrite"/>
    <property name="ShowOnLidClosed" type="b" access="readwrite"/>
    <method name="DeviceState"><arg type="(byyu)" direction="out"/></method>
  </interface>
</node>

<node name="/xyz/ljones/aura/M3D0AP048745_scsi">
  <interface name="xyz.ljones.ScsiAura">
    <property name="DeviceType" type="u" access="read"/>
    <property name="Enabled" type="b" access="readwrite"/>
    <property name="LedMode" type="y" access="readwrite"/>   <!-- setter actually takes u -->
    <property name="LedModeData" type="(usu(yyy)(yyy)(yyy)(yyy))" access="readwrite"/>
    <method name="AllModeData"><arg type="a{u(usu(yyy)(yyy)(yyy)(yyy))}" direction="out"/></method>
  </interface>
</node>
```

Errors: all controller errors are mapped to `org.freedesktop.DBus.Error.Failed` with the `Display` text (`impl From<RogError> for zbus::fdo::Error`, `gitlab:asusd/src/error.rs`), and explicit capability checks use `org.freedesktop.DBus.Error.NotSupported`.

---

## 2. Rust crates and types

### 2.1 Workspace layout (`gitlab:Cargo.toml`, version `6.3.8`, `rust-version = "1.82"`, `zbus = "5.13.1"`, edition 2021)

| Crate (package name) | Path | Role | Notable deps |
|---|---|---|---|
| `asusd` | `asusd/` | the daemon **and** a library exporting `DBUS_NAME/DBUS_PATH/DBUS_IFACE`, controllers | udev, inotify, rusb, logind-zbus, tokio |
| `rog_dbus` | `rog-dbus/` | zbus `#[proxy]` traits + ObjectManager helpers (`list_iface_blocking`, `has_iface`, `find_iface_async`; **[6.4.0]** `find_iface_blocking`, `has_iface*` removed) | **depends on `asusd`**, `rog_anime`, `rog_slash`, `rog_scsi`, `rog_aura`, `rog_profiles`, `rog_platform` |
| `rog_platform` | `rog-platform/` | sysfs wrappers: `platform::{RogPlatform, PlatformProfile, GpuMode, Properties}`, `asus_armoury::{FirmwareAttribute, FirmwareAttributes, Attribute, AttrValue, FirmwareAttributeType}`, `cpu::{CPUControl, CPUEPP, CPUGovernor}`, `power::AsusPower`, `backlight`, `keyboard_led`, `hid_raw`, `usb_raw`; **[6.4.0]** `cled`, `gpu_pci` | udev, inotify, rusb |
| `rog_profiles` | `rog-profiles/` | `FanCurvePU`, `FanCurveProfiles`, `fan_curve_set::CurveData`, `find_fan_curve_node()` | udev; feature `dbus` (default) |
| `rog_aura` | `rog-aura/` | `AuraDeviceType`, `PowerZones`, `AuraModeNum`, `AuraZone`, `AuraEffect`, `Colour`, `Speed`, `Direction`, `LedBrightness`, `keyboard::{LaptopAuraPower, AuraPowerState, AuraLaptopUsbPackets, LedUsbPackets, KeyLayout, LedCode, AdvancedAuraType}`, `aura_detection::{LedSupportData, LedSupportFile}`, `effects::*` | feature `dbus` (default) |
| `rog_anime` | `rog-anime/` | `AnimeType`, `AnimeDataBuffer`, `Animations`, `DeviceState`, `usb::{Brightness, AnimBooting, AnimAwake, AnimSleeping, AnimShutdown, get_anime_type}`, `AnimeImage`, `AnimeGif`, `AnimeDiagonal`, `Sequences`, `ActionLoader` | png_pong, gif, glam; features `dbus`, `detect` |
| `rog_slash` | `rog-slash/` | `SlashMode`, `SlashType`, `DeviceState`, `usb::slash_pkt_*` | |
| `rog_scsi` | `rog-scsi/` | `AuraMode`, `AuraEffect`, `Speed`, `Direction`, `Colour`, `ScsiType`, `open_device` | `sg` (git) |
| `config_traits` | `config-traits/` | `StdConfig`, `StdConfigLoad*` (RON files, migrations) | ron |
| `dmi_id` | `dmi-id/` | `DMIID` reader | |
| `asusctl`, `asusd-user`, `asus-shutdown`, `rog-control-center` | binaries | | rog-control-center: Slint (git), ksni tray, notify-rust; 6.3.8 also `supergfxctl` (git) — **[6.3.9] removed** |

### 2.2 Core enums/structs with their wire encodings

```rust
// rog_platform::platform
#[repr(u32)] #[zvariant(signature = "u")]
pub enum PlatformProfile { Balanced = 0, Performance = 1, Quiet = 2, LowPower = 3, Custom = 4 }
// sysfs strings: "balanced" "performance" "quiet" "low-power" "custom"; From<&str> is lenient.
impl PlatformProfile { pub fn next(current: Self, choices: &[Self]) -> Self }

#[repr(u8)] #[zvariant(signature = "s")]
pub enum Properties { ChargeControlEndThreshold, DgpuDisable, GpuMuxMode, PostAnimationSound,
                      PanelOd, MiniLedMode, EgpuEnable, ThrottlePolicy }

#[repr(u8)]  // not used on D-Bus any more (GUI computes modes from armoury attrs)
pub enum GpuMode { Optimus = 0, Integrated = 1, Egpu = 2, Vfio = 3, Ultimate = 4, Error = 254, NotSupported = 255 }

// rog_platform::cpu
#[repr(u32)] #[zvariant(signature = "u")]
pub enum CPUEPP { Default = 0, Performance = 1, BalancePerformance = 2, BalancePower = 3, Power = 4 }
// sysfs: "default" "performance" "balance_performance" "balance_power" "power"

// rog_platform::asus_armoury   (D-Bus `Name` property = variant name as string)
#[repr(u8)] #[zvariant(signature = "s")]
pub enum FirmwareAttribute {
  ApuMem, CoresPerformance, CoresEfficiency, PptPl1Spl, PptPl2Sppt, PptPl3Fppt, PptFppt,
  PptApuSppt, PptPlatformSppt, NvDynamicBoost, NvTempTarget, DgpuBaseTgp /*nv_base_tgp*/,
  DgpuTgp /*nv_tgp*/, ChargeMode, BootSound, McuPowersave, PanelOverdrive, PanelHdMode,
  EgpuConnected, EgpuEnable, DgpuDisable, GpuMuxMode, MiniLedMode, PendingReboot,
  PptEnabled /*removed in 6.4.0*/, None, ScreenAutoBrightness }
impl From<&str> for FirmwareAttribute; impl From<FirmwareAttribute> for &str;  // sysfs names
impl FirmwareAttribute { pub fn property_type(&self) -> FirmwareAttributeType }
pub enum FirmwareAttributeType { Immediate, Ppt, Gpu, Bios, ReadOnly /*, Norestore [6.4.0]*/ }

// rog_profiles
#[zvariant(signature = "s")]
pub enum FanCurvePU { CPU = 0, GPU = 1, MID = 2 }         // sysfs pwm1/pwm2/pwm3
pub struct CurveData { pub fan: FanCurvePU, pub pwm: [u8; 8], pub temp: [u8; 8], pub enabled: bool }
impl FromStr for CurveData  // "30c:1%,49c:2%,..." or "30:1,49:2,..." (0-255)
pub struct FanCurveProfiles { pub balanced: Vec<CurveData>, pub performance: Vec<CurveData>,
                              pub quiet: Vec<CurveData>, pub custom: Vec<CurveData> }

// rog_aura
#[zvariant(signature = "u")]
pub enum AuraModeNum { Static = 0, Breathe = 1, RainbowCycle = 2, RainbowWave = 3, Star = 4, Rain = 5,
                       Highlight = 6, Laser = 7, Ripple = 8, Pulse = 10, Comet = 11, Flash = 12 }
impl zvariant::Basic for AuraModeNum { 'u' }   // usable as dict key
#[zvariant(signature = "u")] pub enum AuraZone { None=0, Key1, Key2, Key3, Key4, Logo, BarLeft, BarRight }
#[zvariant(signature = "u")] pub enum LedBrightness { Off=0, Low, Med, High }
#[zvariant(signature = "s")] pub enum Speed { Low = 0xe1, Med = 0xeb, High = 0xf5 }
#[zvariant(signature = "s")] pub enum Direction { Right=0, Left, Up, Down }
pub struct Colour { pub r: u8, pub g: u8, pub b: u8 }                       // (yyy), FromStr "ff00ff"
pub struct AuraEffect { pub mode: AuraModeNum, pub zone: AuraZone, pub colour1: Colour,
                        pub colour2: Colour, pub speed: Speed, pub direction: Direction }
pub enum AuraDeviceType { LaptopKeyboard2021=0, LaptopKeyboardPre2021=1, LaptopKeyboardTuf=2,
                          ScsiExtDisk=3, Ally=4, AnimeOrSlash=5, Unknown=255 }   // default "u"
#[zvariant(signature = "u")]
pub enum PowerZones { Logo=0, Keyboard=1, Lightbar=2, Lid=3, RearGlow=4, KeyboardAndLightbar=5, Ally=6, None=255 }
// rog_aura::keyboard
pub struct AuraPowerState { pub zone: PowerZones, pub boot: bool, pub awake: bool, pub sleep: bool, pub shutdown: bool }
pub struct LaptopAuraPower { pub states: Vec<AuraPowerState> }
pub type AuraLaptopUsbPackets = Vec<Vec<u8>>;

// rog_anime
#[zvariant(signature = "u")] pub enum usb::Brightness { Off=0, Low, Med, High }
#[zvariant(signature = "s")] pub enum usb::AnimBooting { GlitchConstruction, StaticEmergence }
#[zvariant(signature = "s")] pub enum usb::AnimAwake { BinaryBannerScroll, RogLogoGlitch }
#[zvariant(signature = "s")] pub enum usb::AnimSleeping { BannerSwipe, Starfield }
#[zvariant(signature = "s")] pub enum usb::AnimShutdown { GlitchOut, SeeYa }
pub struct Animations { pub boot: AnimBooting, pub awake: AnimAwake, pub sleep: AnimSleeping, pub shutdown: AnimShutdown }
pub struct DeviceState { pub display_enabled: bool, pub display_brightness: Brightness,
  pub builtin_anims_enabled: bool, pub builtin_anims: Animations, pub off_when_unplugged: bool,
  pub off_when_suspended: bool, pub off_when_lid_closed: bool, pub brightness_on_battery: Brightness }
#[zvariant(signature = "s")] pub enum AnimeType { GA401, GA402, GU604, G635L, G835L, Unsupported }
pub struct AnimeDataBuffer { data: Vec<u8>, anime: AnimeType }   // (ays); AnimeDataBuffer::new / from_vec

// rog_slash
pub enum SlashMode { Static = 0x06, Bounce = 0x10, Slash = 0x12, Loading = 0x13, BitStream = 0x1d,
  Transmission = 0x1a, Flow = 0x19, Flux = 0x25, Phantom = 0x24, Spectrum = 0x26, Hazard = 0x32,
  Interfacing = 0x33, Ramp = 0x34, GameOver = 0x42, Start = 0x43, Buzzer = 0x44 }   // default "u"
pub struct DeviceState { pub slash_enabled: bool, pub slash_brightness: u8, pub slash_interval: u8, pub slash_mode: SlashMode }
```

### 2.3 Serialisation rules over D-Bus (zvariant 5)

`zvariant_derive` docs (`~/.cargo/registry/.../zvariant_derive-5.15.0/src/lib.rs`): a unit-only enum is encoded as **`u32` = serde's positional variant index** by default; with `#[zvariant(signature = "s")]` it is encoded as the **variant name string**. Discriminant values (`= 10`, `= 0xe1`, `= 255`) are irrelevant to the wire format. Consequences for a client that does not reuse the rog_* types (e.g. `busctl`, or your own types):

| Type | Wire | Values |
|---|---|---|
| `PlatformProfile` | `u` | 0 Balanced, 1 Performance, 2 Quiet, 3 LowPower, 4 Custom |
| `CPUEPP` | `u` | 0 Default, 1 Performance, 2 BalancePerformance, 3 BalancePower, 4 Power |
| `Properties`, `FirmwareAttribute`, `FanCurvePU`, `Speed`, `Direction` (rog_aura), `AnimeType`, `Anim*`, scsi `Speed` | `s` | variant name, e.g. `"ThrottlePolicy"`, `"PptPl1Spl"`, `"CPU"`, `"Med"`, `"Left"`, `"GA402"`, `"SeeYa"` |
| `AuraModeNum` | `u` | 0 Static … 8 Ripple, **9 Pulse, 10 Comet, 11 Flash** (positional, not 10/11/12) |
| `AuraZone`, `LedBrightness`, `Brightness` (anime) | `u` | positional = discriminant |
| `AuraDeviceType` | `u` | 0..5 as listed, **6 = Unknown** (not 255) |
| `PowerZones` | `u` | 0 Logo … 6 Ally, **7 = None** |
| `SlashMode` (6.3.8 setter) | `u` | positional 0 Static, 1 Bounce, … 6 Flow, … 15 Buzzer |
| `SlashMode` (**6.4.0** property) | `y` | raw byte (`Flow` = 0x19 …) |
| scsi `AuraMode`, scsi `Direction` | `u` | positional = discriminant |
| `Colour` | `(yyy)` | |
| `AuraEffect` | `(uu(yyy)(yyy)ss)` | |
| `LaptopAuraPower` | `(a(ubbbb))` | |
| `CurveData` | `(sayayb)` | `[u8; 8]` → `ay` |
| `Animations` | `(ssss)` | |
| anime `DeviceState` | `(bub(ssss)bbbu)` | |
| `AnimeDataBuffer` | `(ays)` | |
| slash `DeviceState` | `(byyu)` | |
| scsi `AuraEffect` | `(usu(yyy)(yyy)(yyy)(yyy))` | |

If you reuse the rog_* crates via git, all of this is handled by their `Type`/`Serialize` impls. If you write your own types, derive them the same way (`#[derive(Type, Value, OwnedValue, Serialize, Deserialize)]` + matching `#[zvariant(signature = …)]`) or use `u32`/`String` directly.

### 2.4 Publication status (crates.io checked 2026-09-09)

| Crate | crates.io |
|---|---|
| `rog_dbus`, `rog_platform`, `rog_aura`, `rog_profiles`, `rog_slash`, `rog_scsi`, `asusd`, `asusctl` | **not published** |
| `rog_anime` | 1.3.0 from 2021-12-19 — obsolete, do not use |

So the proxies must come from git:

```toml
[dependencies]
rog_dbus     = { git = "https://github.com/OpenGamingCollective/asusctl.git", tag = "6.4.0" }
rog_platform = { git = "https://github.com/OpenGamingCollective/asusctl.git", tag = "6.4.0" }
# 6.3.8 is still fetchable from the frozen GitLab:
# rog_dbus = { git = "https://gitlab.com/asus-linux/asusctl.git", tag = "6.3.8" }
```

Caveats: `rog_dbus` depends on the `asusd` crate, which pulls `udev`, `rusb` (libusb), `inotify`, `logind-zbus`, `tokio`, `notify`-style deps and needs `libudev`/`libusb` headers at build time; GitHub `main` uses let-chains (`if … && let Some(..)` in `gh:rog-platform/src/cpu.rs`) so it needs a recent toolchain (see `gh:rust-toolchain.toml`). The proxies are plain `#[zbus::proxy]` traits, so re-declaring the handful you need (and the enums from §2.2) in your own crate is the pragmatic route — it also lets one binary speak both `org.asuslinux.*` and `xyz.ljones.*`.

Proxy helper you will want to replicate (`gitlab:rog-dbus/src/lib.rs`):

```rust
pub async fn find_iface_async<T>(iface_name: &str) -> Result<Vec<T>, Box<dyn std::error::Error>>
where T: zbus::proxy::ProxyImpl<'static> + From<zbus::Proxy<'static>>
{
    let conn = zbus::Connection::system().await?;
    let om = zbus::fdo::ObjectManagerProxy::new(&conn, "xyz.ljones.Asusd", "/").await?;
    let objects = om.get_managed_objects().await?;
    let mut paths: Vec<_> = objects.iter()
        .filter(|(_, ifaces)| ifaces.keys().any(|k| k.as_str() == iface_name))
        .map(|(p, _)| p.clone()).collect();
    paths.sort();
    // build T::builder(&conn).path(p)?.destination("xyz.ljones.Asusd")?.build().await? for each
    …
}
```

---

## 3. Version history of the D-Bus names (for dual-stack support)

Verified by fetching `rog-dbus/src/*.rs` and `asusd/src/lib.rs` at tags 5.0.10, 6.0.0, 6.0.12, 6.1.0-rc1, 6.1.0-rc2, 6.1.0, 6.1.12, 6.2.0, 6.3.8 and GitHub `main`.

### 3.1 ≤ 5.0.10 (Mar 2024)

* Bus name `org.asuslinux.Daemon`; **every** interface is also named `org.asuslinux.Daemon`.
* Paths: `/org/asuslinux/Platform`, `/org/asuslinux/Aura`, `/org/asuslinux/Anime`, `/org/asuslinux/FanCurves`. (4.x had `/org/asuslinux/RogBios` → renamed to `Platform`, `/org/asuslinux/Led` → `Aura`.)
* Platform members: `NextThrottleThermalPolicy`, `SupportedInterfaces() -> as`, `SupportedProperties`, `ChargeControlEndThreshold y`, `DgpuDisable b`, `EgpuEnable b`, `GpuMuxMode y` (set takes `GpuMode`), `MiniLedMode b`, `NvDynamicBoost y`, `NvTempTarget y`, `PanelOd b`, `PostAnimationSound b`, `PptApuSppt y`, `PptFppt y`, `PptPl1Spl y`, `PptPl2Sppt y`, `PptPlatformSppt y`, `ThrottleThermalPolicy u` (`ThrottlePolicy { Balanced=0, Performance=1, Quiet=2 }`).
* FanCurves: `FanCurveData(u)`, `ResetProfileCurves(u)`, `SetActiveCurveToDefaults()`, `SetFanCurve`, `SetFanCurvesEnabled`, `SetProfileFanCurveEnabled`.
* Aura: `LedPower -> AuraPowerDev`, `SetLedPower((AuraPowerDev, bool))`, `DeviceType -> AuraDevice`.

### 3.2 6.0.0 – 6.0.12 (May – Aug 2024)

* Bus name still `org.asuslinux.Daemon`; interfaces renamed per controller: `org.asuslinux.Platform`, `org.asuslinux.FanCurves`, `org.asuslinux.Anime`, `org.asuslinux.Slash` (all at `/org/asuslinux`) and `org.asuslinux.Aura` at `/org/asuslinux/<device>` (multiple devices, hot-plug; ObjectManager under `/org/asuslinux`, later moved to `/`).
* Platform gained `Version`, `ThrottleBalancedEpp/ThrottlePerformanceEpp/ThrottleQuietEpp u`, `ThrottlePolicyLinkedEpp b`, `ThrottlePolicyOnAc/OnBattery u`, `ChangeThrottlePolicyOnAc/OnBattery b` (6.0.12), `BootSound b` replaced `PostAnimationSound`; `SupportedInterfaces` removed in 6.0.12.
* Slash (6.0.x): `Enabled`, `Brightness y`, `Interval y`, `SlashMode u`.
* `ThrottlePolicy` still `u` 0..2; `CPUEPP` `u`; enums in Aura already `u`/`s` as today.

### 3.3 6.1.0-rc2 (2025-01-12) → 6.1.0 (2025-02-04): the `xyz.ljones` rename

`asusd/src/lib.rs` at 6.1.0-rc1 still has `org.asuslinux.Daemon`; at 6.1.0-rc2 it is `xyz.ljones.Asusd` with `ASUS_ZBUS_PATH = "/xyz/ljones"`. Along with the rename:

* `ThrottlePolicy` → `PlatformProfile` (+ `LowPower`, `Custom`); `ThrottleThermalPolicy` → `PlatformProfile`, `NextThrottleThermalPolicy` → `NextPlatformProfile`, `Throttle*Epp` → `Profile*Epp`, `ThrottlePolicyOnAc` → `PlatformProfileOnAc`, etc. (6.1.0-rc6: "Move to using platform_profile api only (no throttle_thermal_policy)").
* All `ppt_*`, `nv_*`, `panel_od`, `mini_led_mode`, `boot_sound`, `dgpu_disable`, `egpu_enable`, `gpu_mux_mode` properties **removed** from Platform and replaced by `xyz.ljones.AsusArmoury` objects (6.1.0-rc2 "asus-armoury driver support"). Per-profile/per-AC tuning groups and `EnablePptGroup` (rc5–rc7).
* `OneShotFullCharge` added; `PlatformProfileChoices` added by 6.1.12.
* 6.1.11: `xyz.ljones.Backlight` (screenpad).
* 6.3.8: `Slash` gains `ShowOnLidClosed`.

### 3.4 6.3.8 (GitLab) → 6.4.0 (GitHub, Aug 2026) diff affecting clients

| Change | Where |
|---|---|
| New interface `xyz.ljones.XgmLed` at `/xyz/ljones` (`XgmLedEnabled b`) | `gh:asusd/src/ctrl_xgm_led.rs`, `gh:rog-dbus/src/zbus_xgm_led.rs` |
| Platform: new `DisableNvidiaPowerdOnBattery b` | `gh:rog-dbus/src/zbus_platform.rs` |
| Platform: `EnablePptGroup=true` fails unless a fan curve is enabled for the current profile (when fan curves are supported) | `gh:asusd/src/ctrl_platform.rs` |
| Slash: `Mode` property is `y` (raw mode byte) in both directions; proxy `mode() -> u8` | `gh:rog-dbus/src/zbus_slash.rs` |
| `FirmwareAttribute::PptEnabled` removed; `FirmwareAttributeType::Norestore` added; `apu_mem` → Norestore; `charge_mode` → ReadOnly | `gh:rog-platform/src/asus_armoury.rs` |
| `rog_dbus`: `has_iface`, `has_iface_blocking` removed; `find_iface_blocking` added; `list_iface_blocking` now sorted+deduped | `gh:rog-dbus/src/lib.rs` |
| `asusd.ron` gains `xgm_led_enabled: Option<bool>` | `gh:asusd/src/config.rs` |
| rog-control-center drops `supergfxctl`; GPU status from `rog_platform::gpu_pci` (sysfs/PCI) | `gh:rog-platform/src/gpu_pci.rs` |
| CPU EPP attributes optional (missing EPP no longer disables `CPUControl`) | `gh:rog-platform/src/cpu.rs` |
| AC/DC and lid polling replaced by event monitors | `gh:asusd/src/lib.rs` |
| Desktop file id `org.opengamingcollective.rog-control-center` | `gh:rog-control-center/data/` |

Suggested detection strategy for the GUI: try `NameHasOwner("xyz.ljones.Asusd")`, else `org.asuslinux.Daemon`; read `Version` (present from 6.0.0; absent on 5.x → treat as legacy); on `xyz.ljones` enumerate ObjectManager and gate features on interface presence and on `Platform.SupportedProperties`.

---

## 4. Daemons, configs, and the official GUI

### 4.1 `asusd` config files (all RON, all under `/etc/asusd/`, written by the daemon; `config_traits` migrates older `.cfg`/`.conf` names)

| File | Struct | Contents |
|---|---|---|
| `asusd.ron` | `asusd::config::Config` (`gitlab:asusd/src/config.rs`) | `charge_control_end_threshold: u8` (default 100), `base_charge_control_end_threshold: u8`, `disable_nvidia_powerd_on_battery: bool` (true), `ac_command: String`, `bat_command: String` (run on AC/DC change), `platform_profile_linked_epp: bool` (true), `platform_profile_on_battery` (Quiet), `change_platform_profile_on_battery` (true), `platform_profile_on_ac` (Performance), `change_platform_profile_on_ac` (true), `profile_quiet_epp` (Power), `profile_balanced_epp` (BalancePower), `profile_performance_epp` (Performance), `profile_custom_epp` (Performance), `ac_profile_tunings: HashMap<PlatformProfile, Tuning { enabled: bool, group: HashMap<FirmwareAttribute, i32> }>`, `dc_profile_tunings`, `armoury_settings: HashMap<FirmwareAttribute, i32>`, `screenpad_gamma: Option<f32>`, `screenpad_sync_primary: Option<bool>`; **[6.4.0]** `xgm_led_enabled: Option<bool>`. Migrations from `Config611` and `Config601` (which still had flat `ppt_pl1_spl: Option<u8>` etc.). **The daemon inotify-watches this file and hot-reloads it**, emitting `PropertiesChanged` for charge limit and armoury limits. |
| `fan_curves.ron` | `FanCurveConfig { profiles: FanCurveProfiles }` | per-profile `Vec<CurveData>` |
| `aura_<idProduct>.ron` (e.g. `aura_19b6.ron`, `aura_tuf.ron`) | `AuraConfig` (`gitlab:asusd/src/aura_laptop/config.rs`) | `brightness`, `current_mode`, `builtins: BTreeMap<AuraModeNum, AuraEffect>`, `multizone: Option<BTreeMap<AuraModeNum, Vec<AuraEffect>>>`, `multizone_on`, `enabled: LaptopAuraPower`, `ally_fix` |
| `anime.ron` | `AniMeConfig` | `system/boot/wake/shutdown: Vec<ActionLoader>`, `display_enabled`, `display_brightness`, `builtin_anims_enabled`, `off_when_*`, `brightness_on_battery`, `builtin_anims` |
| `slash.ron` | `SlashConfig` | `enabled`, `brightness` (255), `display_interval`, `display_mode` (Bounce), `show_on_*` |
| `asusd_user_ledmodes.ron` | `LedSupportFile` | optional user overrides appended to the support DB |

Static data: `/usr/share/asusd/aura_support.ron` (LED capability DB, `rog-aura/data/aura_support.ron`), `/usr/share/asusd/anime/{asus,custom}/*.gif|png`, `/usr/share/rog-gui/layouts/*.ron` (per-key keyboard layouts for the GUI/user daemon), udev `/usr/lib/udev/rules.d/99-asusd.rules`, D-Bus policy `/usr/share/dbus-1/system.d/asusd.conf`, units `/usr/lib/systemd/system/{asusd,asus-shutdown}.service`, `/usr/lib/systemd/user/asusd-user.service` (`gitlab:Makefile`).

### 4.2 `asusd-user` (per-user daemon, `gitlab:asusd-user/src/*`)

* Runs as a **user** systemd service (`asusd-user.service`, `WantedBy=default.target`). Connects to the **system** bus as a client of `xyz.ljones.Anime` / `xyz.ljones.Aura` (uses `list_iface_blocking` to check they exist).
* Config dir `~/.config/rog/`: `rog-user.ron` (`ConfigBase { active_anime: Option<String>, active_aura: Option<String> }`, defaults `"anime-default"`, `"aura-default"`), `anime-default.ron` (`ConfigAnime { name, anime: Vec<ActionLoader> }`), `aura-default.ron` (`ConfigAura { name, aura: AdvancedEffects }` — per-key effects `Static`, `Breathe`, `DoomFlicker` on `LedCode`s). Layouts from `/usr/share/rog-gui/`.
* If an AniMe sequence is active it also **registers `xyz.ljones.Asusd` on the SESSION bus**, object `/xyz/ljones/Anime`, interface **`xyz.ljones.Asusd`** with methods `InsertAsusGif(u s u u d) -> s`, `InsertImage(u s d d (dd) d) -> s`, `InsertImageGif(u s d d (dd) u u d) -> s`, `InsertPause(u t) -> s`, `RemoveItem(u) -> s`, `SetState(b)` (`gitlab:asusd-user/src/{ctrl_anime.rs,zbus_anime.rs}`).
* If an Aura sequence is active it streams `DirectAddressingRaw` packets to the system daemon every 33 ms.

### 4.3 `asus-shutdown` (`gitlab:asus-shutdown/src/main.rs`)

System service, `Requires=asusd.service`, holds a logind **shutdown delay inhibitor**; on `PrepareForShutdown(true)` it enumerates every `xyz.ljones.AsusArmoury` object with `QueuedGpuValue != -1`, waits for the dGPU to be idle (up to 15 s, polls `/proc` fds / runtime status; also stops `nvidia-powerd`/`nvidia-persistenced`), then calls `ApplyQueuedGpuValue`. Running the binary without `IS_SERVICE=1` prints a dry-run of pending actions.

### 4.4 ROG Control Center — the feature bar (`gitlab:rog-control-center/`, Slint UI)

| Page (`ui/pages/*.slint`) | Exposed controls | D-Bus used |
|---|---|---|
| **System** | Platform profile (buttons from `PlatformProfileChoices`), EPP per Balanced/Performance/Quiet, "link EPP", AC/battery profile + auto-change toggles, charge limit slider, panel overdrive, Mini-LED, boot sound, screen auto brightness, MCU powersave, screenpad brightness / gamma / sync, **PPT group enable** + sliders with min/max/default for `ppt_pl1_spl`, `ppt_pl2_sppt`, `ppt_pl3_fppt`, `ppt_fppt`, `ppt_apu_sppt`, `ppt_platform_sppt`, `nv_dynamic_boost`, `nv_tgp`, `nv_temp_target` (each with a reset-to-default); **[6.3.10+]** hardware monitor (CPU/GPU temp, usage, RAM, fan RPMs, battery health/power/time) | Platform, Backlight, AsusArmoury (subscribes to `CurrentValue` and Platform `PlatformProfile` changed signals to refresh min/max) |
| **Fans** | Per-profile (Balanced/Performance/Quiet) editable curves for CPU/GPU/MID, per-fan enable, reset to defaults | FanCurves |
| **GPU** | Dropdown Integrated / Ultimate (only if `gpu_mux_mode` exists) / Hybrid; "reboot required" toast; eGPU enabled state | AsusArmoury `dgpu_disable`, `gpu_mux_mode`, `egpu_enable` |
| **Aura** | Keyboard brightness, mode list from `SupportedBasicModes`, colour1/2, speed, direction, zone (multizone), power state matrix (boot/awake/sleep/shutdown × keyboard/logo/lightbar/lid/rear-glow/ally) | Aura |
| **AniMe** | Enable, brightness, builtins enabled, builtin animation choice per boot/awake/sleep/shutdown, off-when-lid/suspend/unplugged | Anime |
| **Slash** | Enabled, brightness, interval, mode list, show-on-boot/shutdown/sleep/battery, battery warning, lid closed | Slash |
| **[6.4.0] Battery** | charge limit page | Platform |
| **App settings** | run/start in background, tray icon, dGPU notifications, dark mode, `ac_command`/`bat_command`, Ally full-screen (`rog_ally` feature) | local config `~/.config/rog/rog-control-center.cfg` |
| **Tray** (ksni) | icon reflects GPU mode/power (6.3.8 via supergfxd `org.supergfxctl.Daemon` if present; 6.3.9+ via sysfs `dgpu_disable`/`gpu_mux_mode` + PCI runtime status); "Open/Quit ROGCC" | |
| **Notifications** | desktop notifications on dGPU power state change | |
| Single-instance IPC | `xyz.ljones.rogcc` on the **session** bus, `/xyz/ljones/rogcc`, property `State u` (`AppState`) | `gitlab:rog-control-center/src/zbus_proxies.rs` |

Not exposed in the GUI but available on D-Bus: `OneShotFullCharge`, `ProfileCustomEpp` (no property at all), `Backlight.ScreenpadPower`, `Aura.DirectAddressingRaw`, `Anime.Write`/`RunMainLoop`, `AsusArmoury` attributes the GUI has no widget for (`apu_mem`, `cores_performance`, `cores_efficiency`, `charge_mode`, `panel_hd_mode`, `egpu_connected`), ScsiAura, XgmLed (CLI only).

---

## 5. Laptop vs desktop vs ROG Ally

### 5.1 What each interface needs

| Interface | Requirement | Typical hardware |
|---|---|---|
| `Platform` (and therefore **the whole daemon**) | platform device **`asus-nb-wmi`** (`RogPlatform::new()` enumerates `subsystem=platform, sysname=asus-nb-wmi`; failure is `?`-propagated in `daemon.rs` → process exits) | ROG/TUF/Zephyrus/Strix/Vivobook/Zenbook/ProArt laptops, ROG Ally |
| `Platform.PlatformProfile*` | `/sys/firmware/acpi/platform_profile{,_choices}` | laptops via `asus-wmi` (or other `platform_profile` providers) |
| `Platform.ChargeControlEndThreshold` | a `power_supply` of type Battery with `charge_control_end_threshold` | laptops; generic kernel interface, works on non-ASUS too |
| `FanCurves` | `platform_profile` **and** hwmon `asus_custom_fan_curve` | G14/G15/G16/M16, some TUF (FA507…), 2022+ ROG |
| `AsusArmoury/*` | `/sys/class/firmware-attributes/asus-armoury/attributes/*` (kernel ≥ 6.19 `asus-armoury`) | each attribute only if the firmware exposes it |
| `Backlight` | `intel_backlight` or `asus_screenpad` backlight class device | laptops (`intel_backlight` only — AMD laptops get only the screenpad half) |
| `Aura` | USB `0b05:{1866,18c6,1869,1854,19b6,1a30,1abe,1b4c}` HID, or TUF DMI + `asus::kbd_backlight` with `kbd_rgb_mode` | laptop keyboards, Ally |
| `Anime` | DMI match (GA401/GA402/GU604/G635L/G835L) + USB `0b05:193b` | Zephyrus G14 2020–2022, M16, Strix Scar 2024/25 |
| `Slash` | `SlashType::from_dmi()` (GA403/GA605/GU605/G614 2024–2025) + USB `0b05:19b3` HID | Zephyrus G14/G16 2024+, Strix G16 2025 |
| `ScsiAura` | block device with `ID_VENDOR_ID=0b05`, `ID_MODEL_ID=1932` | ROG Strix Arion enclosure (any host) |
| `XgmLed` [6.4.0] | LED `asus:xgm*` | XG Mobile eGPU |

### 5.2 Desktop boards — observed on this machine (ROG CROSSHAIR X670E EXTREME, kernel 7.2.3-arch1-3)

* Platform devices: `eeepc-wmi` and `asus-ec-sensors` — **no `asus-nb-wmi`** (the NB driver only binds to notebooks; desktops get `eeepc-wmi`). Modules loaded: `asus_wmi`, `eeepc_wmi`, `asus_armoury`, `asus_ec_sensors`, `asus_rog_ryujin`.
* `/sys/class/firmware-attributes/asus-armoury/attributes/` contains **only `pending_reboot`** → asusd would register zero armoury objects.
* No `/sys/firmware/acpi/platform_profile`, no `power_supply`, no `backlight`.
* DMI: `sys_vendor=ASUS`, `product_family="To be filled by O.E.M."`, `chassis_type=3` → the udev rule's family match fails, so `asusd.service` is **never wanted**; even a manual `systemctl start asusd` exits at `RogPlatform::new()` with `asus-nb-wmi not found` (same in 6.4.0: `gh:asusd/src/daemon.rs` line 77 still `RogPlatform::new()?`).
* USB ASUS devices present (`0b05:18f3` AURA LED Controller, `1a60` ROG RYUJIN II 360, `1a83` ROG AZOTH, `1ace` ROG OMNI receiver, `1a21` OLED controller) — none are in asusd's product-id tables, so even a running asusd would ignore them.
* Practical consequence for OmaAsus: on a desktop there is no asusd to talk to. Desktop-side data must come from hwmon (`asusec`/`asus_ec_sensors`, `nct6799`, `asus` = `asus_wmi` hwmon with fan PWM, `rog_ryujin`, `k10temp`, `amdgpu`), and RGB from OpenRGB/`asus_rog_*` drivers. Design the D-Bus layer as optional: probe `NameHasOwner`, and fall back to a sysfs backend when absent.

### 5.3 ROG Ally

Treated as a laptop: `asus-nb-wmi` binds, `platform_profile` exists, armoury attributes (`ppt_*`, `mcu_powersave`, `charge_mode`, `apu_mem`, …) exist on recent kernels, keyboard/LED via `AuraDeviceType::Ally` (USB `1abe` Ally, `1b4c` Ally X) with the single `PowerZones::Ally` power zone (`AuraPowerState` packed into one byte). rog-control-center has a `rog_ally` cargo feature and `start_fullscreen`/`fullscreen_width/height` config. No AniMe/Slash/Backlight.

---

## 6. `asusctl` CLI (fallback control path)

Parsed with `argh` (`gitlab:asusctl/src/cli_opts.rs`, `gh:docs/usage/asusctl.md`). Runs as a normal user (D-Bus policy allows `users`/`wheel`). **Refuses to do anything if `Platform.Version != CARGO_PKG_VERSION`** ("Version mismatch"), or if the Platform proxy cannot be reached (prints systemd hints). Exit status is 0 even on many logical errors; parse stdout.

```
asusctl info [--show-supported]              # version, DMI, supported ifaces/properties, aura caps
asusctl profile list | get | next | set <Balanced|Performance|Quiet|LowPower|Custom> [-a|--ac] [-b|--battery]
asusctl profile tuning [true|false]          # [6.4.0] = Platform.EnablePptGroup
asusctl battery info | limit <20-100> | oneshot [percent]
asusctl fan-curve --get-enabled | --default
asusctl fan-curve --mod-profile <P> [--fan cpu|gpu|mid] [--data 30c:1%,49c:2%,...(8 pts)]
                  [--enable-fan-curves true|false] [--enable-fan-curve true|false --fan <f>]
asusctl leds get | set <off|low|med|high> | next | prev          # keyboard brightness
asusctl aura effect --next-mode | --prev-mode
asusctl aura effect <static|breathe|rainbow-cycle|rainbow-wave|stars|rain|highlight|laser|ripple|pulse|comet|flash>
                    [-c/--colour HEX] [--colour2 HEX] [--speed low|med|high] [--direction up|down|left|right] [--zone 0-7|none|one..four|logo|lightbar-left|lightbar-right]
asusctl aura power <keyboard|logo|lightbar|lid|rear-glow|ally> [--boot] [--awake] [--sleep] [--shutdown]   # 2021+ kbds; omitted = false
asusctl aura power-tuf [--awake true|false] [--boot true|false] [--sleep true|false] [--keyboard] [--lightbar]  # 0x1866 / TUF
asusctl anime [--enable-display B] [--enable-powersave-anim B] [--brightness off|low|med|high] [--clear]
              [--off-when-unplugged B] [--off-when-suspended B] [--off-when-lid-closed B] [--override-type GA401|GA402|GU604|G635L|G835L]
asusctl anime image|pixel-image|gif|pixel-gif --path F [--scale] [--x-pos] [--y-pos] [--angle] [--bright 0..1] [--loops N]
asusctl anime set-builtins [--boot A] [--awake A] [--sleep A] [--shutdown A] --set true
asusctl slash [--enable|--disable] [-l/--brightness 0-255] [--interval 0-5] [--mode NAME] [--list]
              [-B/--show-on-boot B] [-S/--show-on-shutdown B] [-s/--show-on-sleep B] [-b/--show-on-battery B] [-w/--show-battery-warning B]
asusctl slash get | set <same flags> | list      # [6.3.9+] subcommand form
asusctl scsi [--enable B] [--mode NAME] [--speed ...] [--direction ...] [--colours HEX]... [--list]
asusctl armoury list | get <sysfs_name> | set <sysfs_name> <i32>     # value -1 = restore default; GPU attrs are queued until shutdown
asusctl backlight [--screenpad-brightness 0-100] [--screenpad-gamma 0.5-2.2] [--sync-screenpad-brightness B]
asusctl xgmled get | set <0|1>                    # [6.4.0]
```

Mapping to D-Bus is one-to-one with §1 (e.g. `profile set X -a` → `Set PlatformProfileOnAc`, which also applies it; `armoury set` → `Set CurrentValue` on `/xyz/ljones/asus_armoury/<name>`; `fan-curve --default` → `SetCurvesToDefaults(active)`). `armoury list` prints `<Name>: current: min..[cur]..max / default:` using `AvailableAttrs` to decide what to show.

Common bindings: Fn+F5 → `asusctl profile next`; Aura keys → `asusctl aura effect --next-mode` / `--prev-mode`; Fn+F2/F3 → `asusctl leds prev|next`.

---

## 7. Notes and gotchas for the GUI implementation

1. **Discovery first.** Query `GetManagedObjects` on `/` and subscribe to `InterfacesAdded/Removed`; Aura/Slash/Anime/Scsi objects come and go with USB hot-plug and paths embed USB bus numbers.
2. **Property caching.** zbus proxies cache properties and update from `PropertiesChanged`; asusd only emits explicit changes for `PlatformProfile`, `EnablePptGroup`, `ChargeControlEndThreshold`, armoury `CurrentValue/Min/Max/Default/ScalarIncrement`, `PrimaryBrightness`, `ScreenpadPower`. Keyboard brightness changed by hotkeys, Aura mode, AniMe state etc. are **not** signalled — poll or re-read on focus.
3. **PPT semantics.** Reading an armoury PPT `CurrentValue` gives the stored per-(profile, AC/DC) value, not the live sysfs value. Changing profile or plugging AC changes which group you are editing; re-read `Min/Max` after every `PlatformProfile`/`EnablePptGroup` change (the GUI does exactly this). In 6.4.0, enabling the group requires an enabled custom fan curve where fan curves exist.
4. **GPU switching.** `Set CurrentValue` on `dgpu_disable`/`gpu_mux_mode`/`egpu_enable` returns immediately and only queues; show "reboot required", and read `QueuedGpuValue` to display pending state. `asus-shutdown.service` must be running for the write to happen.
5. **`SetCurvesToDefaults` flips the platform profile** temporarily; expect two `PlatformProfile` `PropertiesChanged` signals.
6. **Aura lock errors.** `LedMode`/`LedModeData` getters use `try_lock` and can fail spuriously; retry.
7. **Enum wire values** differ from Rust discriminants for `AuraModeNum::{Pulse,Comet,Flash}`, `AuraDeviceType::Unknown`, `PowerZones::None`, and `SlashMode` — see §2.3.
8. **Type mismatches in 6.3.8**: `Slash.Mode` getter (`y`, wrong value) vs setter (`u`); `ScsiAura.LedMode` getter `y` vs setter `u`. Read Slash mode from `DeviceState()` (`slash_mode` is a proper `u`) on 6.3.8; on 6.4.0 `Mode` is `y` = raw byte.
9. **Version gating**: `Platform.Version` string; 6.3.8 vs 6.4.0 differences in §3.4; older `org.asuslinux.*` in §3.1–3.2.
10. **Permissions**: any user in `users`/`wheel`/`sudo`/`adm` can call everything (no polkit); `asusd.ron` is root-owned but hot-reloaded, so editing it is an alternative write path for `ac_command`/`bat_command`/EPP defaults.
11. **Desktop boards**: no daemon at all (§5.2); make the whole asusd backend optional.

## Sources

* GitLab 6.3.8 tree (`https://gitlab.com/asus-linux/asusctl/-/raw/6.3.8/…`): `Cargo.toml`, `README.md`, `MANUAL.md`, `CHANGELOG.md`, `GPU_MODE_SWITCHING_SUMMARY.md`, `Makefile`, `data/{asusd.conf,asusd.service,asusd-user.service,asus-shutdown.service,asusd.rules}`, `asusd/src/{lib.rs,daemon.rs,config.rs,ctrl_platform.rs,asus_armoury.rs,ctrl_fancurves.rs,ctrl_backlight.rs,aura_manager.rs,aura_types.rs,error.rs,aura_laptop/{mod.rs,config.rs,trait_impls.rs},aura_anime/{config.rs,trait_impls.rs},aura_slash/{config.rs,trait_impls.rs},aura_scsi/trait_impls.rs}`, `rog-dbus/src/*.rs`, `rog-platform/src/{lib.rs,platform.rs,asus_armoury.rs,cpu.rs,power.rs,backlight.rs,keyboard_led.rs}`, `rog-profiles/src/{lib.rs,fan_curve_set.rs}`, `rog-aura/src/{lib.rs,builtin_modes.rs,aura_detection.rs,keyboard/{mod.rs,power.rs,advanced.rs}}`, `rog-anime/src/{lib.rs,data.rs,usb.rs}`, `rog-slash/src/{lib.rs,data.rs}`, `rog-scsi/src/builtin_modes.rs`, `asusd-user/src/{daemon.rs,config.rs,ctrl_anime.rs,zbus_anime.rs}`, `asus-shutdown/src/main.rs`, `asusctl/src/{main.rs,cli_opts.rs,aura_cli.rs,fan_curve_cli.rs,slash_cli.rs,anime_cli.rs,scsi_cli.rs}`, `rog-control-center/{Cargo.toml,src/{zbus_proxies.rs,config.rs,notify.rs,tray.rs,ui/*.rs},ui/pages/*.slint}`, `config-traits/src/lib.rs`.
* Older tags for §3: `rog-dbus/src/*.rs` and `asusd/src/lib.rs` at 5.0.10, 6.0.0, 6.0.12, 6.1.0-rc1, 6.1.0-rc2, 6.1.0, 6.1.12, 6.2.0; `rog-platform/src/platform.rs` at 6.0.12.
* GitHub `main` @ `d51d1497` (2026-09-08) / tag 6.4.0: `Cargo.toml`, `CHANGELOG.md`, `docs/SUMMARY.md`, `docs/usage/asusctl.md`, `asusd/src/{daemon.rs,ctrl_platform.rs,ctrl_xgm_led.rs,aura_slash/trait_impls.rs}`, `rog-dbus/src/{lib.rs,zbus_platform.rs,zbus_slash.rs,zbus_xgm_led.rs}`, `rog-platform/src/{platform.rs,asus_armoury.rs,cpu.rs,cled.rs,gpu_pci.rs}`, `asusctl/src/{cli_opts.rs,main.rs}`, `rog-control-center/ui/pages/*.slint`; GitHub releases API for dates.
* https://asus-linux.org/ (points to the OGC GitHub and Discord), crates.io API (publication status), Arch `pacman -Ss asusctl` (extra/asusctl 6.4.0-3), AUR RPC, `zvariant_derive` 5.15.0 docs (enum encoding rules), and local sysfs/udev/lsusb on the research machine for §5.2.
