//! Settings: helper installation, CoolerControl credentials, overlay layout,
//! Hyprland hotkey snippet, LiveDash OLED test, diagnostics.

use crate::app::{App, Message};
use crate::pages::cpu::{slider_style, toggle};
use crate::theme::{self, size, space};
use crate::widgets;
use iced::widget::{column, container, row, scrollable, slider, Row};
use iced::{Element, Length};

#[derive(Debug, Clone)]
pub enum SettingsMsg {
    InstallHelper,
    RecheckHelper,
    CcUrl(String),
    CcPassword(String),
    CcTest,
    OverlayAnchor(String),
    OverlayWidth(f64),
    OverlayHeight(f64),
    OverlayOpacity(f64),
    OverlayMargin(f64),
    HudToggle(bool),
    OledText,
    OledHwMonitor,
    TelemetryHz(f64),
}

pub fn view(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    let inv = app.inventory.as_ref();
    let helper_ok = app.controller_ready;

    let helper = widgets::card(
        p,
        column![
            row![widgets::title(p, "Privileged helper"), widgets::hfill(), if helper_ok { widgets::pill(p, "installed & running", p.ok) } else { widgets::pill(p, "not running", p.warn) }].align_y(iced::Alignment::Center),
            widgets::dim(p, "Fans, CPU governor, GPU limits and lighting controllers live behind root-only sysfs/hidraw nodes. OmaAsus ships a small system service (com.omaasus.Helper1) gated by polkit: routine tuning is allowed for the active session, overclocking offsets and raw device writes ask for your password once."),
            row![
                widgets::btn(p, if helper_ok { "Reinstall helper" } else { "Install helper (pkexec)" }, widgets::ButtonKind::Primary, Some(Message::Settings(SettingsMsg::InstallHelper))),
                widgets::btn(p, "Re-check", widgets::ButtonKind::Ghost, Some(Message::Settings(SettingsMsg::RecheckHelper))),
            ]
            .spacing(space::SM),
            widgets::mono(p, app.helper_log.clone(), size::CAPTION),
        ]
        .spacing(space::MD),
    )
    .width(Length::Fill);

    let text_style = move |_: &iced::Theme, _: iced::widget::text_input::Status| iced::widget::text_input::Style { background: iced::Background::Color(p.glass), border: iced::Border { color: p.line_strong, width: 1.0, radius: theme::radius::SM.into() }, icon: p.text_dim, placeholder: p.text_faint, value: p.text, selection: p.accent_soft };
    let cc_detected = inv.map(|i| i.daemons.coolercontrold).unwrap_or(false);
    let cc = widgets::card(
        p,
        column![
            row![widgets::title(p, "CoolerControl"), widgets::hfill(), if app.cc_connected { widgets::pill(p, "connected", p.ok) } else if cc_detected { widgets::pill(p, "detected — sign in", p.warn) } else { widgets::pill(p, "not installed", p.text_dim) }].align_y(iced::Alignment::Center),
            widgets::dim(p, "If CoolerControl runs on this machine, OmaAsus can delegate fan curves to it and switch its Modes per profile. Enter the daemon URL and the CCAdmin password (default: coolAdmin)."),
            row![
                iced::widget::text_input("http://localhost:11987", &app.config.coolercontrol.url).on_input(|s| Message::Settings(SettingsMsg::CcUrl(s))).font(theme::font::MONO).size(size::BODY).style(text_style).width(Length::Fixed(260.0)),
                iced::widget::text_input("password", app.config.coolercontrol.password.as_deref().unwrap_or("")).secure(true).on_input(|s| Message::Settings(SettingsMsg::CcPassword(s))).font(theme::font::MONO).size(size::BODY).style(text_style).width(Length::Fixed(200.0)),
                widgets::btn(p, "Connect", widgets::ButtonKind::Primary, Some(Message::Settings(SettingsMsg::CcTest))),
            ]
            .spacing(space::SM)
            .wrap(),
        ]
        .spacing(space::MD),
    )
    .width(Length::Fill);

    let o = &app.config.overlay;
    let anchor_btn = |label: &str, a: &'static str| widgets::btn(p, label, if o.anchor == a { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Settings(SettingsMsg::OverlayAnchor(a.into()))));
    let overlay = widgets::card(
        p,
        column![
            widgets::title(p, "Overlay"),
            widgets::dim(p, "The overlay is a layer-shell panel with compositor blur, toggled from anywhere with `omaasus toggle`. Bind it in Hyprland:"),
            widgets::mono(p, "o.bind(\"SUPER + F12\", \"OmaAsus overlay\", hl.dsp.exec({ cmd = \"omaasus toggle\" }))", size::SMALL),
            widgets::dim(p, "(classic syntax: bind = SUPER, F12, exec, omaasus toggle). Add `omaasus --overlay` to autostart so the daemon is always ready."),
            row![widgets::eyebrow(p, "Anchor"), anchor_btn("Right", "right"), anchor_btn("Left", "left"), anchor_btn("Top center", "center")].spacing(space::SM).align_y(iced::Alignment::Center),
            row![
                column![row![widgets::eyebrow(p, "Width"), widgets::hfill(), widgets::mono(p, format!("{} px", o.width), size::SMALL)], slider(360.0..=900.0, o.width as f64, |v| Message::Settings(SettingsMsg::OverlayWidth(v))).step(10.0).style(slider_style(p))].spacing(space::XS).width(Length::Fill),
                column![row![widgets::eyebrow(p, "Height"), widgets::hfill(), widgets::mono(p, if o.height == 0 { "full".into() } else { format!("{} px", o.height) }, size::SMALL)], slider(0.0..=1600.0, o.height as f64, |v| Message::Settings(SettingsMsg::OverlayHeight(v))).step(50.0).style(slider_style(p))].spacing(space::XS).width(Length::Fill),
                column![row![widgets::eyebrow(p, "Margin"), widgets::hfill(), widgets::mono(p, format!("{} px", o.margin), size::SMALL)], slider(0.0..=64.0, o.margin as f64, |v| Message::Settings(SettingsMsg::OverlayMargin(v))).step(2.0).style(slider_style(p))].spacing(space::XS).width(Length::Fill),
                column![row![widgets::eyebrow(p, "Opacity"), widgets::hfill(), widgets::mono(p, format!("{:.0}%", o.opacity * 100.0), size::SMALL)], slider(0.5..=1.0, o.opacity as f64, |v| Message::Settings(SettingsMsg::OverlayOpacity(v))).step(0.02).style(slider_style(p))].spacing(space::XS).width(Length::Fill),
            ]
            .spacing(space::LG),
            toggle(p, "Compact HUD strip when a game is fullscreen", o.hud_enabled, true, |b| Message::Settings(SettingsMsg::HudToggle(b))),
            row![widgets::eyebrow(p, "Telemetry rate"), widgets::hfill(), widgets::mono(p, format!("{} Hz", app.config.telemetry_hz), size::SMALL)],
            slider(1.0..=5.0, app.config.telemetry_hz as f64, |v| Message::Settings(SettingsMsg::TelemetryHz(v))).step(1.0).style(slider_style(p)),
        ]
        .spacing(space::MD),
    )
    .width(Length::Fill);

    let has_oled = inv.map(|i| i.features.livedash_oled).unwrap_or(false);
    let oled = widgets::card(
        p,
        column![
            row![widgets::title(p, "LiveDash OLED"), widgets::hfill(), if has_oled { widgets::pill(p, "detected (0b05:1a21)", p.ok) } else { widgets::pill(p, "not present", p.text_dim) }].align_y(iced::Alignment::Center),
            widgets::dim(p, "ROG Extreme boards carry a 2\" OLED (and the AniMe Matrix on newer models). OmaAsus can push a two-line text readout (CPU/GPU temperature) in the chip's text mode — experimental, requires the helper."),
            row![
                widgets::btn(p, "Show live temps on OLED", widgets::ButtonKind::Primary, has_oled.then_some(Message::Settings(SettingsMsg::OledText))),
                widgets::btn(p, "Back to hardware monitor", widgets::ButtonKind::Ghost, has_oled.then_some(Message::Settings(SettingsMsg::OledHwMonitor))),
            ]
            .spacing(space::SM),
        ]
        .spacing(space::MD),
    )
    .width(Length::Fill);

    let diag: Element<Message> = match inv {
        None => widgets::dim(p, "…"),
        Some(i) => {
            let f = &i.features;
            let flag = |name: &str, on: bool| widgets::pill(p, name, if on { p.ok } else { p.text_faint });
            let flags: Vec<Element<Message>> = vec![
                flag("NVIDIA", f.nvidia), flag("AMD GPU", f.amd_gpu), flag("Super I/O fans", f.super_io_fans), flag("Ryujin AIO", f.ryujin_aio), flag("LiveDash OLED", f.livedash_oled),
                flag("Aura USB", f.aura_usb), flag("Lian Li hub", f.lianli_uni_hub), flag("ASUS EC sensors", f.asus_ec_sensors), flag("EPP", f.cpu_epp), flag("boost", f.cpu_boost), flag("SMT", f.cpu_smt),
                flag("platform_profile", f.platform_profile), flag("asusd", f.asusd), flag("supergfxd", f.supergfxd), flag("CoolerControl", f.coolercontrol), flag("OpenRGB", f.openrgb), flag("liquidctl", f.liquidctl), flag("GameMode", i.daemons.gamemode), flag("power-profiles-daemon", i.daemons.power_profiles_daemon),
            ];
            column![
                widgets::mono(p, format!("{:?} · {} {} · BIOS {} ({}) · kernel {}", i.platform, i.dmi.board_vendor, i.dmi.board_name, i.dmi.bios_version, i.dmi.bios_date, i.kernel), size::SMALL),
                Row::with_children(flags).spacing(space::XS).wrap(),
                widgets::dim(p, format!("config: {}", crate::config_store::path().display())),
            ]
            .spacing(space::SM)
            .into()
        }
    };
    let diagnostics = widgets::card(p, column![widgets::title(p, "Detected hardware"), diag].spacing(space::MD)).width(Length::Fill);

    let left = scrollable(column![helper, cc, oled].spacing(space::LG)).height(Length::Fill);
    let right = scrollable(column![overlay, diagnostics].spacing(space::LG)).height(Length::Fill);
    column![
        widgets::headline(p, "Settings"),
        row![container(left).width(Length::FillPortion(1)), container(right).width(Length::FillPortion(1))].spacing(space::LG).height(Length::Fill),
    ]
    .spacing(space::LG)
    .height(Length::Fill)
    .into()
}
