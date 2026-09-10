//! Cooling page: fan targets, per-target mode, interactive curve editor,
//! and the fan-engine owner selector (OmaAsus vs CoolerControl).

use crate::app::{App, Message};
use crate::pages::cpu::slider_style;
use crate::theme::{size, space};
use crate::widgets::{self, curve::{CurveEditor, CurveEvent}};
use iced::widget::{canvas, column, row, scrollable, slider, Column, Row};
use iced::{Element, Length};
use oma_hw::model::CurveTemp;
use oma_hw::profile::{FanCurve, FanMode, FanOwner, FanTarget, TempSource};

#[derive(Debug, Clone)]
pub enum CoolingMsg {
    Select(FanTarget),
    Mode(FanTarget, &'static str),
    Curve(CurveEvent),
    Fixed(f64),
    Source(TempSource),
    MinDuty(f64),
    Ramp(f64),
    Hysteresis(f64),
    Preset(&'static str),
    Owner(FanOwner),
    CcMode(String),
    CcRefresh,
}

pub fn view(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    let (Some(inv), Some(model)) = (app.inventory.as_ref(), app.model.as_deref()) else { return widgets::dim(p, "Detecting cooling hardware…") };
    let snap = app.snapshot.as_ref();
    let snap_ref: Option<&crate::telemetry::Snapshot> = snap.map(|s| &**s);
    let available = crate::fans::FanBackend::available(model, snap_ref);
    let profile = app.active_profile();
    let owner_effective = app.effective_fan_owner();

    // ---- owner card --------------------------------------------------------
    let owner_chip = |label: &str, o: FanOwner| widgets::btn(p, label, if app.config.fan_owner == o { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Cooling(CoolingMsg::Owner(o))));
    let status = match owner_effective {
        FanOwner::OmaAsus => widgets::pill(p, "OmaAsus fan engine active", p.ok),
        FanOwner::CoolerControl => widgets::pill(p, if app.cc_connected { "CoolerControl connected" } else { "CoolerControl detected — sign in under Settings" }, if app.cc_connected { p.ok } else { p.warn }),
        _ => widgets::pill(p, "Fans left to firmware", p.text_dim),
    };
    let owner = widgets::card(
        p,
        column![
            row![widgets::title(p, "Fan engine"), widgets::hfill(), status].align_y(iced::Alignment::Center),
            widgets::dim(p, "Choose who drives the fans. CoolerControl (when installed) keeps its own curves; OmaAsus then activates a CoolerControl Mode per profile. The OmaAsus engine evaluates the curves below itself, through the privileged helper."),
            Row::with_children(vec![owner_chip("Automatic", FanOwner::Auto), owner_chip("OmaAsus", FanOwner::OmaAsus), owner_chip("CoolerControl", FanOwner::CoolerControl), owner_chip("Off", FanOwner::None)]).spacing(space::SM).wrap(),
        ]
        .spacing(space::MD),
    )
    .width(Length::Fill);

    if owner_effective == FanOwner::CoolerControl {
        let modes: Vec<Element<Message>> = app
            .cc_modes
            .iter()
            .map(|m| {
                let active = profile.and_then(|pr| pr.cc_mode.as_ref()) == Some(&m.uid);
                widgets::btn(p, &m.name, if active { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Cooling(CoolingMsg::CcMode(m.uid.clone()))))
            })
            .collect();
        let cc_card = widgets::card(
            p,
            column![
                row![widgets::title(p, "CoolerControl mode for this profile"), widgets::hfill(), widgets::btn(p, "Refresh", widgets::ButtonKind::Ghost, Some(Message::Cooling(CoolingMsg::CcRefresh)))].align_y(iced::Alignment::Center),
                widgets::dim(p, format!("Profile “{}” activates the selected CoolerControl Mode when applied. Create Modes in CoolerControl (Modes → save current settings).", profile.map(|x| x.name.as_str()).unwrap_or("—"))),
                if modes.is_empty() { widgets::dim(p, if app.cc_connected { "No Modes defined yet." } else { "Not connected." }) } else { Row::with_children(modes).spacing(space::SM).wrap().into() },
            ]
            .spacing(space::MD),
        )
        .width(Length::Fill);
        return column![owner, cc_card, fan_readings(app).height(Length::Fill)].spacing(space::LG).height(Length::Fill).into();
    }

    // ---- targets list -------------------------------------------------------
    let Some(profile_ref) = profile else { return widgets::dim(p, "No active profile.") };
    let cooling: &oma_hw::profile::CoolingSettings = &profile_ref.cooling;
    let selected = app.cooling_sel.clone().or_else(|| available.first().map(|a| a.target.clone()));
    let list: Vec<Element<Message>> = available
        .iter()
        .map(|a| {
            let is_sel = Some(&a.target) == selected.as_ref();
            let mode = cooling.get(&a.target);
            let mode_label = match mode {
                None | Some(FanMode::Auto) => ("auto", p.text_faint),
                Some(FanMode::Fixed(d)) => return target_row(app, a, is_sel, format!("fixed {d:.0}%"), p.text_dim),
                Some(FanMode::Curve(_)) => ("curve", p.accent),
                Some(FanMode::HardwareCurve(_)) => ("hw curve", p.accent_2),
            };
            target_row(app, a, is_sel, mode_label.0.into(), mode_label.1)
        })
        .collect();
    let targets = widgets::card(p, column![widgets::eyebrow(p, "Outputs"), scrollable(Column::with_children(list).spacing(space::XS)).height(Length::Fill)].spacing(space::MD).height(Length::Fill)).width(Length::Fixed(300.0)).height(Length::Fill);

    // ---- editor -------------------------------------------------------------
    let editor: Element<Message> = match selected {
        None => widgets::card(p, widgets::dim(p, "No controllable fan outputs detected.")).into(),
        Some(target) => {
            static AUTO: FanMode = FanMode::Auto;
            let mode: &FanMode = cooling.get(&target).unwrap_or(&AUTO);
            let kind = match mode {
                FanMode::Auto => "auto",
                FanMode::Fixed(_) => "fixed",
                FanMode::Curve(_) => "curve",
                FanMode::HardwareCurve(_) => "hw",
            };
            // Only the modes this output supports.
            let caps = available.iter().find(|a| a.target == target).map(|a| a.caps.clone());
            let takes_duty = caps.as_ref().is_some_and(|c| c.duty);
            let mut kinds = vec![("Auto", "auto")];
            if takes_duty {
                kinds.extend([("Fixed", "fixed"), ("Curve", "curve")]);
            }
            if caps.as_ref().is_some_and(|c| c.firmware_curve.is_some()) {
                kinds.push((if takes_duty { "Hardware curve" } else { "Firmware curve" }, "hw"));
            }
            let kind_row = Row::with_children(kinds.into_iter().map(|(l, k)| widgets::btn(p, l, if k == kind { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Cooling(CoolingMsg::Mode(target.clone(), k))))).collect::<Vec<_>>()).spacing(space::SM).wrap();
            let live_duty = app.fan_engine_duty(&target);
            let floor = app.fan_engine.floor(&target);
            let body: Element<Message> = match mode {
                FanMode::Auto if takes_duty => widgets::dim(p, "Firmware / driver default behaviour. Pick Fixed or Curve to take control.").into(),
                FanMode::Auto => widgets::dim(p, "Runs the firmware's own curve for the current power mode. Pick Firmware curve to set this profile's own.").into(),
                FanMode::Fixed(d) => column![
                    row![widgets::eyebrow(p, "Duty"), widgets::hfill(), widgets::mono(p, format!("{d:.0}%"), size::SMALL)],
                    slider(floor..=100.0, d.max(floor), |v| Message::Cooling(CoolingMsg::Fixed(v))).step(1.0).style(slider_style(p)),
                ]
                .spacing(space::SM)
                .into(),
                FanMode::Curve(c) | FanMode::HardwareCurve(c) => {
                    let temps = snap.map(|s| crate::fans::temps_from(s));
                    let now_t = temps.as_ref().and_then(|t| t.resolve(&c.source));
                    let live = now_t.map(|t| (t, live_duty.unwrap_or_else(|| c.duty_at(t))));
                    let mut sources = vec![TempSource::CpuTctl, TempSource::Gpu, TempSource::CpuGpuMax, TempSource::Coolant, TempSource::Vrm, TempSource::Motherboard];
                    for d in &inv.hwmon {
                        if matches!(d.name.as_str(), "asusec") || d.is_super_io() {
                            for t in &d.temps {
                                let live = snap_ref.map(|s| s.hwmon_temps.contains_key(&(d.name.clone(), t.label.clone()))).unwrap_or(false);
                                if live && !t.label.starts_with("PCH") && !t.label.starts_with("AUXTIN") {
                                    sources.push(TempSource::Hwmon { driver: d.name.clone(), label: t.label.clone() });
                                }
                            }
                        }
                    }
                    let src_row = Row::with_children(sources.into_iter().map(|s| {
                        let active = s == c.source;
                        widgets::btn(p, s.label(), if active { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Cooling(CoolingMsg::Source(s))))
                    }).collect::<Vec<_>>()).spacing(space::XS).wrap();
                    // Firmware curves run on the fan's own sensor inside the firmware:
                    // no source to pick, no software ramp or hysteresis, fixed points.
                    let firmware_temp = matches!(mode, FanMode::HardwareCurve(_)) && caps.as_ref().and_then(|c| c.firmware_curve.as_ref()).is_some_and(|s| s.temp == CurveTemp::Firmware);
                    let min_duty = || column![row![widgets::eyebrow(p, "Minimum duty"), widgets::hfill(), widgets::mono(p, format!("{:.0}%", c.min_duty.max(floor)), size::SMALL)], slider(floor..=100.0, c.min_duty.max(floor), |v| Message::Cooling(CoolingMsg::MinDuty(v))).step(1.0).style(slider_style(p))].spacing(space::XS).width(Length::Fill);
                    let preset = |label: &str, name: &'static str| widgets::btn(p, label, widgets::ButtonKind::Ghost, Some(Message::Cooling(CoolingMsg::Preset(name))));
                    let mut presets = vec![widgets::eyebrow(p, "Presets"), preset("Silent", "silent"), preset("Balanced", "balanced"), preset("Performance", "performance")];
                    let mut body = Column::new()
                        .push(canvas(CurveEditor { palette: p, points: &c.points, color: p.accent, live, min_duty: c.min_duty, on_event: |e| Message::Cooling(CoolingMsg::Curve(e)), editable: true }).width(Length::Fill).height(Length::Fill))
                        .push(widgets::dim(p, if firmware_temp { "Drag the points · the firmware runs this curve on the fan's own temperature, for this profile's power mode" } else { "Drag points · click to add · right-click to remove" }));
                    if firmware_temp {
                        body = body.push(row![min_duty()].spacing(space::LG));
                    } else {
                        body = body.push(widgets::eyebrow(p, "Temperature source")).push(src_row).push(
                            row![
                                min_duty(),
                                column![row![widgets::eyebrow(p, "Ramp (s / full sweep)"), widgets::hfill(), widgets::mono(p, format!("{:.0}s", c.ramp_s), size::SMALL)], slider(0.0..=30.0, c.ramp_s, |v| Message::Cooling(CoolingMsg::Ramp(v))).step(1.0).style(slider_style(p))].spacing(space::XS).width(Length::Fill),
                                column![row![widgets::eyebrow(p, "Hysteresis"), widgets::hfill(), widgets::mono(p, format!("{:.1}°", c.hysteresis_c), size::SMALL)], slider(0.0..=10.0, c.hysteresis_c, |v| Message::Cooling(CoolingMsg::Hysteresis(v))).step(0.5).style(slider_style(p))].spacing(space::XS).width(Length::Fill),
                            ]
                            .spacing(space::LG),
                        );
                        presets.extend([preset("Coolant", "coolant"), preset("Pump", "pump")]);
                    }
                    body.push(Row::with_children(presets).spacing(space::SM).align_y(iced::Alignment::Center)).spacing(space::MD).height(Length::Fill).into()
                }
            };
            let name = available.iter().find(|a| a.target == target).map(|a| a.label.clone()).unwrap_or_else(|| target.to_string());
            widgets::card(
                p,
                column![
                    row![widgets::title(p, name), widgets::hfill(), widgets::dim(p, format!("profile: {}", profile.map(|x| x.name.as_str()).unwrap_or("—"))), live_duty.map(|d| widgets::pill(p, format!("driving {d:.0}%"), p.ok)).unwrap_or_else(|| iced::widget::Space::new().into())].spacing(space::SM).align_y(iced::Alignment::Center),
                    kind_row,
                    body,
                ]
                .spacing(space::LG)
                .height(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        }
    };

    let left = column![targets.height(Length::FillPortion(3)), fan_readings(app).height(Length::FillPortion(2))].spacing(space::LG).width(Length::Fixed(320.0)).height(Length::Fill);
    column![owner, row![left, editor].spacing(space::LG).height(Length::Fill)].spacing(space::LG).height(Length::Fill).into()
}

fn target_row<'a>(app: &'a App, a: &crate::fans::Available, selected: bool, mode: String, color: iced::Color) -> Element<'a, Message> {
    let p = app.palette;
    let content = row![
        column![widgets::body(p, a.label.clone()), widgets::dim(p, a.detail.clone())].spacing(2.0).width(Length::Fill),
        widgets::pill(p, mode, color),
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center);
    iced::widget::button(content).width(Length::Fill).padding([8, 10]).style(widgets::button_style(p, widgets::ButtonKind::Nav { active: selected })).on_press(Message::Cooling(CoolingMsg::Select(a.target.clone()))).into()
}

fn fan_readings(app: &App) -> iced::widget::Container<'_, Message> {
    let p = app.palette;
    let rows: Vec<Element<Message>> = app
        .snapshot
        .as_ref()
        .map(|s| {
            s.fans
                .iter()
                .map(|f| widgets::fan_row(p, f))
                .collect()
        })
        .unwrap_or_default();
    let body: Element<Message> = if rows.is_empty() { widgets::dim(p, "No tachometer signals yet.") } else { scrollable(Column::with_children(rows).spacing(space::SM)).height(Length::Fill).into() };
    widgets::card(p, column![widgets::eyebrow(p, "Live readings"), body].spacing(space::MD).height(Length::Fill)).width(Length::Fill)
}

pub fn preset(name: &str, source: TempSource) -> FanCurve {
    let mut c = match name {
        "silent" => FanCurve::silent(),
        "performance" => FanCurve::performance(),
        "coolant" => FanCurve::coolant(),
        "pump" => FanCurve::pump(),
        _ => FanCurve::balanced(),
    };
    if !matches!(name, "coolant" | "pump") {
        c.source = source;
    }
    c
}

