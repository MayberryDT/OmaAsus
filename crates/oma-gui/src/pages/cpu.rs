//! Processor page: governor / EPP / boost / SMT / frequency limits, per-core
//! live view, and preferred-core ranking.

use crate::app::{App, Message};
use crate::theme::{self, size, space};
use crate::widgets::{self, sparkline::Sparkline};
use iced::widget::{canvas, column, row, scrollable, slider, Column, Row};
use iced::{Element, Length};

#[derive(Debug, Clone)]
pub enum CpuMsg {
    Governor(String),
    Epp(String),
    Boost(bool),
    Smt(bool),
    MaxMhz(f64),
    MinMhz(f64),
    Apply,
    Revert,
    SaveToProfile,
}

pub fn view(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    let snap = app.snapshot.as_ref();
    let Some(inv) = app.inventory.as_ref() else {
        return widgets::dim(p, "Detecting processor…");
    };
    let info = &inv.cpu;
    let edit = &app.cpu_edit;
    let live = snap.map(|s| s.cpu_control.clone()).unwrap_or_default();
    let dirty = *edit != live;

    let model = info.model.split(" 16-Core").next().unwrap_or(&info.model).split(" Processor").next().unwrap_or(&info.model).to_string();
    let header = Row::new()
        .spacing(space::XL)
        .align_y(iced::Alignment::Center)
        .push(
            column![
                widgets::eyebrow(p, "Processor"),
                widgets::headline(p, model),
                widgets::dim(p, format!("{} cores · {} threads · {} · amd-pstate {}", info.physical_cores, info.logical_cpus, info.scaling_driver.as_deref().unwrap_or("?"), info.amd_pstate_status.as_deref().unwrap_or("-"))),
            ]
            .spacing(space::XS),
        )
        .push(widgets::metric(p, "Tctl", format!("{:.0}", snap.and_then(|s| s.cpu.tctl_c).unwrap_or(0.0)), "°C", theme::thermal(&p, snap.and_then(|s| s.cpu.tctl_c).unwrap_or(0.0), 35.0, 95.0)))
        .push(widgets::metric(p, "Fastest core", format!("{:.0}", snap.map(|s| s.cpu.max_core_mhz).unwrap_or(0.0)), "MHz", p.cpu))
        .push(widgets::metric(p, "Load", format!("{:.0}", snap.map(|s| s.cpu.util_total).unwrap_or(0.0)), "%", p.cpu))
        .wrap();

    // --- Control card -----------------------------------------------------
    let chips = |label: &str, options: &[String], current: &str, mk: fn(String) -> CpuMsg| -> Element<'_, Message> {
        let buttons: Vec<Element<Message>> = options
            .iter()
            .map(|o| {
                let active = o == current;
                widgets::btn(p, pretty(o), if active { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Cpu(mk(o.clone()))))
            })
            .collect();
        column![widgets::eyebrow(p, label), Row::with_children(buttons).spacing(space::SM).wrap()].spacing(space::SM).into()
    };
    // Detection already fills in the list where the governor pins it
    // (oma_hw::cpu::epp_choices); a CPU without EPP has none to offer.
    let epp_options: Vec<String> = info.available_epp.clone();
    let toggles = row![
        toggle(p, "Core boost", edit.boost.unwrap_or(true), info.has_boost, |b| Message::Cpu(CpuMsg::Boost(b))),
        toggle(p, "SMT (threads)", edit.smt.unwrap_or(true), info.has_smt_control, |b| Message::Cpu(CpuMsg::Smt(b))),
    ]
    .spacing(space::LG);
    let max_mhz = (info.cpuinfo_max_khz / 1000) as f64;
    let min_mhz = (info.cpuinfo_min_khz / 1000) as f64;
    let cur_max = if edit.scaling_max_khz > 0 { edit.scaling_max_khz as f64 / 1000.0 } else { max_mhz };
    let cur_min = if edit.scaling_min_khz > 0 { edit.scaling_min_khz as f64 / 1000.0 } else { min_mhz };
    let limits = column![
        row![widgets::eyebrow(p, "Frequency ceiling"), widgets::hfill(), widgets::mono(p, format!("{cur_max:.0} MHz"), size::SMALL)].align_y(iced::Alignment::Center),
        slider(min_mhz..=max_mhz, cur_max, |v| Message::Cpu(CpuMsg::MaxMhz(v))).step(25.0).style(slider_style(p)),
        row![widgets::eyebrow(p, "Frequency floor"), widgets::hfill(), widgets::mono(p, format!("{cur_min:.0} MHz"), size::SMALL)].align_y(iced::Alignment::Center),
        slider(min_mhz..=max_mhz, cur_min, |v| Message::Cpu(CpuMsg::MinMhz(v))).step(25.0).style(slider_style(p)),
    ]
    .spacing(space::SM);
    let actions = row![
        widgets::btn(p, "Apply", widgets::ButtonKind::Primary, dirty.then_some(Message::Cpu(CpuMsg::Apply))),
        widgets::btn(p, "Revert", widgets::ButtonKind::Ghost, dirty.then_some(Message::Cpu(CpuMsg::Revert))),
        widgets::hfill(),
        widgets::btn(p, "Save into active profile", widgets::ButtonKind::Ghost, Some(Message::Cpu(CpuMsg::SaveToProfile))),
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center);
    let note: Element<Message> = if app.controller_ready || oma_hw::helper::Controller::is_root() { iced::widget::Space::new().height(0.0).into() } else { widgets::pill(p, "Helper not installed — changes will fail. See Settings.", p.warn) };
    let epp_row: Element<Message> = if info.has_epp {
        let row = chips("Energy performance preference", &epp_options, edit.epp.as_deref().unwrap_or(""), CpuMsg::Epp);
        if oma_hw::knowledge::epp_pinned_by_governor(&edit.governor) {
            column![row, widgets::dim(p, "The performance governor holds EPP at performance; the others take effect under powersave.")].spacing(space::XS).into()
        } else {
            row
        }
    } else {
        iced::widget::Space::new().height(0.0).into()
    };
    let control = widgets::card(
        p,
        Column::new()
            .spacing(space::LG)
            .push(row![widgets::title(p, "Power & scheduling"), widgets::hfill(), if dirty { widgets::pill(p, "unsaved", p.warn) } else { widgets::pill(p, "live", p.ok) }].align_y(iced::Alignment::Center))
            .push(note)
            .push(chips("Governor", &info.available_governors, &edit.governor, CpuMsg::Governor))
            .push(epp_row)
            .push(toggles)
            .push(limits)
            .push(widgets::vfill())
            .push(actions)
            .height(Length::Fill),
    )
    .width(Length::Fill);

    // --- Per-core grid -----------------------------------------------------
    let cores: Vec<Element<Message>> = snap
        .map(|s| {
            s.cpu
                .freq_mhz
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let u = s.cpu.util.get(i).copied().unwrap_or(0.0) as f32 / 100.0;
                    let rank = info.prefcore_ranking.get(i).copied();
                    let best = info.prefcore_ranking.iter().max().copied().unwrap_or(0);
                    let is_pref = rank.is_some_and(|r| r == best);
                    column![
                        row![widgets::dim(p, format!("C{i:02}")), widgets::hfill(), if is_pref { widgets::pill(p, "★", p.accent_2) } else { iced::widget::Space::new().into() }].align_y(iced::Alignment::Center),
                        widgets::mono(p, format!("{f:.0}"), size::LEAD),
                        widgets::bar(p, u, theme::mix(p.cpu, p.accent, u)),
                    ]
                    .spacing(space::XS)
                    .width(Length::Fixed(74.0))
                    .into()
                })
                .collect()
        })
        .unwrap_or_default();
    let core_card = widgets::card(
        p,
        column![
            row![widgets::title(p, "Cores"), widgets::hfill(), widgets::dim(p, "★ preferred core · MHz · load")].align_y(iced::Alignment::Center),
            scrollable(Row::with_children(cores).spacing(space::SM).wrap()).height(Length::Fill),
        ]
        .spacing(space::MD)
        .height(Length::Fill),
    )
    .width(Length::Fill);

    let history = widgets::card(
        p,
        column![
            row![widgets::eyebrow(p, "Package temperature"), widgets::hfill(), widgets::mono(p, format!("{:.1} °C", snap.and_then(|s| s.cpu.tctl_c).unwrap_or(0.0)), size::SMALL)],
            canvas(Sparkline { palette: p, data: &app.hist.cpu_temp, min: 30.0, max: 95.0, color: p.accent, capacity: crate::app::HISTORY }).width(Length::Fill).height(Length::Fill),
        ]
        .spacing(space::SM)
        .height(Length::Fill),
    )
    .width(Length::Fill);

    // Fixed composition: header, then controls beside telemetry; only the core grid scrolls internally.
    let right = column![history.height(Length::FillPortion(2)), core_card.height(Length::FillPortion(5))].spacing(space::LG).width(Length::FillPortion(6)).height(Length::Fill);
    column![
        header,
        row![control.width(Length::FillPortion(5)).height(Length::Fill), right].spacing(space::LG).height(Length::Fill),
    ]
    .spacing(space::LG)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn pretty(s: &str) -> String {
    match s {
        "balance_performance" => "Balance → performance".into(),
        "balance_power" => "Balance → power".into(),
        _ => {
            let mut c = s.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        }
    }
}

pub fn toggle<'a>(p: theme::Palette, label: &str, on: bool, enabled: bool, mk: impl Fn(bool) -> Message + 'a) -> Element<'a, Message> {
    let t = iced::widget::toggler(on).size(22.0).on_toggle_maybe(enabled.then_some(mk)).style(move |_theme, status| {
        let active = matches!(status, iced::widget::toggler::Status::Active { is_toggled: true } | iced::widget::toggler::Status::Hovered { is_toggled: true });
        iced::widget::toggler::Style {
            background: iced::Background::Color(if active { p.accent } else { p.glass_strong }),
            background_border_width: 0.0,
            background_border_color: iced::Color::TRANSPARENT,
            foreground: iced::Background::Color(if active { iced::Color::WHITE } else { p.text_dim }),
            foreground_border_width: 0.0,
            foreground_border_color: iced::Color::TRANSPARENT,
            border_radius: Some(999.0.into()),
            padding_ratio: 0.15,
            text_color: None,
        }
    });
    row![t, widgets::body(p, label)].spacing(space::SM).align_y(iced::Alignment::Center).into()
}

pub fn slider_style(p: theme::Palette) -> impl Fn(&iced::Theme, slider::Status) -> slider::Style {
    move |_, status| {
        let hovered = !matches!(status, slider::Status::Active);
        slider::Style {
            rail: slider::Rail { backgrounds: (iced::Background::Color(p.accent), iced::Background::Color(p.glass_strong)), width: 6.0, border: iced::Border { radius: 3.0.into(), ..Default::default() } },
            handle: slider::Handle { shape: slider::HandleShape::Circle { radius: if hovered { 9.0 } else { 7.0 } }, background: iced::Background::Color(iced::Color::WHITE), border_width: 2.0, border_color: p.accent },
        }
    }
}
