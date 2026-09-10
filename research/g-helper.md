# G-Helper: what it teaches a Linux implementation

[G-Helper](https://github.com/seerge/g-helper) is a Windows control tool for ASUS
laptops. It drives the same firmware interface Armoury Crate uses (the ASUS
System Control Interface, `\\.\ATKACPI`) and the keyboard's USB HID interfaces.
Linux reaches that firmware through the kernel's `asus-wmi`, `asus-nb-wmi` and
`asus-armoury` drivers and through `asusd`.

**License.** G-Helper is GPL-3.0; OmaAsus is MIT. Nothing here is copied code.
This note records hardware facts (IDs, encodings, protocol bytes, behaviour)
that were read in G-Helper's source for reference and are to be verified on
hardware before OmaAsus relies on them. Entries carry a provenance label:
*verified* (checked on a named machine) or *reference* (seen in G-Helper,
unverified here). Citations are to G-Helper's `app/` tree as of 2026-09-10.

## 1. Firmware interface

Methods on `\\.\ATKACPI` (ioctl 0x0022240C): DSTS `0x53545344` (read), DEVS
`0x53564544` (write), INIT, WDOG (`AsusACPI.cs:32-38`).

**Presence.** A DSTS result with bit `0x00010000` set means the feature exists
(`AsusACPI.cs:481-487, 778-786`). The kernel names the same bits PRESENCE
`0x10000`, USER `0x20000`, BIOS `0x40000`. On Linux, attribute presence under
`/sys/devices/platform/asus-nb-wmi/` or
`/sys/class/firmware-attributes/asus-armoury/attributes/` is the equivalent
probe: the kernel only creates what the firmware reports.

**Device IDs and their Linux exposure** (verified on the ROG Zephyrus G14
GA403WR, kernel 7.2.3, asusd 6.4.0, unless marked):

| Feature | ID | Linux interface | GA403WR |
|---|---|---|---|
| Performance mode (ROG) | `0x00120075` 0 bal, 1 turbo, 2 silent, 3 full speed, 4 manual | `platform_profile`, `throttle_thermal_policy`, asusd `PlatformProfile` | present |
| Performance mode (Vivobook) | `0x00110019`, 1/2 swapped | same; kernel remaps | n/a |
| Fan RPM CPU/GPU/mid | `0x00110013/14/31`, low 16 bits = rpm/100 | hwmon `asus` `fanN_input` | CPU, GPU |
| Fan curves CPU/GPU/mid | `0x00110024/25/32`, 16 bytes: 8 temps °C + 8 duties % | hwmon `asus_custom_fan_curve` `pwmN_auto_pointM_{temp,pwm}` (temps in plain °C, duty 0-255), asusd `FanCurves` | CPU, GPU; 8 points |
| Fan hysteresis | `0x00110034` | none | - |
| Fan range (fallback) | `0x00110022/23` | none | - |
| CPU/GPU temperature | `0x00120094/97` | none (use `k10temp`, `amdgpu`, NVML) | - |
| PL1 / PL2 / fPPT | `0x001200A3 / A0 / C1` | armoury `ppt_pl1_spl`, `ppt_pl2_sppt`, `ppt_pl3_fppt` with `min_value`/`max_value` | present |
| APU sPPT / platform sPPT | `0x001200B0 / B1` | armoury (model dependent) | absent |
| GPU→CPU boost, CPU temp limit, cross-load | `0x0012009C / 9E / 9F` | none | - |
| NVIDIA dynamic boost / temp target | `0x001200C0 / C2` | armoury `nv_dynamic_boost`, `nv_temp_target` | present |
| dGPU base / extra TGP | `0x00120099 / 98` | armoury `nv_base_tgp`, `nv_tgp` | present |
| GPU Eco (dGPU off) | `0x00090020` (Vivo `0x00090120`) | `dgpu_disable`, armoury; owned by supergfxd | present |
| GPU MUX | `0x00090016` (Vivo `0x00090026`), 0 = dGPU direct | `gpu_mux_mode`, armoury, `pending_reboot` | present |
| XG Mobile | `0x00090018 / 19` | `egpu_*` | absent |
| Display on dGPU | `0x00090030` | none | - |
| Battery charge limit | `0x00120057` | `BATn/charge_control_end_threshold`, asusd `ChargeControlEndThreshold` | present |
| Charger type | `0x0012006C` | armoury `charge_mode` (read-only) | present |
| Panel overdrive | `0x00050019`, probe `0x00050020` | `panel_od`, armoury `panel_overdrive` | present |
| MiniLED / FHD / HDR | `0x0005001E` / `1C` / `0x00050071` | armoury (model dependent) / none for HDR | absent (OLED) |
| Keyboard backlight (ACPI) | `0x00050021`, `0x80 \| level` | `leds/asus::kbd_backlight` | present, max 3 |
| Boot sound | `0x00130022` | `boot_sound`, armoury | present |

The root-only `/sys/kernel/debug/asus-nb-wmi` interface can issue single
32-bit DSTS/DEVS calls. It could reach the single-value features Linux lacks
(temperatures, `9C/9E/9F`, HDR) but not buffer ones (hysteresis, fan range). It
is debugfs, so it is not a supported interface; the right home for missing
attributes is `asus-armoury` upstream.

**Values disagree between asusd and sysfs.** On battery the GA403WR's sysfs
reported `ppt_pl1_spl` 35 (range 15-35) while asusd's `CurrentValue` was 80,
and `nv_dynamic_boost` 0 against asusd's 25. asusd keeps per-profile, per-AC
values. Read current values and ranges from sysfs; write through asusd when it
runs so its stored copy stays consistent. Ranges change with AC state, so
re-read them when the charger connects or disconnects.

## 2. How G-Helper decides what a machine has

Three strategies (`AppConfig.cs`, `AsusACPI.cs`, `USB/AsusHid.cs`):

1. **Model-string rules**, case-insensitive substring match on the SMBIOS
   product name (`AppConfig.cs:182`). This is loose: `"503"` matches any model
   containing 503. It decides families (TUF, Strix, Vivobook, Zephyrus...),
   fan RPM scaling, firmware workarounds, charge-limit steps, lighting families.
2. **Firmware probing** (presence bit) for power-limit sliders, GPU controls,
   panel features, mid fan, full-speed mode.
3. **USB HID probing** of the keyboard (below) for lighting capabilities.

On Linux the SMBIOS product name is DMI `product_name`
("ROG Zephyrus G14 GA403WR_GA403WR"); the model code is `board_name`
("GA403WR"), the family `product_family` ("ROG Zephyrus G14"), and the BIOS
version `bios_version` ("GA403WR.309").

**For OmaAsus:** match model codes by exact or prefix `board_name`, families by
`product_family`, never by substring of the whole product name. Precedence:
user override > live probe > exact board > board prefix > family > default.
Keep only facts no probe can report.

**User overrides.** G-Helper's `config.json` can force quirks on or off
(`no_rgb`, `no_gpu`, `manual_mode`, `mode_reapply`, ...) and replace numbers
(`fan_max_N`, `fan_scale`). A user override file is a useful escape hatch.

## 3. Facts probes can't provide (reference, GA403 family)

| Fact | GA403 value | Source |
|---|---|---|
| Max fan speed CPU / GPU / mid | 6800 / 6800 / 8000 rpm | `FanSensorControl.cs:50-80` |
| Min fan speed | 2200 rpm | same |
| Slash bar LEDs | 7 (35 on GA405/GU405/GU606/GX651) | `SlashDevice.cs:128`, `AppConfig.cs:475` |
| Keyboard per-key direct colour | not supported; per-lamp via LampArray instead | `AppConfig.cs:530-545` |
| NVIDIA max power when the driver can't say | 90 W | `NvidiaSmi.cs:8-14` |
| Charge limit | firmware quantises; exact steps to verify | `BatteryControl.cs:71-76` |

Other models' workaround flags exist (toggle modes to force new limits on
GA403UI/UU/UV, custom fans required for PPT to stick on some models, re-apply
after resume). They need testing on Linux before adoption.

## 4. How it applies settings

- Cancel any apply in progress; set mode, then fan curves (after ~100 ms), then
  power limits (after ~1 s) (`ModeControl.cs:120-204`).
- If a custom curve write fails, set the mode again so firmware returns to its
  own curve (`ModeControl.cs:253-263`).
- Re-apply after resume, GPU switches and on firmware resets.
- Fan curve clean-up before writing: strictly increasing temperatures, an
  optional (30 °C, 0 %) start, optional per-point windows, a duty floor at high
  temperature (`AsusACPI.cs:703-753`, `Fans.cs:1507-1517`).
- GPU Eco guards: two dGPU-load samples above 10 %, XG Mobile, external display
  on the dGPU; write only on change; detect half-applied states; MUX changes
  need a reboot (`GPUModeControl.cs`). On Linux, supergfxd owns these.

## 5. Keyboard and Slash over HID (reference)

Keyboard `0b05:19b6` on the GA403WR exposes feature reports `0x5A` (62 bytes)
and `0x5D` (127 bytes), a HID LampArray (usage page `0x59`, reports
`0x41-0x46`) and vendor pages `0xFF89`/`0xFF82` (verified from the report
descriptor in sysfs). `/dev/hidraw*` is root-only by default.

- **Capability probe:** write `5D B9`, `5D "ASUS Tech.Inc."`,
  `5D 05 20 31 00 20`, read feature `0x5D`: byte 9 backlight type
  (single / 4-zone / per-key), 10 year, 12 layout, 13 feature bits (logo,
  lightbar, rear glow...), 14 more bits, 17 family (`AsusHid.cs:254-296`,
  `Aura.cs:328-383`).
- **Firmware effects** (static, breathe, cycle, rainbow, strobe...), brightness
  and power states: covered by asusd's `xyz.ljones.Aura`.
- **LampArray:** take control `5D C0 03 01` then control `01`; multi-update
  packets of up to 8 lamps; hand back with control `01` then `5D C0 04 01 01`
  (`AsusLampArray.cs`). This enables software effects per lamp (heatmaps,
  battery, audio). Must coordinate with asusd, which re-applies its state.
- **Slash:** 128-byte packets on report `0x5D` for 19b6. Firmware animations
  and options are covered by asusd's `xyz.ljones.Slash`; custom frames
  (`D3 00 00 <len> <brightness bytes>`), FX1-3 (`0x60-0x62`) and power saving
  (`D5`) are not (`SlashDevice.cs`).
- **asusd quirk (verified):** `SupportedBasicModes` lists Pulse as 10 while
  `AllModeData` keys it as 9.

## 6. Other devices (reference, future scope)

ROG mice (~30 models: DPI, polling, lighting, battery), ROG keyboards (Azoth,
Falchion, Strix Flare/Scope, Claymore II, TUF K1/K3), XG Mobile (lighting, fan
curve), ROG Ally controller modes and bindings. Protocols in `Peripherals/`,
`USB/XGM.cs`, `Ally/AllyControl.cs`.
