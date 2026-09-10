# Managing an ASUS ROG desktop from Linux — reference for the Rust control center

Target machine (verified live on this host, 2026-09-09):

| Component | Identity | Linux path / driver |
|---|---|---|
| Board | ROG CROSSHAIR X670E EXTREME, BIOS 3603 (03/09/2026) | `/sys/class/dmi/id/board_name` |
| CPU | Ryzen 9 7950X, `amd-pstate-epp` active, governor `performance`, EPP `performance` | `/sys/devices/system/cpu/cpu*/cpufreq/` |
| Super I/O | Nuvoton NCT6799D at 0x2E | `nct6775` driver, hwmon name `nct6799`, `/sys/class/hwmon/hwmon4` (platform `nct6775.656`) |
| EC sensors | `asus-ec-sensors` | hwmon `asusec`, `/sys/class/hwmon/hwmon7` |
| CPU temps | `k10temp` | `/sys/class/hwmon/hwmon5` (Tctl / Tccd1 / Tccd2) |
| AIO | ROG RYUJIN II 360 ARGB **EVA Edition**, USB `0b05:1a60` (bus 3 port 9; 2 interfaces: if0 vendor bulk, if1 HID) | `asus_rog_ryujin` bound by local hack (see 1.2), hwmon `rog_ryujin` `/sys/class/hwmon/hwmon12`, `/dev/hidraw14` |
| LiveDash OLED chip | "OLED Controller" USB `0b05:1a21` (bus 1 port 11, on the board's internal hub next to the AX210 Bluetooth; if0 vendor bulk `ep 0x01/0x81`, if1 HID) | no kernel driver; `/dev/hidraw12` (0600 root, no udev rule) |
| Aura USB | "AURA LED Controller" USB `0b05:18f3` (bus 3 port 11) | `/dev/hidraw16` (uaccess ACL present) |
| Lian Li | "LianLi-UNI FAN-SL-v1.8" ENE `0cf2:a100` (bus 3 port 10) | `/dev/hidraw15` (uaccess ACL present) |
| dGPU | RTX 4090 (ASUS TUF OC), driver 610.57.04 (nvidia-open-dkms), libnvidia-ml 610.57.04 | `/dev/nvidia0`, `card1` (DP-1..3, HDMI-A-1/2) |
| iGPU | Raphael (1002:164e) `amdgpu` | `/sys/devices/pci0000:00/0000:00:08.1/0000:12:00.0`, hwmon `amdgpu` `/sys/class/hwmon/hwmon3`, `card0` (DP-4/DP-5 unused) |
| RAM | 2x Corsair Dominator Platinum RGB DDR5 (SPD5118 temps hwmon9/10; RGB over SMBus PIIX4 `/dev/i2c-12`) | |
| Daemons present | `coolercontrold.service` **enabled+running** (running process is 4.3.1; package updated to 5.0.0, restart pending), `openrgb.service` disabled, `nvidia-powerd`/`nvidia-persistenced` disabled, no `lactd`, no `gamemoded`, `power-profiles-daemon` running, `polkitd` running | |
| Firmware attrs | `asus_armoury` loaded, `/sys/class/firmware-attributes/asus-armoury/attributes/` contains only `pending_reboot` (no tunables on this board) | |
| Session | Hyprland 0.56.2 (Omarchy), Steam native package installed, user `hive` in `wheel` | |

Everything below is organized by subsystem. Values marked **(observed)** were read from this machine.

---

## 1. Fan control

### 1.1 Nuvoton NCT6799D via `nct6775` — `/sys/class/hwmon/hwmon4`

Docs: <https://www.kernel.org/doc/html/latest/hwmon/nct6775.html>; source: `drivers/hwmon/nct6775-core.c`. The driver treats NCT6796D-S and NCT6799D-R identically. On ASUS boards it talks to the chip through the ASUS WMI/ACPI method (no `acpi_enforce_resources=lax` needed; the platform device is `nct6775.656`).

#### Sensors (observed)

| temp | label | value | note |
|---|---|---|---|
| temp1 | SYSTIN | 40.0 °C | motherboard |
| temp2 | CPUTIN | 36.0 °C | kernel doc: floats on ASUS boards, **ignore** |
| temp3..7 | AUXTIN0..4 | 22/23/23/14/26 °C | AUX headers; mostly unconnected garbage |
| temp8 | PECI/TSI Agent 0 Calibration | 41.0 °C | BIOS "CPU" temperature (Tctl minus offset). This is what the BIOS curves follow (`pwm*_temp_sel = 8`). Matches `asusec` CPU (40 °C). |
| temp9 | AUXTIN5 | -60 °C | unconnected |
| temp10..12 | PCH_* | 0 | unconnected |
| temp13 | TSI0_TEMP | 52.6 °C | raw Tctl over TSI (k10temp Tctl was 51.1 °C at the same time) |
| fan1..7 | (no labels) | all 0 rpm | no fans are plugged into board headers on this machine; all fans go through the Ryujin controller and the Lian Li hub |

`fanN_pulses` = 2 (pulses/rev), `fanN_min`, `fanN_target`, `fanN_tolerance`, `fanN_alarm/beep` also present. Which physical header (CPU_FAN, CPU_OPT, CHA_FAN1-3, AIO_PUMP, W_PUMP+, ...) maps to which `pwmN` is not exported by the driver; discover empirically (plug a fan, watch `fanN_input`). `pwm6`/`pwm7` have a flat 100 % curve from the BIOS, consistent with the AIO_PUMP/W_PUMP+ headers.

#### PWM attributes (per channel `pwm[1-7]`)

| attribute | semantics |
|---|---|
| `pwmN` | duty 0..255 (read-back of current output; writes only take effect in `pwmN_enable = 1`) |
| `pwmN_enable` | 0 = fan control off (**driver writes pwm=255 = full speed**), 1 = manual, 2 = "Thermal Cruise", 3 = "Fan Speed Cruise" (target RPM; untested), 4 = Smart Fan III (NCT6775F only, rejected here), 5 = **Smart Fan IV** (BIOS default here on all 7 channels) |
| `pwmN_mode` | 0 = DC (voltage, 3-pin), 1 = PWM (4-pin). Observed 1 on all channels. |
| `pwmN_temp_sel` | temperature source, value N ↔ `tempN_input` **as numbered in this hwmon dir**. Store validates that `tempN` exists (`have_temp` bit + `temp_src`), else `-EINVAL`. Observed 8 on all channels. |
| `pwmN_weight_temp_sel`, `_weight_duty_base`, `_weight_duty_step`, `_weight_temp_step`, `_weight_temp_step_base`, `_weight_temp_step_tol` | optional secondary source that *adds* `weight_duty_step` per `weight_temp_step` above `weight_temp_step_base`; 0 = disabled. Only `pwm2` exposes these on this chip (observed all 0). |
| `pwmN_target_temp` | Thermal Cruise target, millidegrees (0..127000). Unused in mode 5 (observed 0; pwm2 shows 50000). |
| `pwmN_temp_tolerance` | hysteresis band in millidegrees for modes 2 and 5 (observed 0) |
| `pwmN_crit_temp_tolerance` | tolerance around the critical temperature (observed 2000) |
| `pwmN_start` | duty used to spin the fan up (1..255) |
| `pwmN_floor` | minimum duty when below the range; 0 = allow the fan to stop |
| `pwmN_step_up_time` / `pwmN_step_down_time` | milliseconds between duty steps when the chip ramps (observed 0/0 — BIOS "fast" ramp) |
| `pwmN_stop_time` | ms below range before the fan is switched off (observed 24000) |
| `pwmN_auto_pointP_temp` / `pwmN_auto_pointP_pwm` | Smart Fan IV curve. On NCT6779+ (`auto_pwm_num = 4`) points **1..4 are the curve** and **point 5 is the critical point**: `auto_point5_temp` is `REG_CRITICAL_TEMP` (fan goes to `auto_point5_pwm`, 255 = "chip default full speed", any other value enables the custom critical-PWM bit). Units: millidegrees (store rounds to whole °C, max 255000) and 0..255. |

BIOS curve **(observed)** on pwm1..5: `(20 °C,153) (45,178) (60,216) (70,255) crit (125,255)`; pwm6/7: `(0,255) (100,255)x3 crit (100,255)`. So the BIOS ships "60 % at 20 °C" as the floor for fan headers.

How Smart Fan IV behaves: the chip linearly interpolates duty between consecutive points, moves toward the target by one step every `step_up_time`/`step_down_time`, applies `temp_tolerance` as hysteresis, and jumps to the critical PWM above `auto_point5_temp` (minus `crit_temp_tolerance`). Writing `5` to `pwmN_enable` runs `check_trip_points()`: temps and pwms of points 1..4 must be **monotonically non-decreasing**, and if a custom critical PWM is set, point 4 must be ≤ point 5, otherwise `-EINVAL` with `dev_err "Inconsistent trip points"`. Write the points first, then the mode.

#### Safe takeover / hand-back protocol

```
# take manual control of channel N (root)
saved_enable=$(cat pwmN_enable)           # remember (5 here)
echo 1 > pwmN_enable                      # manual
echo 128 > pwmN                           # 0..255
# hand back
echo $saved_enable > pwmN_enable          # curve registers are untouched by manual mode; the chip resumes the BIOS curve
```
Never write `0` (that pins 100 % and the driver forces pwm=255). If you want firmware-managed curves that survive a GUI/daemon crash, keep `pwmN_enable = 5` and edit `auto_point{1..4}_{temp,pwm}`, `temp_sel`, `step_*_time`, `temp_tolerance`, `floor`, `start` instead — the chip then does the work with no userspace loop. Re-apply after resume from suspend (BIOS/ACPI can reprogram the chip). CoolerControl's approach (`coolercontrold/daemon/src/repositories/hwmon/fans.rs`): read `pwmN_enable` at init and store it as `pwm_enable_default`; set `1` before writing duty; on "reset to default"/shutdown write the stored value back only if current < 2 (constants `PWM_ENABLE_MANUAL_VALUE=1`, `PWM_ENABLE_AUTO_VALUE=2`, `PWM_ENABLE_NCT6775_SMART_FAN_IV_VALUE=5`). It never edits Smart Fan IV curves.

All `pwm*` files are `0644 root` — writes need root (helper) — reads are unprivileged.

### 1.2 ROG Ryujin II via `asus_rog_ryujin` hwmon — `/sys/class/hwmon/hwmon12`

Docs: <https://docs.kernel.org/hwmon/asus_rog_ryujin.html>; source `drivers/hwmon/asus_rog_ryujin.c`. Upstream IDs: `0x1988` (Ryujin II 360), `0x1BCB` (III Extreme), `0x1ADE` (III EVA), `0x1ADA` (III White). **The EVA Edition `0x1A60` is not upstream**; `modinfo` shows only alias `hid:...p00001988`. This host binds it with a local hack: `/etc/systemd/system/ryujin-eva-bind.service` → `/usr/local/bin/ryujin-eva-bind.sh` writes `0003 0B05 1A60` to `/sys/bus/hid/drivers/asus_rog_ryujin/new_id` and binds the HID device; `/etc/udev/rules.d/71-asus-ryujin-eva.rules` grants `uaccess` to hidraw. A one-line upstream patch (add the ID with `has_controller = true`, same offsets as 0x1988 per liquidctl) would remove the hack.

| attr | meaning | observed |
|---|---|---|
| `fan1_input` "Pump speed" | rpm | 1740 |
| `fan2_input` "Internal fan speed" | the 60 mm blower inside the pump block | 0 (pwm2 = 0) |
| `fan3..6_input` "Controller fan 1..4 speed" | the external 4-port fan controller box (Ryujin II only) | 0/0/0/990 |
| `temp1_input` "Coolant temp" | millidegrees | 30500 |
| `pwm1` | pump duty | 168 (66 %) |
| `pwm2` | internal (pump-block) fan duty | 0 |
| `pwm3` | **all four** controller ports share one duty | 92 (37 %) |

Writes: `0644`, range 0..255, converted with `DIV_ROUND_CLOSEST(val*100,255)` to percent. Pump and internal fan are written together by the chip (`EC 1A 00 <pump%> <fan%>`); the driver first reads the other duty so a write to `pwm1` does not disturb `pwm2`. Controller duty is `EC 21 00 00 <duty>`. Each write waits for the device's confirmation report (timeouts → `-ETIMEDOUT`). The LCD is explicitly **not** handled by the driver. CoolerControl uses these hwmon files (it logs `AsusRyujin liquidctl driver not supported` and drives `rog_ryujin` as an hwmon device; current profiles: pump ← coolant temp, fan3 ← coolant temp).

### 1.3 liquidctl 1.16 `asus_ryujin.py` (hidraw, user-space)

Supports `0x1988, 0x1A60 (EVA), 0x1BCB, 0x1AA2, 0x1ADE, 0x1ADA` (so `liquidctl list` sees "ASUS Ryujin II 360 EVA Edition" here). Protocol: 65-byte HID reports, prefix `0xEC`.

| request | bytes | reply header |
|---|---|---|
| firmware | `EC 82` | `EC 02` + ASCII at [3:18] |
| cooler status | `EC 99` | `EC 19` — Ryujin II: temp int at byte 3, tenths at 4; pump rpm u16le at 5; internal fan rpm at 7 |
| cooler duty | `EC 9A` | `EC 1A` — pump %, fan % |
| controller speeds | `EC A0` | `EC 20` — 4 x u16le rpm |
| controller duty | `EC A1` | `EC 21` — 0..255 |
| set cooler | `EC 1A 00 <pump%> <fan%>` | (Ryujin III uses channel byte 1) |
| set controller | `EC 21 00 00 <0..255>` | |

Channels: `pump`, `pump-fan`, `fans` (internal + external together), `external-fans`; duty 0..100 %. `set_screen()` raises `NotSupportedByDriver` ("Not yet reverse engineered"). Caveat: the kernel driver and liquidctl talk to the **same** HID interface; interleaved replies confuse both. Pick one — the hwmon driver.

### 1.4 Lian Li UNI FAN SL hub (`0cf2:a100`) — `/dev/hidraw15`

Sources: liquidctl `lianli_uni.py` (credits uni-sync), uni-sync (Rust, <https://github.com/EightB1ts/uni-sync>), OpenRGB `LianLiUniHubSLController`. The hub has 4 channels; each channel daisy-chains up to 4 fans, one tach per channel.

Report descriptor (observed): report ID `0xE0`, 64-byte input + 64-byte output + feature report. Protocol (SL / PID 7750 & a100):

| operation | bytes |
|---|---|
| enable **PWM sync** on channel c (hub follows the motherboard 4-pin header it is cabled to) | `E0 10 31 (0x11 << (c-1))` |
| disable PWM sync (manual mode) | `E0 10 31 (0x10 << (c-1))` |
| set fixed speed on channel c | `E0 (0x20 + c-1) 00 <speed_byte>`; SL: `speed_byte = int((800 + 11*duty)/19) & 0xFF`, duty 0 → 40 (SL-Infinity: `(250+17.5*duty)/20`, 0→10; SL v2/AL v2: `(200+19*duty)/21`, 0→7) |
| read RPM | `get_input_report(0xE0, 65)`, big-endian u16 per channel at offset `1 + 2*(c-1)` (v2 hubs: offset 2) |
| RGB activate | `E0 10 32 (0x10*c + num_fans)` |
| RGB mode | `E0 (0x10+c) <mode> <speed> <direction> <brightness>` (modes: static 0x01, breathing 0x02, rainbow 0x05, rainbow morph 0x04, color cycle 0x23, runway 0x1C, staggered 0x18, tide 0x1A, meteor 0x24, mixing 0x1E, stack 0x20, stack multi 0x21, neon 0x22; brightness 0x00 brightest .. 0x08 off; speed 0x02 slowest .. 0xFE fastest) |
| RGB colors | `E0 (0x30+c) <RGB x (16 LEDs x fans)>` |
| merge channels | `E0 10 33 00 01 02 03 08` (merged) / `E0 10 34` (unmerged) |

uni-sync: `/etc/uni-sync/uni-sync.json` (per-controller `sync_rgb`, per-channel `mode: PWM|Manual`, `speed`), runs once at boot (systemd unit) — not a daemon. On this host CoolerControl drives `fan1` manually from a custom sensor (`cpu_gpu_max`). Alternative: enable PWM sync and cable the hub to a CHA_FAN header, then the NCT6799 Smart Fan IV curve controls chassis fans with zero software.

### 1.5 `asus-ec-sensors` — `/sys/class/hwmon/hwmon7` (read-only)

temp1 CPU 40 °C, temp2 CPU Package 52 °C, temp3 Motherboard 40 °C, temp4 T_Sensor -60 (none), temp5 VRM 35 °C, temp6/7 Water_In/Water_Out -40 (no flow sensor header used). Also `spd5118` DIMM temps (hwmon9/10) and NVMe temps (hwmon0/1/2).

---

## 2. Displays: Ryujin LCD and LiveDash OLED

### 2.1 Which device is which

* **`0b05:1a21` "OLED Controller" is the motherboard's LiveDash OLED/AniMe-Matrix chip**, not the Ryujin. Evidence: it sits on the internal USB hub (bus 1, port 11, next to the onboard AX210 Bluetooth at port 6) while the Ryujin (`1a60`) is on bus 3 behind the front-panel/ASMedia path; the Polylux project drives the same PID on a ROG Maximus Z690 Extreme as "LiveDash OLED + AniMe Matrix". The X670E Extreme also has both (ROG forum "ROG CROSSHAIR X670E EXTREME: AniMe Matrix LED display and LiveDash OLED").
* **The Ryujin II's 3.5" LCD (320x240) is on the AIO's own USB device** (`0x1988`/`0x1a60`, interface 0 vendor-bulk `ep 0x01 OUT`, interface 1 HID).
* Both devices expose byte-identical HID report descriptors (vendor page `0xFF72`, report ID `0xEC` 64-byte in/out, report ID `0xEE` 64-byte input). The Aura controller `18f3` uses the same `0xEC` report without the `0xEE` input.

### 2.2 Prior art

* Polylux (<https://github.com/vladulus/polylux>, Windows, MIT, "Claude's project"): clean-room USBPcap decode of Armoury Crate. Protocol notes in `docs/PROJECT_STATE.md` §9.2/§9.3, drivers in `polylux/drivers/chip_1a21.py`, `livedash_oled/driver.py`, `ryujin_lcd/driver.py`. **Status: OLED text mode works; OLED custom image upload is byte-correct but display activation is blocked by an ASUS firmware bug on Z690 Extreme (Armoury Crate fails identically); Ryujin LCD upload "decoded but unverified".**
* liquidctl: `set_screen` not implemented for Ryujin; issue #677 (Ryujin III) and the asus-ryujin guide mention no LCD. OpenRGB issue #690 is an open feature request for LiveDash/AIO LCDs. No Linux project drives either display today.

### 2.3 LiveDash OLED (`1a21`) protocol (from Polylux; verified on Z690 Extreme, unverified on X670E)

All HID packets are 65 bytes (`0xEC` first) written as HID output reports on interface 1; bulk data goes to interface 0 `ep 0x01`.

| purpose | bytes |
|---|---|
| heartbeat | `EC DC 00` |
| firmware string | `EC 82 00` (reply `EC 02` + ASCII, same as Aura) |
| init (matrix data mode) | `EC DC 00` x3 (50 ms apart), `EC 82 00`, `EC C1 00`, `EC 42 01` |
| chip mode switch | `EC 51 NN`: `00` OLED hardware-monitor, `01` Q-code, `09` OLED text mode, `02..05`/`11` matrix preset animations, `10` OLED preset GIFs, `15` **exit preset → data-input mode** |
| OLED text (2 lines) | `EC 53 00` + label ASCII at bytes 3..20 (18 B) + value UTF-8 at bytes 21..64 (44 B) — send `EC 51 09` once first |
| OLED custom GIF upload | `EC 72 01 00 01 00 00 00`, `EC 51 00`, `EC 73 01`, `EC 7F 02 sizeLO sizeHI 00 00 00`, then BULK OUT GIF87a (256x64 mono, ≤100 KB) zero-padded to 4096 B, then `EC 73 FF` |
| AniMe Matrix frame | `EC 7F 04 00 03` (+60 zeros) then BULK OUT 768 B planar R,G,B (mapping is a non-raster LUT; see Polylux `anime_matrix/lut.py`) |

Boot ownership: the BIOS uses the same protocol during POST (Q-codes, then hardware-monitor). On Linux nothing else drives the chip after boot, so no service must be killed; the OLED simply keeps the BIOS state. Send `EC 51 15` before any data writes.

Linux access: interface 1 → `/dev/hidraw12` (needs a udev rule: `KERNEL=="hidraw*", ATTRS{idVendor}=="0b05", ATTRS{idProduct}=="1a21", TAG+="uaccess"`); interface 0 has no kernel driver (`Driver=[none]`), so `rusb`/libusb can claim it directly with a matching `SUBSYSTEM=="usb", ATTR{idVendor}=="0b05", ATTR{idProduct}=="1a21", TAG+="uaccess"` rule. Rust: `hidapi 2.6.7` (feature `linux-native` or `linux-static-hidraw`) + `rusb`.

### 2.4 Ryujin II LCD protocol (from Polylux; unverified)

```
HID  EC 71 01 01 00 00 00 00      switch to custom-image mode
HID  EC F1 00 00 00 00 00 00      unknown, always present
HID  EC 72 01 02 01 00 00 00      register upload (byte 3 = 0x02 selects the LCD; 0x00 = OLED)
HID  EC 73 01 00 00 00 00 00      start
HID  EC 7F 02 sLO sMID sHI 00 00  24-bit LE size of the unpadded GIF
BULK ep 0x01: GIF89a 320x240, in 4096-byte chunks, zero padded
HID  EC 73 FF 00 00 00 00 00      end/commit
```
Armoury Crate also streams a hardware-monitor text overlay (`EC 52`/`EC 53`) continuously. The stock LCD firmware shows its own hardware-monitor page when nothing is pushed. ASUS FAQ 1048625 describes the Windows features: custom image/GIF, hardware monitor (CPU temp/freq, coolant, fan/pump), banner text.

Practical plan: expose "LCD/OLED" in the GUI as experimental; implement text mode on the OLED first (low risk, one HID write), keep image upload behind a flag, and capture a USBPcap on a Windows install of this exact board if custom images matter. CoolerControl's LCD pipeline (`PUT /devices/{uid}/settings/{channel}/lcd`, `LcdInfo` with `screen_width/height`, `gif_supported`) only covers liquidctl-supported screens (NZXT Kraken, Corsair), not ASUS.

---

## 3. RGB / Aura

### 3.1 OpenRGB 1.0rc3 on this machine (observed via `openrgb --list-devices`)

0. Corsair Dominator Platinum RGB DDR5 x2 (SMBus PIIX4 `/dev/i2c-12` @0x19/0x1B; modes Direct, Custom, Color Shift, ...)
1. ASUS TUF GeForce RTX 4090 Gaming OC (ENE SMBus on NVIDIA i2c `/dev/i2c-1` @0x67; 4 LEDs; Direct/Off/Static/Breathing/Flashing/Spectrum Cycle/Rainbow/Chase Fade/Chase/Random Flicker)
2. Lian Li Uni Hub - SL (`/dev/hidraw15`, 4 channel zones, 15 modes)
3. **ASUS ROG CROSSHAIR X670E EXTREME** "ASUS Aura USB Mainboard Device" (`/dev/hidraw16`, serial 9876543210): zones `Aura Mainboard` (6 board LEDs + `RGB Header 1/2`), `Aura Addressable 1..3` (resizable ARGB headers); modes Direct, Off, Static, Breathing, Flashing, Spectrum Cycle, Rainbow, Chase Fade, Chase
4. ASUS ROG Azoth keyboard.

Warning printed: `60-openrgb.rules` exists in both `/etc/udev/rules.d` and `/usr/lib/udev/rules.d` — delete the `/etc` copy. The rules give `TAG+="uaccess"` (ACL for the active seat) on hidraw and i2c-dev; `i2c-dev` + `i2c-piix4` must be loaded for DRAM/GPU. Nothing needs root once ACLs are in place, and **no OpenRGB server is required for the CLI**; for the SDK you must run one: `openrgb --server --server-port 6742` (system unit `openrgb.service` exists, disabled; a user unit is preferable because the hidraw ACLs belong to the seat user). Only one process may open the HID devices → route all RGB through the server.

### 3.2 OpenRGB SDK network protocol (NetworkProtocol.h, protocol version **6** in 1.0rc3)

TCP `127.0.0.1:6742`. Every packet: header `"ORGB"` (4 B) + `pkt_dev_idx u32` + `pkt_id u32` + `pkt_size u32` (all little-endian) + payload. Handshake: `REQUEST_PROTOCOL_VERSION (40)` (payload u32 client version; server answers with its version, min of the two is used), `SET_CLIENT_NAME (50)` (NUL-terminated string), optional `SET_CLIENT_FLAGS (52)`. Then `REQUEST_CONTROLLER_COUNT (0)` → u32, `REQUEST_CONTROLLER_DATA (1)` per index (payload u32 protocol version; reply is the serialized `RGBController`: size, type, name, vendor, description, version, serial, location, mode count/active mode, modes[] (name, value, flags, speed/brightness/direction min-max-cur, color mode, colors[]), zones[] (name, type, leds min/max/count, matrix map; v4+ segments; v5+ flags), leds[] (name, value), colors[] (u32 BGR-order `0x00BBGGRR`)). Control: `RGBCONTROLLER_UPDATELEDS (1050)` (payload: u32 size, u16 count, colors), `UPDATEZONELEDS (1051)`, `UPDATESINGLELED (1052)`, `SETCUSTOMMODE (1100)`, `UPDATEMODE (1101)` (u32 size, u32 mode index, serialized mode), `SAVEMODE (1102)`, `UPDATEZONEMODE (1103)`, `RESIZEZONE (1000)`. Server pushes `DEVICE_LIST_UPDATED (100)`. v6 additions: profile manager (`150..160`), plugin manager (`200/201`), settings manager (`250..254`), log manager (`300..304`), detection (`101..103`, `140`), device info (`120..124`), `SETHIDDEN (1005)`, `SIGNALUPDATE (1150)`, ACK (`10`) with status codes 0 OK / 1 generic / 2 unsupported / 3 not allowed / 4 invalid id / 5 invalid data.

Rust crates: **`openrgb2` 0.4.1** (2026-09-05, tokio async, protocol 6 aware, "successor to openrgb"; some v6 packets not yet on the client API) — use this. `openrgb` 0.1.2 (2022) is dead. Community wiki copy of the protocol doc: `Documentation/OpenRGBSDK.md` in the OpenRGB repo.

### 3.3 Direct HID protocol for the Aura USB controller (`18f3`) if bypassing OpenRGB

From OpenRGB `AsusAuraUSBController.{h,cpp}` and liquidctl `aura_led.py` (which lists `0x18F3` as "not fully tested"). 65-byte reports, byte0 `0xEC`:

* `EC 82` → firmware (reply `EC 02` + 16 ASCII); `EC B0` → config table (reply `EC 30`, 60 bytes from offset 4; OpenRGB derives mainboard LED count and header count from it, 6-byte rows).
* Direct mode: `EC 40 <(apply?0x80:0)|channel> <offset> <count> <RGB...>` — max 20 LEDs (0x14) per packet; set bit 7 on the last chunk to apply. Direct channel IDs: mainboard 0x00, ARGB 1..3 = 0x00..0x02 by device index (liquidctl table: led1 rgb/0x01 effect id, led2..4 argb effect ids 0x02/0x04/0x08, direct ids 0x00/0x01/0x02).
* Effect mode: `EC 35 <channel_type 0=RGB,1=ARGB> 00 00 <mode>` then `EC 36 00 <channel_id bits> 00 <R G B ...>` then commit `EC 3F 55 00 00`; end direct `EC 35 00 00 00 00`; end effect `EC 35 00 00 00 FF`. Modes: 0 off, 1 static, 2 breathing, 3 flashing, 4 spectrum cycle, 5 rainbow, 6 spectrum-cycle breathing, 7 chase fade, 8 spectrum-cycle chase fade, 9 chase, 10 spectrum-cycle chase, 11 spectrum-cycle wave, 12 chase rainbow pulse, 13 random flicker (14 music; 0xFF direct); liquidctl adds 0x10..0x13 (gentle transition, wave propagation, wave propagation pause, red pulse).
* Note (Polylux): if "Aura" is disabled in BIOS the controller still ACKs writes but LEDs stay dark.

---

## 4. CPU — Ryzen 9 7950X on `amd-pstate-epp`

Doc: <https://docs.kernel.org/admin-guide/pm/amd-pstate.html>. Kernel 7.2.3-arch1-3, `amd_pstate/status = active`, `prefcore = enabled`, `dynamic_epp = disabled` (all observed).

| path | values | perms (observed) |
|---|---|---|
| `/sys/devices/system/cpu/amd_pstate/status` | `active` (EPP, firmware picks perf), `passive` (kernel governors set desired perf), `guided`, `disable`; writable at runtime | 0644 root |
| `/sys/devices/system/cpu/amd_pstate/prefcore` | `enabled`/`disabled` (kernel param `amd_prefcore=`) | 0444 |
| `/sys/devices/system/cpu/amd_pstate/dynamic_epp` | kernel-autonomous EPP (`amd_dynamic_epp=enable`) | 0644 root |
| `cpuN/cpufreq/scaling_governor` | `performance` or `powersave` (only these two exist in EPP mode) | 0644 root |
| `cpuN/cpufreq/energy_performance_preference` | `default`, `performance`, `balance_performance`, `balance_power`, `power`, or an integer 0..255 (0 = max perf). **Observed: with governor=performance the available list collapses to `performance` and EPP is pinned; switch the governor to `powersave` to make EPP selectable.** | 0644 root |
| `cpuN/cpufreq/scaling_min_freq` / `scaling_max_freq` | kHz; in EPP mode they become the CPPC min/max perf limits. Observed 3012481 (lowest non-linear) / 5883197 (max boost). `cpuinfo_min_freq` 425292 | 0644 root |
| `cpuN/cpufreq/boost` and global `/sys/devices/system/cpu/cpufreq/boost` | 0/1 (per-policy boost was added for amd-pstate; observed both = 1) | 0644 root |
| `cpuN/cpufreq/amd_pstate_highest_perf`, `amd_pstate_max_freq`, `amd_pstate_lowest_nonlinear_freq`, `amd_pstate_hw_prefcore`, `amd_pstate_prefcore_ranking` | read-only. Ranking observed: cores 1 and 6 = 236 (best), 4 = 231, 0 = 226 ... 12 = 166; larger = preferred, can change at runtime | 0444 |
| `cpuN/cpufreq/scaling_cur_freq` | kHz, APERF/MPERF-derived current | 0444 |
| `cpuN/cpufreq/cpuinfo_avg_freq` | kHz, average effective frequency over the last sampling window (amd-pstate, newer kernels; present here) — use this for the per-core frequency graph | 0444 |
| `/sys/devices/system/cpu/smt/control` | `on`/`off`/`forceoff`; `smt/active` | 0644 root |

Load: parse `/proc/stat` per `cpuN` line (user nice system idle iowait irq softirq steal) and diff; or `procfs 0.18` / `sysinfo 0.39` crates.

Temperatures: `k10temp` `temp1_input` Tctl (51.1 °C), `temp3` Tccd1, `temp4` Tccd2 (unprivileged). `asusec` temp2 "CPU Package" is another view.

Package power:
* RAPL: `/sys/class/powercap/intel-rapl:0` (`name = package-0`, `max_energy_range_uj = 65532610987`) and `intel-rapl:0:0` (`core`), driver `intel_rapl_msr` + `rapl`. **`energy_uj` is `0400 root`** since 5.10 (PLATYPUS, CVE-2020-8694) — read it from the root helper and differentiate; the `amd_energy` hwmon driver was removed from mainline (RAPL via powercap replaces it). The `constraint_*` power-limit files exist but are not writable/meaningful on AMD.
* perf: `/sys/bus/event_source/devices/power/events/energy-pkg` exists, but `perf_event_paranoid = 2` here → needs root or `CAP_PERFMON`.
* Unprivileged proxy **(observed)**: the Raphael iGPU hwmon `/sys/class/hwmon/hwmon3/power1_input` labeled `PPT` read 46.07 W at idle — on Raphael the amdgpu SMU reports the **socket** PPT, so this looks like package power without root. Validate against RAPL before trusting it.

PBO / overclocking from userspace (all root, all volatile until reboot, all "at your own risk"):
* `ryzen_smu` kernel module (out-of-tree; original gitlab.com/leogx9r/ryzen_smu unmaintained; active fork <https://github.com/amkillam/ryzen_smu> lists **Raphael as supported** incl. PM table; AUR `ryzen_smu-dkms-git`; whether it builds on 7.2 must be tested). Sysfs `/sys/kernel/ryzen_smu_drv/`: `drv_version`, `version`, `mp1_if_version`, `codename`, `pm_table_version`, `pm_table_size`, `pm_table` (binary, read-only), `smu_args` (6 x u32 LE = 24 B), `rsmu_cmd`, `mp1_smu_cmd`, `hsmp_smu_cmd`, `smn`. Sequence: write args → write command id → read command file for status (0x01 OK, 0xFF failed, 0xFB timeout ...).
* Raphael RSMU command IDs (ZenStates-Core `Hardware/Smu/Settings/Zen4Settings.cs`): mailbox MSG `0x03B10524`, RSP `0x03B10570`, ARG `0x03B10A40`; **PPT `0x56`** (mW), **TDC `0x57`** (mA), **EDC `0x58`** (mA), TctlMax `0x59`, PBO scalar set `0x5B` / get `0x6D`, boost limit all-core set `0x70` / get `0x6E`, IsOverclockable `0x6F`, EnableOcMode `0x5D` / Disable `0x5E`, all-core OC freq `0x5F`, per-core `0x60`, VID `0x61`, **Curve Optimizer per-core `0x06`, all-core `0x07`, get `0xD5`** (arg = core selector in the upper bits | signed margin in the low 16 bits — see `SetPsmMarginSingleCore.cs` for the exact packing), curve shaper `0xA6/0x84`, GetPboFusedPowerLimit `0xDC` (locked on some Zen4 boards), GetTableVersion `0x05`, TransferTableToDram `0x03`. MP1 equivalents: PPT `0x3E`, TDC `0x3C`, EDC `0x3D`, TctlMax `0x3F`, CO `0x35/0x36`.
* `ryzen_monitor` / `ryzen_monitor_ng` (mann1x) read the PM table (per-core power/volts/clocks/CO counts, PPT/TDC/EDC actuals) — Zen3-oriented, Zen4 PM-table layouts depend on the AGESA version. `ryzenadj` supports **APUs only** — not applicable to the 7950X. `ryzen_smu_hwmon` (FrozenGalaxy) exposes PM-table values as hwmon for 7950X3D.
* BIOS side: PBO limits set via SMU only take effect when PBO is enabled in BIOS; the BIOS re-applies its own values on every boot.

Tools: `cpupower` (package `cpupower`; not installed here) `frequency-set -g`, `frequency-info`; recent versions add `cpupower set --epp`. `power-profiles-daemon` is running (`org.freedesktop.UPower.PowerProfiles`, D-Bus property `ActiveProfile` = `power-saver|balanced|performance`) and on amd-pstate it maps profiles to EPP — either use it (unprivileged, polkit-gated D-Bus) or mask it to own EPP yourself; CoolerControl 5.0 and LACT both integrate with it.

Scheduler tweaks gamers use: `gamemode` (see §9; not installed: `pacman -S gamemode lib32-gamemode`), `ananicy-cpp` + `cachyos-ananicy-rules` (AUR; nice/ionice/sched-policy rules by process name, no API), `sched_ext` schedulers (`scx-scheds`: `scx_lavd`, `scx_bpfland`, controlled by `scx_loader` on D-Bus `org.scx.Loader` — `StartScheduler/SwitchScheduler/StopScheduler`, property `CurrentScheduler`), `split_lock_mitigate=0`, and pinning games to the preferred cores (`sched_setaffinity` using `amd_pstate_prefcore_ranking` — on a non-X3D 7950X both CCDs are equal so pinning to CCD0 (cpus 0-7,16-23) mostly reduces cross-CCD latency).

---

## 5. NVIDIA RTX 4090 (driver 610.57.04, NVML v13)

Observed: power limit 450 W (min 10, max 600, default 450), max graphics 3105 MHz, mem 10501 MHz, 2 fans, temperature.memory N/A (consumer), persistence off, clocks_event_reasons 0x1 (idle).

### 5.1 NVML functions (`libnvidia-ml.so.1`)

| capability | NVML | root? | `nvidia-smi` |
|---|---|---|---|
| temps | `nvmlDeviceGetTemperature(NVML_TEMPERATURE_GPU)`; hotspot/memory via `nvmlDeviceGetThermalSettings` / `nvmlDeviceGetTemperatureV` / field values | no | `--query-gpu=temperature.gpu` |
| clocks | `nvmlDeviceGetClockInfo(GRAPHICS/SM/MEM/VIDEO)`, `GetMaxClockInfo`, `GetClock(type, id)` | no | `clocks.gr,clocks.mem` |
| power | `nvmlDeviceGetPowerUsage` (mW), `GetPowerManagementLimit`, `GetPowerManagementLimitConstraints` | no | `power.draw` |
| utilization / VRAM | `nvmlDeviceGetUtilizationRates`, `GetMemoryInfo(_v2)` | no | `utilization.gpu,memory.used` |
| fan | `nvmlDeviceGetNumFans`, `GetFanSpeed_v2`, `GetTargetFanSpeed`, `GetMinMaxFanSpeed`, `GetFanControlPolicy_v2` | no | `fan.speed` |
| PCIe | `nvmlDeviceGetCurrPcieLinkGeneration/Width`, `GetPcieThroughput` | no | `pcie.link.gen.current` |
| throttle reasons | `nvmlDeviceGetCurrentClocksEventReasons` bitmask: 0x1 GpuIdle, 0x2 ApplicationsClocksSetting, 0x4 SwPowerCap, 0x8 HwSlowdown, 0x10 SyncBoost, 0x20 SwThermalSlowdown, 0x40 HwThermalSlowdown, 0x80 HwPowerBrakeSlowdown, 0x100 DisplayClockSetting | no | `clocks_event_reasons.active`, `-q -d PERFORMANCE` |
| power limit | `nvmlDeviceSetPowerManagementLimit(mW)` (v2 struct variant exists) | **yes** | `nvidia-smi -pl 400` |
| locked clocks | `nvmlDeviceSetGpuLockedClocks(min,max)` / `ResetGpuLockedClocks`; `SetMemoryLockedClocks` (Ampere+) / `ResetMemoryLockedClocks` | **yes** | `-lgc 1500,2700`, `-rgc`, `-lmc`, `-rmc` |
| clock offsets | **`nvmlDeviceSetClockOffsets(nvmlClockOffset_t*)`** with `{version = nvmlClockOffset_v1, type = NVML_CLOCK_GRAPHICS(0)/NVML_CLOCK_MEM(2), pstate = NVML_PSTATE_0, clockOffsetMHz}`; read `GetClockOffsets` (gives min/max). Deprecated: `SetGpcClkVfOffset(int)`, `SetMemClkVfOffset(int)`, `GetGpcClkMinMaxVfOffset`, `GetMemClkMinMaxVfOffset`. Driver 555+ required, 565+ recommended (LACT). | **yes** | none (X-only `nvidia-settings -a GPUGraphicsClockOffsetAllPerformanceLevels=…`) |
| fan control | `nvmlDeviceSetFanSpeed_v2(fan, pct)` (switches to manual — you must watch temps), `SetDefaultFanSpeed_v2(fan)` (back to auto; NVIDIA forum reports it sometimes fails to resume the algorithm — verify policy afterwards), `SetFanControlPolicy(fan, NVML_FAN_POLICY_TEMPERATURE_CONTINOUS_SW=0 / NVML_FAN_POLICY_MANUAL=1)` | **yes** ("Requires privileged user") | none on Wayland |
| persistence | `nvmlDeviceSetPersistenceMode` | yes | `nvidia-smi -pm 1` or `nvidia-persistenced.service` |
| PowerMizer | `nvmlDeviceSetPowerMizerMode` (v580+) | yes | none |

NVML is display-server agnostic, so all of this works under Hyprland/Wayland; `nvidia-settings` cannot (NV-CONTROL needs X). `Coolbits` is irrelevant for NVML paths. `nvidia-powerd` implements laptop Dynamic Boost (D-Bus `nvidia.powerd.server`) — keep disabled on a desktop. `nvidia-persistenced` is optional; it shortens NVML/CUDA init and keeps the GPU initialized when no client is open.

### 5.2 Rust: `nvml-wrapper` 0.13.0 (2026-08-31) / `nvml-wrapper-sys` 0.10.0

Wrapped on `Device`: `temperature`, `clock_info`, `max_clock_info`, `clock`, `supported_graphics_clocks`, `supported_memory_clocks`, `power_usage`, `power_management_limit_constraints`, `set_power_management_limit`, `set_gpu_locked_clocks`, `reset_gpu_locked_clocks`, `set_mem_locked_clocks`, `reset_mem_locked_clocks`, `fan_speed(idx)`, `num_fans`, `min_max_fan_speed`, `fan_control_policy`, `set_fan_control_policy`, **`set_fan_speed(idx, pct)`**, `set_default_fan_speed`, `utilization_rates`, `memory_info`, `pcie_throughput`, `performance_state`, `current_throttle_reasons`, `set_persistent`, `power_mizer_mode`/`set_power_mizer_mode`. **Not wrapped: clock offsets** — call `nvml_wrapper_sys::bindings::NvmlLib::nvmlDeviceSetClockOffsets/nvmlDeviceGetClockOffsets` (struct `nvmlClockOffset_v1_t { version, type_, pstate, clockOffsetMHz, minClockOffsetMHz, maxClockOffsetMHz }`), or the deprecated `nvmlDeviceSetGpcClkVfOffset`/`nvmlDeviceSetMemClkVfOffset` (all present in the sys bindings; obtain the raw handle via `Device::handle()` / `unsafe`). Feature `legacy-functions` exposes older `_v1` symbols. Reading from a non-root GUI is fine (NVML opens `/dev/nvidiactl` which is world-accessible); writes fail with `NVML_ERROR_NO_PERMISSION` unless root → do them in the helper.

---

## 6. amdgpu iGPU (Raphael)

Sysfs under `/sys/devices/pci0000:00/0000:00:08.1/0000:12:00.0/`: `power_dpm_force_performance_level` (`auto` observed; values `auto`, `low`, `high`, `manual`, `profile_standard`, `profile_min_sclk`, `profile_min_mclk`, `profile_peak` — doc in `amdgpu_pm.c`), `power_dpm_state`, `pp_dpm_sclk` (`0: 400Mhz`, `1: 600Mhz *`, `2: 2200Mhz`; in `manual` write index masks like `"1 2"`), `pp_dpm_mclk/fclk/socclk/vclk/dclk/dcefclk/pcie`, `pp_od_clk_voltage` (only `OD_SCLK 400..2200 MHz`, no voltage table; write `s 1 2000` then `c` to commit, `r` to reset), `pp_power_profile_mode`, `gpu_busy_percent` (0), `mem_info_vram_used/total`, `pp_force_state`. hwmon3: `temp1` edge 41 °C, `power1_input` PPT (see §4), `freq1` sclk, `in0` vddgfx, `in1` vddnb. All control files `0644 root`.

Verdict: not worth a control page. Nothing renders on it (Hyprland is on the 4090; card0 outputs are unused). Optionally expose a read-only tile and, if `amdgpu` still burns power, force `low`/`profile_min_sclk` in a "gaming" profile. `power-profiles-daemon` also writes `power_dpm_force_performance_level` (LACT documents the conflict and offers `--block-action=amdgpu_dpm`).

---

## 7. Privilege model and daemon integration

### 7.1 What needs root on this box

Writes to `nct6775` pwm/curve files, `rog_ryujin` pwm, cpufreq/EPP/boost/SMT, `amd_pstate/status`, `powercap` reads, amdgpu DPM files, all NVML setters, `ryzen_smu`. Unprivileged with udev ACLs: hidraw for Ryujin/Lian Li/Aura (present), i2c for DRAM/GPU RGB (present), hidraw12 + usb interface for the OLED (rule missing), reading every hwmon/cpufreq value, NVML reads, `/proc/stat`, Hyprland IPC, GameMode D-Bus, OpenRGB/CoolerControl/LACT sockets.

### 7.2 Patterns in the field

| project | privilege model | client API |
|---|---|---|
| **asusd** (asusctl) | root systemd `Type=dbus` service `xyz.ljones.Asusd` on the system bus, hardened unit (`ProtectSystem=strict`, `ReadWritePaths=/sys/`, `NoNewPrivileges`), authorization by **D-Bus policy file** (`/usr/share/dbus-1/system.d/asusd.conf` allows groups `users/wheel/sudo/adm`), no polkit. Laptop-only udev trigger (`DRIVER=="asus-nb-wmi"`), would not even start on this desktop. | zbus-generated proxies (`rog-dbus` crate in the asusctl repo) |
| **fancontrol** (lm_sensors) | root bash loop reading `/etc/fancontrol`; no API | none |
| **CoolerControl 5.0** | root `coolercontrold.service` (Rust); REST + HTTPS on `127.0.0.1:11987`, gRPC on `11988`; auth by its own admin password (`CCAdmin`, must be changed on first use; `sudo coolercontrold --reset-password`) or **bearer tokens `cc_<uuid>`** with read-only/write scopes created via `POST /tokens`; config `/etc/coolercontrol/config.toml`, state `/var/lib/coolercontrol`. Embeds liquidctl (Python child process) for HID devices, NVML (+ nvapi) for NVIDIA, sysfs for hwmon/CPU/amdgpu. | REST/SSE, gRPC (`/proto/coolercontrol`), plugins (JS UI + data proxy) |
| **LACT** | root `lactd.service`; newline-delimited JSON over `/run/lactd.sock` (group `wheel` by default via `daemon.admin_group` in `/etc/lact/config.yaml`), optional unauthenticated TCP; every config change must be confirmed (`confirm_pending_config`) within 5 s or it reverts. | `lact-client`/`lact-schema` crates inside the repo (not on crates.io) |
| **power-profiles-daemon** | root daemon, D-Bus `org.freedesktop.UPower.PowerProfiles`, polkit for `SetProfile`? (writes allowed to active session users) | zbus |

### 7.3 Recommended design for our helper

A small Rust root daemon (`omaasusd`) on the **system bus** (`zbus 5.19`, name e.g. `dev.omaasus.Helper`), systemd unit hardened like asusd (`ProtectSystem=strict`, `ReadWritePaths=/sys/class/hwmon /sys/devices/system/cpu /sys/class/powercap /sys/bus/pci/devices`, `DeviceAllow=/dev/nvidiactl /dev/nvidia0 /dev/hidraw*`, `NoNewPrivileges`, `CapabilityBoundingSet=` empty except what NVML needs), D-Bus policy allowing the `wheel` group to call it, and **polkit** checks per method group via `zbus_polkit 5.1.0` (`AuthorityProxy::check_authorization(subject-from-sender, "dev.omaasus.fan-control", details, AllowUserInteraction, "")`) with actions `dev.omaasus.{fan,cpu,gpu,display}` set to `auth_admin_keep` for session-local active users (or `yes` if you want no prompts). Every setter must be idempotent, validated, and have a "restore defaults" path that the daemon runs on shutdown/SIGTERM (write back saved `pwm_enable`, `nvmlDeviceSetDefaultFanSpeed_v2`, reset locked clocks, restore EPP). Prefer this over `pkexec`-launched one-shot helpers (no state, prompt per action, no crash-time restore) and never setuid. udev: ship `70-omaasus.rules` adding `uaccess` for `0b05:1a21` (hidraw and usb) so the display drivers can run unprivileged in the GUI process.

### 7.4 CoolerControl 5.0 REST API (from repo `openapi/openapi.json`, title "CoolerControl Daemon API 5.0.0")

Base `http://localhost:11987` (or https). Auth: `POST /login` with HTTP Basic `CCAdmin:<password>` → session cookie; or header `Authorization: Bearer cc_...` (create with `POST /tokens {"label","write_access":bool,"expires_at"?}` → `{id,token,...}`; `GET /tokens`, `DELETE /tokens/{id}`); `POST /verify-session`, `POST /logout`, `POST /set-passwd`. Unauthenticated: `GET /handshake`, `GET /health` (observed 401 on the running 4.3.1 build — behaviour differs by version). Rate: polling ≤ 1/s; use SSE.

Core endpoints:
* `GET /devices` → `{devices: [DeviceDto{uid, name, d_type: CPU|GPU|Liquidctl|Hwmon|CustomSensors|ServicePlugin, type_index, info: DeviceInfo{channels{name → ChannelInfo{label, speed_options{min_duty,max_duty,fixed_enabled,extension}, lighting_modes[], lcd_modes[], lcd_info}}, temps{name → TempInfo}, driver_info{drv_type,name,version,locations[]}, temp_min, temp_max, profile_min_length, profile_max_length, amd_gpu_overdrive?, thinkpad_fan_control?}}]}`. Device UIDs are sha256 strings (this host: `nct6799` `00a4da18…`, `rog_ryujin` `f23cf56d…`, `Lian Li Uni SL` `c6d76dc7…`, NVIDIA `7e51251c…`, CPU `1ec0a6b4…`, `asusec` `30b0270e…`, `Custom Sensors` `19e098e3…`).
* `GET /status` (latest) / `POST /status {"all":bool,"since":date-time}` → `{devices:[{uid,d_type,type_index,status_history:[{timestamp, temps:[{name,temp}], channels:[{name, rpm?, duty?, freq?, watts?, pwm_mode?}]}]}]}`; `GET /status/{uid}`, `GET /status/{uid}/channels/{ch}`; `GET /sse?events=status,health,logs,modes,alerts,notifications,system` streams frames `event: status\ndata: StatusResponse` (this is what the GUI should consume).
* Control: `PUT /devices/{uid}/settings/{ch}/manual {"speed_fixed":0..100}`; `PUT .../profile {"profile_uid"}`; `PUT .../lighting {LightingSettings{mode, colors:[[r,g,b]], speed?, backward?}}`; `PUT .../lcd {LcdSettings{mode: none|liquid|image|temp|carousel, brightness?, orientation?, image_file_processed?, temp_source?}}` (+ `/lcd/images` GET/PUT/POST, `/lcd/shutdown-image`); `PUT .../reset` (hand back to firmware = restores saved `pwm_enable`); `GET /devices/{uid}/settings`.
* Profiles: `GET/PUT/POST /profiles`, `DELETE /profiles/{uid}`, `POST /profiles/order`, `POST /profiles/generate` (5.0 wizard). `Profile{uid,name,function_uid, p_type: Default|Fixed{speed_fixed}|Graph{speed_profile:[[temp,duty]...], temp_source{device_uid,temp_name}, temp_min, temp_max}|Mix{member_profile_uids, mix_function_type}|Overlay…}`.
* Functions (response shaping): `GET/PUT/POST /functions`, `Function{uid,name,f_type: Identity|Standard{deviance, response_delay, sample_window, only_downward...}|ExponentialMovingAvg..., duty_minimum, duty_maximum, step_size_min/max_decreasing, threshold_hopping, bypass_min_at_extremes}`.
* Custom sensors: `/custom-sensors` (`cs_type: Mix{mix_function, sources}|File{file_path}|Offset|TimeAverage|ExponentialMovingAvg`) — e.g. this host's `cpu_gpu_max`.
* Modes (= whole-system presets: `ModeDto{uid,name,device_settings:[[device_uid,[Setting...]]]}`): `GET/PUT/POST /modes`, `POST /modes-active/{uid}` **(activate a mode — the hook for our gaming profile)**, `GET /modes-active`, `PUT /modes/{uid}/settings`, `/modes/{uid}/duplicate`, `/modes/order`; `GET /power-profiles` → `{available[], active, modes{profile→mode_uid}}`, `PUT /power-profiles/modes` (auto-activate a Mode when power-profiles-daemon switches).
* Misc: `/alerts`, `/settings` (daemon settings, PATCH), `/settings/devices/{uid}` (disable/blacklist channels), `/settings/overrides` (names), `/settings/ui`, `/plugins/...`, `/calibrations/...` (5.0 fan calibration), `/detect`, `/hardware-report`, `/stress-test/{cpu,gpu,ram,drive}`, `/metrics` (Prometheus), `POST /shutdown`, `POST /amd-gpu-overdrive`.
* `Setting` (persisted per channel): exactly one of `reset_to_default`, `speed_fixed`, `profile_uid`, `lighting`, `lcd` plus `channel_name`.

### 7.5 LACT JSON API (`docs/API.md`, `lact-schema/src/request.rs`)

Socket `/run/lactd.sock`; one JSON object per line: `{"command": "...", "args": {...}}` → `{"status":"ok|error","data":...}`. Commands: `ping`, `list_devices` (→ `[{id:"10DE:2684-1043:889B-0000:01:00.0", name}]`), `system_info`, `device_info{id}`, `device_api_info`, `device_stats{id}` (temps, clocks, power, fan, throttling), `device_clocks_info`, `device_power_profile_modes`, `displays_info`, `get_gpu_config{id}` / `set_gpu_config{id, config}` (**preferred**; config = CONFIG.md shape in JSON: `fan_control_enabled`, `fan_control_settings{mode: static|curve, static_speed, temperature_key, interval_ms, curve{"40":0.3,...}, spindown_delay_ms, change_threshold, auto_threshold}`, `power_cap`, `performance_level`, `power_profile_mode_index`, `gpu_clock_offsets{pstate→MHz}`, `mem_clock_offsets`, `nvidia_gpu_vf_curve`, `voltage_offset`, `nvidia_thermal_options`, `power_states`), then **`confirm_pending_config{"command":"confirm"|"revert"}`** within 5 s; deprecated single setters `set_fan_control`, `set_power_cap`, `set_performance_level`, `set_clocks_value{command: MaxCoreClock|GpuClockOffset(pstate)|MemClockOffset(pstate)|GpuVfCurveClock(i)|...}`, `batch_set_clocks_value`, `set_power_profile_mode`, `get_power_states`/`set_enabled_power_states`, `reset_pmfw`; profiles: `list_profiles`, `get_profile`, `set_profile{name}`, `create_profile`, `delete_profile`, `move_profile`, `hold_profile`/`release_profile`, `set_profile_rule{rule: {type: process{name,args?}|gamemode{name?}}}`, `evaluate_profile_rule`; `enable_overdrive`/`disable_overdrive` (AMD), `vbios_dump`, `process_list`, `detach_gpu`/`reattach_gpu`, `generate_snapshot`, `rest_config`. NVIDIA support needs the proprietary driver + CUDA libs; offsets need driver ≥ 555 (565+ recommended); fan curves are software loops using NVML; locked clocks Volta+/Ampere+.

### 7.6 Assessment: drive CoolerControl/LACT, or ship our own helper?

**Recommendation: CoolerControl as the fan engine + our own small privileged helper for everything else; treat LACT as optional, not a dependency.**

Why CoolerControl for fans:
* It is already installed, enabled, and configured on this machine with the exact devices we care about (`nct6799`, `rog_ryujin` hwmon, Lian Li via liquidctl, NVIDIA fans via NVML, `asusec`/`k10temp` temps, custom sensor, three graph profiles). It handles the messy parts correctly: saving/restoring `pwm_enable`, per-device min/max duty, hysteresis functions, mixing sources, calibration, resume-from-sleep, and it exposes SSE so we need no polling loop. Reimplementing this is weeks of work with real risk (a dead fan loop on a 4090/7950X).
* Its API covers what a control-center UI needs: apply manual duty, assign profile, create/edit profiles and functions, activate **Modes** (perfect for "Gaming"/"Quiet" presets), power-profile mapping. Auth is a stored write token (`cc_…`) in our config; the GUI runs unprivileged.
* Costs: depends on a Python liquidctl child for Lian Li/Aura; the daemon's default password must be initialized once (UI or `--reset-password`); CoolerControl will not edit Smart Fan IV firmware curves (only manual mode) — if we want BIOS-resident curves, our helper writes `auto_point*` directly and we must tell CoolerControl to leave those channels alone (`/settings/devices/{uid}` disable channel).

Why not LACT as a required daemon: it overlaps CoolerControl on GPU fans (two daemons fighting NVML fan policy), adds a second root socket, and everything else it offers for NVIDIA is ~10 NVML calls (`SetPowerManagementLimit`, `SetGpuLockedClocks`, `SetClockOffsets`, `SetPersistenceMode`) that our helper can make with `nvml-wrapper` + `nvml-wrapper-sys` and guard with the same "apply → confirm within N s or revert" pattern LACT uses. If the user already runs LACT, detect `/run/lactd.sock` and prefer sending `set_gpu_config` there (and keep `fan_control_enabled:false` so CoolerControl keeps the fans).

What only our helper can do (hence it must exist regardless): CPU governor/EPP/boost/SMT/min-max freq, RAPL energy, amdgpu DPM level, `nct6775` Smart Fan IV curve editing and `temp_sel`, `ryzen_smu` PBO/CO (optional, gated), NVIDIA power/clock tuning when LACT is absent, and shutdown-time restore of all of it. Keep the helper's surface small and typed (D-Bus methods with enums, not "write this sysfs path").

---

## 8. Motherboard-specific: Armoury Crate features vs Linux

| Armoury Crate feature (X670E Extreme) | What it is | Linux path |
|---|---|---|
| Fan Xpert 4 / Q-Fan | per-header curves, DC/PWM, spin-up, source selection, "Fan tuning" (min-RPM calibration) | `nct6775` Smart Fan IV (`auto_point*`, `temp_sel`, `pwm_mode`, `step_*`, `floor/start`) — equivalent; calibration = CoolerControl 5.0 `/calibrations` |
| AI Cooling | ML-tuned curve based on CPU temp + load | no equivalent; approximate with CoolerControl Functions or our own PID |
| AI Overclocking / PBO / Curve Optimizer / DOCP-EXPO | SMU/BIOS tuning | BIOS only, or `ryzen_smu` RSMU commands (volatile, unsupported) |
| Aura Sync | RGB on board, headers, RAM, GPU, AIO, hub | OpenRGB (all detected here) |
| LiveDash OLED | 2" OLED: POST codes, hardware monitor, custom text/GIF | `0b05:1a21` HID/bulk protocol (§2.3), no existing Linux tool |
| AniMe Matrix (I/O shroud LED matrix) | preset animations, custom frames | same chip, `EC 42 01` + `EC 7F 04` frames, LUT unmapped for this board |
| Ryujin II LCD | AIO 3.5" LCD image/GIF/hardware monitor | `0b05:1a60` bulk protocol (§2.4), unverified |
| Armoury "Device" tab: pump/fan of Ryujin | duty control | `rog_ryujin` hwmon |
| Sonic Studio, GameFirst | audio/network | out of scope |

`asus-armoury` (`platform/x86`, merged for 6.19, LWN 1042745) is a firmware-attributes driver for **laptops and handhelds** (`ppt_pl1_spl`, `ppt_pl2_sppt`, `ppt_fppt`, `ppt_apu_sppt`, `ppt_platform_sppt`, `nv_dynamic_boost`, `nv_temp_target`, `dgpu_base_tgp`, `charge_mode`, `boot_sound`, `mcu_powersave`, `panel_od`, `mini_led_mode`, `egpu_enable`, `dgpu_disable`, `gpu_mux_mode`, `apu_mem`, `cores_performance/efficiency`) under `/sys/class/firmware-attributes/asus-armoury/attributes/<name>/current_value`; on this desktop it registers nothing but `pending_reboot` — the WMI methods do not exist on the AM5 desktop BIOS. `asus_wmi` on this board binds as `eeepc-wmi` (`/sys/devices/platform/eeepc-wmi/`: only `cpufv`, an empty hwmon `asus`) — no useful desktop knobs. `asus-ec-sensors` is the only desktop-specific ASUS driver that matters.

---

## 9. Gaming automation

### 9.1 Signals to detect "a game is running"

* **GameMode** (Feral; package `gamemode`): session bus name `com.feralinteractive.GameMode`, object `/com/feralinteractive/GameMode`, interface `com.feralinteractive.GameMode`: methods `RegisterGame(i pid)→i`, `UnregisterGame(i)→i`, `QueryStatus(i)→i` (0 inactive, 1 active, 2 registered), `RegisterGameByPID(ii)`, `UnregisterGameByPID(ii)`, `QueryStatusByPID(ii)`, `*ByPIDFd(hh)`, `RefreshConfig()→i`, `ListGames()→a(io)`; property `ClientCount (i)` with change notifications; signals `GameRegistered(i pid, o path)`, `GameUnregistered(i, o)`; game objects `/com/feralinteractive/GameMode/Games/<pid>` implement `com.feralinteractive.GameMode.Game` with properties `ProcessId (i)`, `Executable (s)`, `Requester (i)`, `Timestamp (t)`. All methods are unprivileged. Watch `ClientCount`/`GameRegistered` from the GUI (zbus). `gamemode.ini` `[custom] start=/end=` scripts can also poke our GUI/daemon; `[gpu]` NVIDIA offsets there use `nvidia-settings` (X-only) — leave them off and let our helper do NVML. Steam users add `gamemoderun %command%`; LACT and CoolerControl-modes cannot see this without gamemode.
* **Hyprland IPC**: `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket2.sock` streams `EVENT>>DATA\n`: `fullscreen>>0|1`, `activewindow>>CLASS,TITLE`, `activewindowv2>>ADDRESS`, `openwindow>>ADDRESS,WORKSPACE,CLASS,TITLE`, `closewindow>>ADDRESS`, `windowtitle`, `focusedmon`, `workspace(v2)`, `monitoradded/removed`, ... Query `.socket.sock` with `j/clients` / `j/activewindow` (open, write, read, close — the socket is synchronous and a held-open connection freezes Hyprland for 5 s). `j/activewindow` fields (observed): `address, class, title, initialClass, initialTitle, pid, fullscreen (0/1/2), fullscreenClient, xwayland, tags, ...`. Heuristic: `fullscreen == 1` + `class` matching `steam_app_<id>` / `gamescope` / known launchers, or `pid`'s environ containing `SteamGameId`/`SteamAppId`. Crate `hyprland 0.4.0-beta.3` wraps both sockets.
* **Steam**: native Steam installed. Game processes carry env `SteamAppId`, `SteamGameId`, `SteamOverlayGameId`; the launcher runs `reaper SteamLaunch AppId=<id> -- <cmd>`; Proton games have `WINEDLLOVERRIDES`/`PROTON_*`. Poll `/proc/*/environ` (readable for same-uid processes) or watch `openwindow` events and inspect the pid. Steam also registers with GameMode only when the user adds `gamemoderun`.
* **LACT** already implements process-name and gamemode rules (`set_profile_rule`), and **CoolerControl** switches Modes on power-profile changes; both are useful reference points but neither knows about Hyprland fullscreen.

### 9.2 Applying a profile

Gaming profile = (a) `POST /modes-active/{gaming_mode_uid}` on CoolerControl (fans/pump), (b) helper: EPP `performance` (or governor `performance`), boost on, NVIDIA power limit/offsets/persistence, optionally amdgpu `low`, optionally `scx_loader.SwitchScheduler("scx_lavd")`, ppd `performance`; (c) OpenRGB profile via SDK (`PROFILEMANAGER_LOAD_PROFILE 152`). Revert on `GameUnregistered`/`fullscreen>>0`/process exit with a debounce (5–10 s) to avoid flapping on alt-tab. Existing Linux examples: gamemode itself (governor + ppd profile + iGPU heuristics + renice + custom scripts), LACT auto-profiles, CoolerControl power-profile mapping, `ananicy-cpp` rules, Omarchy's `gamemode` hooks (none installed here).

---

## 10. Rust crates (versions as of 2026-09-09 on crates.io)

| crate | version | use |
|---|---|---|
| `zbus` | 5.19.0 | D-Bus helper daemon + GameMode/ppd/scx clients |
| `zbus_polkit` | 5.1.0 | polkit `CheckAuthorization` in the helper |
| `zbus_systemd` | 0.26100.0 | optional systemd unit control |
| `nvml-wrapper` / `nvml-wrapper-sys` | 0.13.0 / 0.10.0 | NVIDIA reads and setters (offsets via sys) |
| `hidapi` | 2.6.7 | Ryujin/Aura/Lian Li/OLED HID reports (`linux-native` feature avoids libusb) |
| `rusb` | current | bulk endpoint of `1a21`/`1a60` for images |
| `openrgb2` | 0.4.1 | OpenRGB SDK client (protocol 6) |
| `hyprland` | 0.4.0-beta.3 | Hyprland IPC events/queries |
| `sysinfo` / `procfs` | 0.39.6 / 0.18.0 | `/proc/stat`, processes, environ |
| `inotify` / `rustix` | 0.11.5 / 1.1.4 | sysfs change watching, low-level I/O |
| `tokio` | 1.53.1 | runtime |
| `reqwest` + `eventsource-stream` or `reqwest-eventsource` | current | CoolerControl REST + SSE |
| (in-repo) `lact-client`, `lact-schema` | git | optional LACT integration |

---

## 11. Sources

* nct6775 kernel doc: https://www.kernel.org/doc/html/latest/hwmon/nct6775.html ; source `drivers/hwmon/nct6775-core.c` (raw.githubusercontent.com/torvalds/linux/master/…)
* asus_rog_ryujin: https://docs.kernel.org/hwmon/asus_rog_ryujin.html ; `drivers/hwmon/asus_rog_ryujin.c` ; Phoronix https://www.phoronix.com/news/ASUS-ROG-RYUJIN-II-360-Linux
* liquidctl 1.16 drivers (local `/usr/lib/python3.14/site-packages/liquidctl/driver/{asus_ryujin,lianli_uni,aura_led}.py`); https://github.com/liquidctl/liquidctl/blob/main/docs/asus-ryujin-guide.md ; issue #677
* uni-sync: https://github.com/EightB1ts/uni-sync
* OpenRGB: `NetworkProtocol.h`, `Controllers/AsusAuraUSBController/AsusAuraUSBController/*`, `Controllers/LianLiController/LianLiUniHubSLController/*` (gitlab.com/CalcProgrammer1/OpenRGB); SDK doc https://openrgb.org/sdk.html ; issue #690 (LCD feature request); crate https://crates.io/crates/openrgb2
* Polylux (LiveDash/Ryujin LCD/AniMe protocol): https://github.com/vladulus/polylux (`docs/PROJECT_STATE.md`, `polylux/drivers/`)
* ASUS: LiveDash intro https://rog.asus.com/support/faq/1038402/ ; Ryujin II LCD setup https://www.asus.com/support/faq/1048625/ ; forum https://rog-forum.asus.com/t5/amd-600-series/rog-crosshair-x670e-extreme-anime-matrix-led-display-and/td-p/920658
* amd-pstate: https://docs.kernel.org/admin-guide/pm/amd-pstate.html
* RAPL permissions: https://github.com/powercap/powercap/issues/7 , https://bbs.archlinux.org/viewtopic.php?id=307345
* ryzen_smu: https://github.com/amkillam/ryzen_smu , https://github.com/leogx9r/ryzen_smu , https://github.com/mann1x/ryzen_monitor_ng , https://github.com/FrozenGalaxy/ryzen_smu_hwmon ; Zen4 SMU IDs: https://github.com/irusanov/ZenStates-Core/blob/master/Hardware/Smu/Settings/Zen4Settings.cs
* NVML: https://docs.nvidia.com/deploy/nvml-api/group__nvmlDeviceCommands.html , https://docs.nvidia.com/deploy/nvml-api/group__nvmlDeviceQueries.html ; nvml-wrapper https://github.com/rust-nvml/nvml-wrapper (CHANGELOG, `nvml-wrapper/src/device.rs`, `nvml-wrapper-sys/src/bindings.rs`); NVIDIA forum threads on NVML fan control under Wayland (https://forums.developer.nvidia.com/t/nvidia-fan-control-wayland/304824 , …/permission-for-nvml-or-nvidia-settings-on-wayland-without-root/322665)
* amdgpu: `drivers/gpu/drm/amd/pm/amdgpu_pm.c` DOC blocks; https://docs.kernel.org/gpu/amdgpu/thermal.html
* CoolerControl: https://docs.coolercontrol.org/ (daemon/access-protection, development/rest, development/prometheus, automation/plugins, daemon/config-files, whats-new/5.0), repo `openapi/openapi.json` and `coolercontrold/daemon/src/repositories/hwmon/fans.rs` (gitlab.com/coolercontrol/coolercontrol), release https://gitlab.com/coolercontrol/coolercontrol/-/releases/5.0.0
* LACT: https://github.com/ilya-zlobintsev/LACT (README, docs/API.md, docs/CONFIG.md, lact-schema/src/request.rs, wiki Hardware-Support)
* asusctl: https://github.com/flukejones/asusctl (`data/asusd.conf`, `data/asusd.rules`, `data/asusd.service`)
* asus-armoury: https://lwn.net/Articles/1042745/ , https://www.phoronix.com/news/ASUS-Armoury-More-Hardware
* GameMode D-Bus: https://github.com/FeralInteractive/gamemode/blob/master/daemon/gamemode-dbus.c , `example/gamemode.ini`
* Hyprland IPC: https://wiki.hypr.land/IPC/ ; crate https://crates.io/crates/hyprland
