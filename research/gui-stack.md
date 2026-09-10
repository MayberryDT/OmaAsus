# GUI stack evaluation for the OmaAsus control center

Date: 2026-09-09. Target: Arch Linux, Hyprland 0.56.2 (Wayland), GTK 4.22.4,
libadwaita 1.9.3, gtk4-layer-shell 1.3.0, Rust 1.98.0 / cargo 1.98.0,
NVIDIA + AMD GPUs (wgpu-capable), 32-thread Ryzen 9 7950X.

All versions below were read from `crates.io/api/v1/crates/<name>` on
2026-09-09; the build numbers are from real `cargo build` runs on this machine.

Requirements being judged: (a) modern / elegant / "high-fashion" look,
(b) toggleable layer-shell overlay on Hyprland with keyboard input **and** a
normal window from the same binary, (c) fan curves, live line charts, gauges,
curve editor, (d) animation, (e) maintainability, (f) build reliability with
this toolchain.

---

## TL;DR

**Recommended: `iced 0.14` + `iced_exwlshell 0.20` (wgpu renderer).**
Runner-up / fallback: `gtk4 0.11` + `libadwaita 0.9` + `gtk4-layer-shell 0.8`.

- iced gives full pixel control (own renderer, gradients, shadows, custom wgpu
  shaders, Canvas with interactive `Program` for a drag-to-edit fan curve),
  built-in animation (`iced::animation`, built on `lilt`), and — verified
  here by compiling and running a skeleton — one process that starts headless,
  opens a keyboard-interactive layer-shell overlay on demand (with native
  Hyprland backdrop blur via `ext-background-effect-v1`), and also opens a
  normal XDG window.
- One build-reliability caveat, found today: `iced_exwlshell 0.20.0` transitively
  depends on the pre-release `winit-core 0.31.0-beta.x`; `beta.3` (published
  2026-09-04) breaks `iced_exdevtools 0.20.0`. Pinning
  `winit-core = "=0.31.0-beta.2"` / `winit-common = "=0.31.0-beta.2"` fixes it
  and must be in Cargo.toml + a committed Cargo.lock.
- GTK4/adw is the "boring, bulletproof" alternative: 22 s clean build,
  CSS with `backdrop-filter: blur()` (new in 4.22), conic gradients,
  `@keyframes`, `color-mix()/oklch`, variable fonts — but the look is pulled
  toward GNOME, Rust ergonomics are GObject-shaped, and charts are Cairo.

---

## 1. iced

| crate | version (crates.io) | notes |
|---|---|---|
| `iced` | **0.14.0** (2025-12-07) | last experimental release before 1.0; Rust 2024 edition; wgpu 27; cosmic-text 0.15 |
| `iced_exwlshell` | **0.20.0** (2026-08-29) | layer-shell + session-lock + xdg windows for iced 0.14 |
| `iced_layershell` | 0.19.1 (2026-07-12) | **deprecated as of 0.20** — merged into `iced_exwlshell` |
| `iced_aw` | **0.14.1** (2026-04-27) | targets iced 0.14; number_input, tab_bar, menu, color_picker, slide_bar, spinner, badge, card, selection_list, sidebar, typed_input, wrap, etc. |
| `iced_anim` | 0.3.1 (2026-01-01) | targets iced_core 0.14 / iced_widget 0.14.2 |
| `lilt` | **0.8.2** (2026-07-28) | framework-agnostic; iced_core 0.14 depends on it |
| `iced_fonts` | 0.3.0 | bundle fonts via feature flags |
| `plotters-iced` | 0.11.0 (2024-09) | stale, not for 0.14 — skip |
| `iced_plot` | 0.5.0 (2026-06) | tiny (901 downloads) — not relied upon |

Resolved lockfile on this machine: iced 0.14.0, iced_widget 0.14.2,
iced_wgpu 0.14.0, iced_winit 0.14.0 (winit 0.30.13), wgpu 27.0.1,
exwlshellev 0.20.0, iced_aw 0.14.1, iced_anim 0.3.1, lilt 0.8.2 — all
coexist in one graph, 445 packages.

**Theming.** Every widget takes a style closure
(`button::Style`, `container::Style`, ...) with `Background::Color` or
`Background::Gradient(gradient::Linear)`, `Border { radius, width, color }`,
`Shadow { color, offset, blur_radius }`, `text_color`. 0.14 overhauled palette
generation with **Oklch** and added `theme::Style` (app background — set to
`Color::TRANSPARENT` for glass overlays), system-theme reactions and a
"warning" palette slot. There is no CSS; the look is entirely yours, which is
what "high-fashion" needs — nothing to fight.

**Canvas.** `iced::widget::canvas` (feature `canvas`): `Frame` with paths, arcs,
bezier, strokes with round caps, gradients, fills, text; `canvas::Program`
has `update()` for mouse/keyboard events + `mouse_interaction()`, so a
draggable fan-curve editor (hit-test control points, return
`Action::publish(Message)`), live sparklines and arc gauges are all first-class.
Geometry is cached via `canvas::Cache`. The skeleton below draws an arc gauge.

**Custom shaders.** `iced::widget::shader` (feature `wgpu`): implement
`shader::Program` (state + `update`/`draw`) and `shader::Primitive`
(`prepare`/`render` with your own `wgpu::RenderPipeline`), storage for
pipelines via `shader::Storage`. Suitable for GPU gauges, glow, particle
accents. 0.14 also merged layers better and moved quads to a single SDF.

**Text/fonts.** cosmic-text; `text::Shaping::Auto` (new in 0.14: basic for
ASCII, advanced otherwise), justified alignment, rich text, markdown, IME.
Load app fonts with `.font(include_bytes!("...ttf"))` on the builder;
`default_font(Font::with_name("Inter"))`.

**Animation.** Built in: `iced::animation::Animation<T>` with
`.quick()/.slow()/.duration()/.delay()/.repeat()/.repeat_forever()/.auto_reverse()`,
`.go(new_state, now)`, `.is_animating(now)`, `.interpolate(a, b, now)`,
`Easing::{Linear, EaseInOut, EaseOutCubic, EaseOutExpo, EaseOutBack,
EaseOutElastic, EaseOutBounce, ..., Custom}` (31 variants; re-exported from
`lilt`). Drive redraws with `window::frames()` subscription or
`Task`/`request_redraw`. `iced_anim` adds an `Animated<T>`-in-view widget and
derive macro if you prefer declarative transitions. `lilt` alone is usable if
you want interruptible spring-like transitions in `update`.

**Layer-shell on Hyprland (`iced_exwlshell`).** Uses waycrate's
`exwlshellev` (winit-like event loop on `smithay-client-toolkit 0.21`), *not*
winit, so it is native `zwlr_layer_shell_v1`. Verified in source (0.20.0):

- `Settings { layer_settings: LayerShellSettings { anchor, layer,
  exclusive_zone, size, margin, keyboard_interactivity, start_mode,
  blur_option, events_transparent }, fonts, default_font, default_text_size,
  antialiasing, virtual_keyboard_support, with_connection, shell_broadcast,
  keep_compositor_alive }`
- `StartMode::{Active, Background, AllScreens, TargetScreen(String), TargetOutput(..)}`
  — `Background` starts the daemon with *no* surface, which is exactly the
  "hotkey toggles an overlay" model.
- `KeyboardInteractivity::{None, Exclusive, OnDemand}` (sctk enum);
  `OnDemand` is right for a quick-settings panel (focus on click, Esc releases).
- `BlurOption::{None, FullRegion, Region(Vec<BlurRegion>)}` — implemented with
  `ext_background_effect_v1`; **Hyprland 0.56.2 advertises
  `ext_background_effect_manager_v1`** (checked in the binary), so the overlay
  gets compositor backdrop blur without a `layerrule`. (You can still add
  `layerrule = blur, omaasus-overlay` + `ignorezero` in Hyprland config.)
- `#[to_exwlshell_message(multi)]` injects variants on your `Message`:
  `NewLayerShell{settings, id}`, `NewBaseWindow{settings, id}` (normal XDG
  toplevel), `NewPopUp`, `PopUpReposition`, `NewMenu`, `NewInputPanel`,
  `RemoveWindow(id)`, `LayoutChange{id, anchor, size}`, `LayerChange`,
  `MarginChange`, `ExclusiveZoneChange`, `KeyboardInteractivityChange`,
  `BlurOptionChange`, `SetInputRegion`, `VirtualKeyboardPressed`,
  `ForgetLastOutput`, and helper constructors
  `Message::layershell_open(NewLayerShellSettings) -> (Id, Task)`,
  `Message::base_window_open(IcedXdgWindowSettings { size, client_side_decorations })`,
  `Message::popup_open(..)`.
- `daemon(boot, namespace, update, view)` builder with `.theme() .style()
  .subscription() .title() .font() .default_font() .on_new_shell() .redraw_scope()
  .settings()`; `view(&self, id: window::Id)` per surface. Single-surface
  `iced_exwlshell::layershell::application(..)` also exists.
- Upstream examples: `application_launcher`, `bottom_panel`, `counter_multi`,
  `multi_window`, `zbus_invoked_widget` (D-Bus-triggered widget — the pattern
  for a hotkey toggle), `input_regions`, `workspace_bar`, `redraw`.

**Same app as a normal window: yes.** The daemon can open an XDG toplevel
(`NewBaseWindow`) alongside layer surfaces in the same process — the skeleton
below does both. Alternatively ship one binary with two entry paths
(`iced::application` via winit for `--window`, `iced_exwlshell::daemon` for the
overlay); the widget/view code is shared either way.

**Hotkey.** Hyprland `bind = SUPER, F12, exec, omaasus toggle` → the CLI sends a
D-Bus method (`zbus`) or a Unix-socket message to the running daemon, which
turns it into `Message::ToggleOverlay` through a `Subscription`. `ashpd`
0.13.13 `GlobalShortcuts` portal (xdg-desktop-portal-hyprland 1.4.1 is
installed) is the portable option.

**Build on this machine (skeleton in Appendix A):**
- true clean debug build 17.0 s; incremental rebuild 0.8 s; clean release 21.8 s
- debug binary 379 MB (wgpu/naga debug info), release binary 21.6 MB
- smoke-run under Hyprland: starts in `StartMode::Background`, no panic.
- **Breakage found:** `iced_exdevtools 0.20.0` → `winit-core ^0.31.0-beta.2`
  resolved to `0.31.0-beta.3` (published 2026-09-04) which added a
  `NativeKeyCode` variant → `error[E0004]: non-exhaustive patterns` in
  `iced_exdevtools/src/keymap.rs:575`. `waycrate_xkbkeycode` also depends on
  `winit-core`/`winit-common` beta. Fix: pin both to `=0.31.0-beta.2` (done in
  Appendix A). No upstream issue/commit for it yet as of today. Expect this
  class of breakage until winit 0.31 ships; a committed `Cargo.lock` is
  mandatory.

**Risks.** iced 1.0 will change APIs (0.14 is the last 0.x); `iced_exwlshell`
tracks iced releases with some lag (0.19 → 0.20 rename churn just happened);
smaller ecosystem than GTK; no accessibility tree yet.

## 2. egui / eframe

`egui` / `eframe` / `egui_extras` / `egui-wgpu` / `egui-winit` **0.36.2**
(2026-09-08), `egui_plot` **0.37.0** (2026-08-05, depends on egui 0.36).

- Styling: `Visuals` (36 fields: widget fills/strokes per state, window/panel
  fill, `window_shadow`, `popup_shadow`, corner radius, selection, text
  colors). No gradients, no blur, no background images; gradients only via
  hand-built `Mesh` with per-vertex colors; fonts via `Context::set_fonts`.
  Immediate mode makes per-frame animation trivial (`ctx.animate_value_with_time`)
  but the result reliably looks like "an egui app". Not a high-fashion tool.
- `egui_plot` 0.37 is excellent for live charts (lines, bars, hover, zoom).
- Layer-shell: **nothing exists.** eframe is winit-only (winit issue #2582
  still open); `NativeOptions::window_builder/event_loop_builder` cannot make a
  layer surface. No `egui-layershell`/`egui_sctk` crate on crates.io (checked
  nine names). You would hand-write `layershellev` + `egui-wgpu` + `egui-winit`
  input translation. `egui_overlay` 0.9 (2024) is a GLFW passthrough hack for
  X11/XWayland. Verdict: viable for a normal window only.

## 3. GTK4 + libadwaita (gtk4-rs)

| crate | version | notes |
|---|---|---|
| `gtk4` | **0.11.4** (2026-06-29) | features up to `v4_24`; use `v4_22` here; `blueprint` feature exists |
| `libadwaita` | **0.9.2** (2026-07-07) | features up to `v1_10`; use `v1_9` |
| `gtk4-layer-shell` | **0.8.1** (2026-08-10) | depends on gtk4 0.11 / glib 0.22; feature `v1_3` matches installed 1.3.0; MSRV 1.85 |
| `relm4` | 0.11.0 (2026-04-08) | Elm-style layer over gtk4-rs, optional |

**CSS power — tested against the installed GTK 4.22.4 `CssProvider`:**

```
linear-gradient    OK        color-mix(in oklch) OK      @keyframes  OK
conic-gradient     OK        custom props/var()  OK      transition  OK
filter: blur()+drop-shadow() OK   font-variation/feature-settings OK
backdrop-filter: blur(10px)  OK   (new: GTK 4.21.2 → 4.22; renders real backdrop blur)
transform          OK        box-shadow (multi, inset) OK
mask               ERROR (no such property)
@font-face         ERROR (unknown @-rule) → ship fonts via fontconfig/system, not CSS
```

Also in 4.22: media queries for colour-scheme/contrast/reduced-motion,
`light-dark()`, filter URLs, repeating conic gradients. GTK's `filter` is
CSS Filter Effects Level 1 (blur, brightness, contrast, drop-shadow,
grayscale, hue-rotate, invert, opacity, saturate, sepia). Custom fonts: load
with `fontconfig` app-font APIs or install under `~/.local/share/fonts`;
Pango picks them up by family name.

**Overlay.** `gtk4-layer-shell`: `win.init_layer_shell(); set_layer(Layer::Overlay);
set_anchor(Edge::Right, true); set_keyboard_mode(KeyboardMode::OnDemand);
set_margin(..); set_exclusive_zone(..); set_namespace("omaasus-overlay")`.
Same `adw::ApplicationWindow` type can be a layer surface or a normal window —
choose at construction (`gtk4_layer_shell::is_supported()`). Mature, widely
used on Hyprland (walker, waybar-likes, swaync). Hyprland backdrop blur via
`layerrule = blur, omaasus-overlay`.

**Charts.** `gtk::DrawingArea` + cairo (`cairo-rs` via gtk4), or a custom
widget using `Snapshot` (GPU render nodes: gradients, blur, shadows,
rounded clips). Curve editor = DrawingArea + `GestureDrag`. Fine but lower
level than iced's Canvas; no ready-made plot widget in Rust land for GTK4.

**Blueprint.** `.blp` → `.ui` via `blueprint-compiler` (system package, not
a crate) at build time; gtk4 crate's `blueprint` feature runs it. Nice for
static layouts; less useful for an app that is mostly custom-drawn.

**Animation.** `adw::TimedAnimation` / `adw::SpringAnimation` (with
`CallbackAnimationTarget`), plus CSS `transition` and `@keyframes`.

**Build on this machine (Appendix B):** clean debug 22.0 s, incremental 0.2 s,
clean release 28.5 s, debug binary 64 MB; smoke-run as a layer-shell overlay
on Hyprland OK.

**Downsides.** GObject ergonomics in Rust (`clone!`, `Rc<RefCell>`, subclass
boilerplate for custom widgets); libadwaita's strong GNOME identity — you
will override a lot of `.adw-*` styling to get away from it; coupling to
system library versions (Arch is fine, but `v4_22` must match the machine
that builds it).

## 4. Slint

`slint` **1.17.1** (2026-07-07; 1.17.0 2026-06-24). `.slint` markup with
gradients (`@linear-gradient`, `@radial-gradient(circle, ..)`,
`@conic-gradient`), per-corner radii, drop shadows (`drop-shadow-blur/-color/
-offset`, plus `-spread` and inner shadows on Skia only), `animate x { duration,
easing }` with a rich easing set, states/transitions, `Path` elements,
`@image-url`, `import "font.ttf"`. Renderers: femtovg (OpenGL), Skia
(`renderer-skia`, best effects), software. Very fashion-capable; the live
preview/editor is good.

**ROG Control Center** (asusctl 6.4.0 workspace) uses `slint ^1.17.1` with
`backend-winit-wayland` + `renderer-femtovg`, `zbus ^5.19`, `tokio ^1.53.1`.
That is a normal winit window — it does **not** do layer-shell.

**Layer-shell.** No official support (Slint discussion #1765: maintainers'
position is "use raw window handles"; last comment 2025-07/11 points to
third parties). Options: `layer-shika` 0.3.1 (2026-01-19, "not production
ready", **AGPL-3.0**, femtovg+EGL, slint-interpreter or compiled) and
`spell-framework` 1.0.6 (2026-07-29, custom `slint::platform::Platform`
backend for wlr-layer-shell). Both are one-person projects; running the same
component tree in a winit window and in a spell/shika surface means two
platform backends in one binary — possible but untested territory.

**Charts.** `Path` with commands or `Image` from a `SharedPixelBuffer`
rendered with `plotters`/`tiny-skia`; interactive curve editor is
`TouchArea` + `Path` math in Rust. Workable, clunkier than iced Canvas.

Verdict: great for a normal window (proven by ROG CC); the overlay story is
the weak point.

## 5. Others (brief)

- **Tauri 2.11.5** (2026-07-01): CSS gives maximal fashion, but Linux uses
  tao (GTK3) + wry (webkit2gtk-4.1 2.52.6 is installed). Layer-shell needs the
  GTK3 `gtk-layer-shell` hack on the GTK window behind the webview; on
  NVIDIA + Wayland, WebKitGTK's DMABUF renderer blank-window/flicker bugs are
  still current (`WEBKIT_DISABLE_DMABUF_RENDERER=1`, `__NV_DISABLE_EXPLICIT_SYNC=1`
  workarounds per Tauri's own "Linux graphics issues" doc). Heavy runtime,
  JS toolchain. Not recommended for a hardware overlay.
- **Dioxus 0.7.10** (0.8.0-alpha.1): desktop = wry/webkit2gtk, same caveats as
  Tauri; "Dioxus Native" (wgpu/blitz) is experimental. No layer-shell.
- **Xilem 0.4.0** (2025-10-29): alpha, winit+vello; no layer-shell; API churn.
- **Freya 0.4.3 / 0.5.0-rc.5** (2026-09-07): Skia + winit, Dioxus-style
  components; nice visuals; no layer-shell.
- **Vizia 0.4.0** (2026-04-23): signals-based reactivity, CSS-like styling
  with variables; winit; no layer-shell; small community.
- **Makepad 1.0.0** (2025-05): own shader-DSL UI, GPU-first, gorgeous demos;
  Linux backend is X11/OpenGL-first, Wayland support unclear; no layer-shell.

None of these can do a native Hyprland overlay without you writing a
windowing backend.

---

## 6. Recommendation

**Use `iced 0.14` + `iced_exwlshell 0.20`.**

- *Elegance/fashion:* iced has no platform look to fight. Gradients, shadows,
  rounded borders, Oklch palettes, custom fonts, and — uniquely — custom wgpu
  shader widgets for glow/gauges. Transparent app background + compositor
  blur (`BlurOption::FullRegion`, natively supported by Hyprland 0.56) gives
  the "glass quick-settings panel" look for free.
- *Overlay:* proven today — one daemon, `StartMode::Background`, hotkey →
  `NewLayerShell` with `KeyboardInteractivity::OnDemand`, and `NewBaseWindow`
  for the full app. Popups/menus also supported.
- *Charts/gauges/curve editor:* `canvas::Program` with event handling is the
  best curve-editor primitive of all the candidates; caches keep live charts
  cheap; shader widget for showpieces.
- *Animation:* built into the framework (`iced::animation` over `lilt`),
  plus `iced_anim` for declarative widget transitions.
- *Maintainability:* Elm architecture, plain Rust types, no GObject, no CSS
  cascade debugging, single language.
- *Build reliability:* pure Rust, no system-library version coupling, wgpu on
  both NVIDIA and AMD. Cost: pin `winit-core`/`winit-common` beta and commit
  `Cargo.lock`; expect one migration when iced 1.0 lands.

Fallback if the pre-release winit dependency becomes a recurring problem:
GTK4 + libadwaita + gtk4-layer-shell (Appendix B). It builds in 22 s, has
mature layer-shell, and 4.22's CSS (`backdrop-filter`, conic gradients,
keyframes) is genuinely fashion-capable — at the cost of GObject ergonomics
and GNOME visual gravity.

## 7. Supporting crates (current versions, 2026-09-09)

| purpose | crate | version |
|---|---|---|
| D-Bus (asusd, UPower, logind, notifications) | `zbus` | **5.19.0** (no 6.x yet); `zbus_macros` 5.19.0 |
| async runtime | `tokio` | **1.53.1** |
| serialization | `serde` 1.0.229, `ron` 0.12.2, `toml` 1.1.5 |
| NVIDIA telemetry | `nvml-wrapper` | **0.13.0** (2026-08-31) |
| HID devices (keyboard LEDs, AniMe, etc.) | `hidapi` | **2.6.7** (defaults `linux-static-hidraw`; `linux-native` avoids the C lib) |
| udev enumeration/hotplug | `udev` | 0.9.3 |
| evdev (extra keys) | `evdev` | 0.13.2 |
| system stats | `sysinfo` | **0.39.6** |
| file watching (config, sysfs) | `notify` | **8.2.0** stable (9.0.0-rc.5 prerelease) |
| XDG dirs | `directories` | 6.0.0 |
| logging | `tracing` 0.1.44, `tracing-subscriber` 0.3.23 |
| errors | `anyhow` 1.0.104, `thiserror` 2.0.20 |
| CLI (`omaasus toggle`) | `clap` | 4.6.6 |
| portals (global shortcuts, screenshots) | `ashpd` | 0.13.13 |
| colour math (Oklch palettes) | `palette` | 0.7.7 |
| off-screen chart rendering (only if needed, e.g. PNG export) | `plotters` | 0.3.7 (2024; skip `plotters-iced`) |
| channels for subscriptions | `futures` 0.3.34, `async-channel` 2.5.0 |
| GPU (matches iced 0.14) | `wgpu` | 27.x (do not add 30.0.1 directly — iced pins 27) |

---

## Appendix A — Recommended stack: Cargo.toml + skeleton (compiles and runs here)

```toml
[package]
name = "omaasus"
version = "0.1.0"
edition = "2024"
rust-version = "1.98"

[dependencies]
iced = { version = "0.14", features = ["wgpu", "canvas", "tokio", "advanced", "image", "svg"] }
iced_exwlshell = "0.20"
iced_aw = { version = "0.14", default-features = false, features = ["number_input", "tab_bar", "slide_bar"] }
iced_anim = "0.3"          # optional declarative transitions
lilt = "0.8"               # same crate iced::animation uses

zbus = { version = "5.19", default-features = false, features = ["tokio"] }
tokio = { version = "1.53", features = ["rt-multi-thread", "macros", "sync", "time", "net"] }
serde = { version = "1", features = ["derive"] }
ron = "0.12"
toml = "1.1"
nvml-wrapper = "0.13"
hidapi = "2.6"
sysinfo = "0.39"
notify = "8.2"
directories = "6"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow = "1"
thiserror = "2"
clap = { version = "4.6", features = ["derive"] }
palette = "0.7"

# Workaround (2026-09-09): winit-core 0.31.0-beta.3 breaks iced_exdevtools 0.20.0
# (non-exhaustive NativeKeyCode match). Keep pinned until upstream releases a fix.
winit-core = "=0.31.0-beta.2"
winit-common = "=0.31.0-beta.2"

[profile.dev]
opt-level = 1            # wgpu/naga are slow at opt-level 0
[profile.dev.package."*"]
opt-level = 2
```

```rust
//! One iced program: starts headless, toggles a keyboard-interactive
//! layer-shell overlay (with compositor blur), and can open a normal window.
use std::collections::HashMap;

use iced::widget::{button, canvas, column, container, text};
use iced::window::Id;
use iced::{Color, Element, Length, Point, Task, Theme};
use iced_exwlshell::actions::IcedXdgWindowSettings;
use iced_exwlshell::daemon;
use iced_exwlshell::reexport::{
    Anchor, BlurOption, KeyboardInteractivity, Layer, LayerSize, NewLayerShellSettings,
    OutputOption, PixelSize,
};
use iced_exwlshell::settings::{LayerShellSettings, Settings, StartMode};
use iced_exwlshell::to_exwlshell_message;

pub fn main() -> Result<(), iced_exwlshell::Error> {
    daemon(App::default, App::namespace, App::update, App::view)
        .theme(App::theme)
        .style(App::style)
        .subscription(App::subscription)
        // .font(include_bytes!("../assets/Inter-Variable.ttf").as_slice())
        .settings(Settings {
            layer_settings: LayerShellSettings {
                start_mode: StartMode::Background, // no surface until asked
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface { Overlay, MainWindow }

#[derive(Default)]
struct App {
    fan_rpm: f32,
    ids: HashMap<Id, Surface>,
}

#[to_exwlshell_message(multi)]
#[derive(Debug, Clone)]
enum Message {
    ToggleOverlay,   // fed by a Subscription listening on D-Bus / a Unix socket
    OpenMainWindow,
    Close(Id),
    Tick,
}

impl App {
    fn namespace() -> String { "omaasus".into() }
    fn theme(&self, _id: Id) -> Theme { Theme::CatppuccinMocha }
    fn style(&self, theme: &Theme) -> iced::theme::Style {
        iced::theme::Style { background_color: Color::TRANSPARENT, text_color: theme.palette().text }
    }
    fn subscription(&self) -> iced::Subscription<Message> {
        iced::time::every(std::time::Duration::from_millis(500)).map(|_| Message::Tick)
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::ToggleOverlay => {
                if let Some((&id, _)) = self.ids.iter().find(|(_, s)| **s == Surface::Overlay) {
                    self.ids.remove(&id);
                    return Task::done(Message::RemoveWindow(id));
                }
                let (id, task) = Message::layershell_open(NewLayerShellSettings {
                    size: LayerSize::px(420, 640),
                    layer: Layer::Overlay,
                    anchor: Anchor::Right | Anchor::Top,
                    margin: Some((24, 24, 0, 0)),
                    exclusive_zone: Some(0),
                    keyboard_interactivity: KeyboardInteractivity::OnDemand,
                    output_option: OutputOption::LastOutput,
                    blur_option: BlurOption::FullRegion, // ext-background-effect-v1 (Hyprland OK)
                    namespace: Some("omaasus-overlay".into()),
                    ..Default::default()
                });
                self.ids.insert(id, Surface::Overlay);
                task
            }
            Message::OpenMainWindow => {
                let (id, task) = Message::base_window_open(IcedXdgWindowSettings {
                    size: Some(PixelSize::px(1100, 720)),
                    client_side_decorations: false,
                });
                self.ids.insert(id, Surface::MainWindow);
                task
            }
            Message::Close(id) => {
                self.ids.remove(&id);
                Task::done(Message::RemoveWindow(id))
            }
            Message::Tick => {
                self.fan_rpm = (self.fan_rpm + 137.0) % 6000.0;
                Task::none()
            }
            _ => Task::none(), // macro-injected variants are consumed by the runtime
        }
    }

    fn view(&self, id: Id) -> Element<'_, Message> {
        let gauge = canvas(Gauge { value: self.fan_rpm / 6000.0 })
            .width(Length::Fixed(200.0))
            .height(Length::Fixed(200.0));
        let body = match self.ids.get(&id) {
            Some(Surface::Overlay) => column![
                text("Quick panel").size(24),
                gauge,
                button("Open full app").on_press(Message::OpenMainWindow),
                button("Close").on_press(Message::Close(id)),
            ],
            _ => column![text("Main window").size(32), gauge, button("Close").on_press(Message::Close(id))],
        };
        container(body.spacing(12).padding(20)).width(Length::Fill).height(Length::Fill).into()
    }
}

struct Gauge { value: f32 }
impl canvas::Program<Message> for Gauge {
    type State = ();
    fn draw(&self, _s: &(), renderer: &iced::Renderer, theme: &Theme,
            bounds: iced::Rectangle, _cursor: iced::mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let c = frame.center();
        let r = bounds.width.min(bounds.height) / 2.0 - 8.0;
        let start = 0.75 * std::f32::consts::PI;
        let sweep = 1.5 * std::f32::consts::PI * self.value;
        let arc = |end: f32| canvas::Path::new(|b| {
            b.arc(canvas::path::Arc { center: c, radius: r, start_angle: start.into(), end_angle: end.into() })
        });
        frame.stroke(&arc(start + 1.5 * std::f32::consts::PI),
            canvas::Stroke::default().with_width(10.0).with_color(theme.extended_palette().background.strong.color));
        frame.stroke(&arc(start + sweep),
            canvas::Stroke::default().with_width(10.0).with_color(theme.palette().primary).with_line_cap(canvas::LineCap::Round));
        frame.fill_text(canvas::Text {
            content: format!("{:.0}%", self.value * 100.0), position: Point::new(c.x, c.y),
            color: theme.palette().text, size: 28.0.into(),
            align_x: iced::alignment::Horizontal::Center.into(), align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        vec![frame.into_geometry()]
    }
}
```

Hotkey wiring (not in the skeleton): expose `dev.omaasus.Control.Toggle()` via
`zbus` in a `Subscription`, and bind in Hyprland:
`bind = SUPER, F12, exec, omaasus toggle` (the CLI calls the method). See
upstream example `iced_examples/zbus_invoked_widget`.

## Appendix B — Fallback stack: GTK4 + libadwaita + gtk4-layer-shell (compiles and runs here)

```toml
[dependencies]
gtk4 = { version = "0.11", features = ["v4_22"] }
libadwaita = { version = "0.9", features = ["v1_9"] }
gtk4-layer-shell = { version = "0.8", features = ["v1_3"] }
```

```rust
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use libadwaita as adw;
use adw::prelude::*;

fn build_window(app: &adw::Application, overlay: bool) -> adw::ApplicationWindow {
    let win = adw::ApplicationWindow::new(app);
    if overlay && gtk4_layer_shell::is_supported() {
        win.init_layer_shell();
        win.set_namespace(Some("omaasus-overlay"));
        win.set_layer(Layer::Overlay);
        win.set_anchor(Edge::Right, true);
        win.set_anchor(Edge::Top, true);
        win.set_margin(Edge::Right, 24);
        win.set_margin(Edge::Top, 24);
        win.set_keyboard_mode(KeyboardMode::OnDemand);
        win.set_default_size(420, 640);
    } else {
        win.set_default_size(1100, 720);
    }
    let da = gtk4::DrawingArea::new();
    da.set_draw_func(|_, cr, w, h| { /* cairo: gauges, fan curve */ let _ = (cr, w, h); });
    win.set_content(Some(&da));
    win
}

fn main() {
    let app = adw::Application::builder().application_id("dev.omaasus.ControlCenter").build();
    app.connect_activate(|app| {
        let overlay = std::env::args().any(|a| a == "--overlay");
        let css = gtk4::CssProvider::new();
        css.load_from_string(
            "window { background: transparent; }
             .panel { background: color-mix(in oklch, #0f0f14 85%, transparent);
                      backdrop-filter: blur(24px); border-radius: 24px;
                      box-shadow: 0 16px 48px rgba(0,0,0,.45); }");
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(), &css, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
        build_window(app, overlay).present();
    });
    app.run();
}
```

Measured: clean debug 22.0 s, incremental 0.2 s, clean release 28.5 s.

## Sources

- crates.io API for every version above (2026-09-09)
- https://github.com/iced-rs/iced/releases/tag/0.14.0
- https://docs.rs/iced/latest/iced/animation/ ; https://docs.rs/iced/latest/iced/widget/shader/
- https://github.com/waycrate/exwlshelleventloop (README, iced_examples, commits to 2026-08-29)
- https://docs.rs/iced_exwlshell/0.20.0 and vendored sources of iced_exwlshell/exwlshellev/iced_exwlshell_macros 0.20.0
- https://docs.rs/crate/gtk4/0.11.4/features ; https://docs.rs/crate/libadwaita/0.9.2/features ; https://docs.rs/crate/gtk4-layer-shell/0.8.1/source/Cargo.toml
- https://docs.gtk.org/gtk4/css-properties.html ; GTK NEWS (4.21.2: backdrop-filter, repeating conic gradients; 4.22.0 released 2026-03-06)
- https://docs.rs/egui/0.36.2/egui/style/struct.Visuals.html ; https://docs.rs/epaint/0.36.2/epaint/enum.Shape.html ; https://docs.rs/eframe/latest/eframe/struct.NativeOptions.html
- https://github.com/slint-ui/slint/releases ; https://slint.dev/blog/slint-1.17-released ; https://github.com/slint-ui/slint/discussions/1765 ; https://docs.slint.dev/latest/docs/slint/reference/elements/rectangle/
- https://github.com/waydeerwm/layer-shika ; https://docs.rs/spell-framework
- https://raw.githubusercontent.com/flukejones/asusctl/main/Cargo.toml (rog-control-center: slint 1.17.1)
- https://v2.tauri.app/develop/debug/linux-graphics/ ; https://github.com/tauri-apps/tauri/issues/9394
- https://dioxuslabs.com/blog/release-070/ ; https://github.com/linebender/xilem/releases ; https://github.com/vizia/vizia/releases
