//! Dashboard: hero profile card, thermal gauges, live sparklines, fans.

use crate::app::{App, Message};
use crate::theme::{self, size, space};
use crate::widgets::{self, gauge::Gauge, sparkline::Sparkline};
use iced::widget::{canvas, column, row, scrollable, Column, Row};
use iced::{Element, Length};

pub fn view(app: &App) -> Element<'_, Message> {
    view_inner(app, false)
}

/// Narrow layout for the overlay panel.
pub fn view_compact(app: &App) -> Element<'_, Message> {
    view_inner(app, true)
}

fn view_inner(app: &App, compact: bool) -> Element<'_, Message> {
    let p = app.palette;
    let snap = app.snapshot.as_ref();

    let profile = app.active_profile();
    let mode = match app.config.mode {
        oma_hw::profile::Mode::Manual => "Manual",
        oma_hw::profile::Mode::Automatic => "Automatic",
    };
    let quick = Row::with_children(
        app.config
            .profiles
            .iter()
            .map(|pr| {
                let active = pr.id == app.config.active_profile;
                widgets::btn(p, &pr.name, if active { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::ApplyProfile(pr.id)))
            })
            .collect::<Vec<_>>(),
    )
    .spacing(space::SM)
    .wrap();
    let hero = widgets::glow_card(
        p,
        column![
            row![
                column![
                    widgets::eyebrow(p, "Active profile"),
                    widgets::headline(p, profile.map(|pr| pr.name.clone()).unwrap_or_else(|| "—".into())),
                ]
                .spacing(space::XS),
                widgets::hfill(),
                column![
                    widgets::pill(p, mode, p.accent),
                    widgets::pill(p, if app.controller_ready { "helper connected" } else { "read-only" }, if app.controller_ready { p.ok } else { p.warn }),
                ]
                .spacing(space::XS)
                .align_x(iced::Alignment::End),
            ]
            .align_y(iced::Alignment::Start),
            widgets::dim(p, &app.inventory_title),
            quick,
        ]
        .spacing(space::MD),
    )
    .width(Length::Fill);

    let cpu_t = snap.and_then(|s| s.cpu.tctl_c).unwrap_or(0.0) as f32;
    let gpu_t = snap.and_then(|s| s.nvidia.as_ref().and_then(|n| n.temp_c)).map(|t| t as f32).unwrap_or(0.0);
    let cool_t = snap.and_then(|s| s.coolant_c).unwrap_or(0.0) as f32;
    let cpu_load = snap.map(|s| s.cpu.util_total as f32 / 100.0).unwrap_or(0.0);
    let gpu_load = snap.and_then(|s| s.nvidia.as_ref().and_then(|n| n.util_gpu)).map(|u| u as f32 / 100.0).unwrap_or(0.0);
    let gpu_w = snap.and_then(|s| s.nvidia.as_ref().and_then(|n| n.power_w)).unwrap_or(0.0) as f32;
    let gpu_lim = snap.and_then(|s| s.nvidia.as_ref().and_then(|n| n.power_limit_w)).unwrap_or(450.0) as f32;

    let gsize = if compact { 112.0 } else { 150.0 };
    let g = move |value: f32, max: f32, label: &str, unit: &str, color, inner: Option<(f32, iced::Color)>| {
        canvas(Gauge { palette: p, value, min: 0.0, max, label: label.into(), unit: unit.into(), color, decimals: 0, inner })
            .width(Length::Fixed(gsize))
            .height(Length::Fixed(gsize))
    };
    let gauges = Row::new()
        .spacing(space::LG)
        .push(g(cpu_t, 95.0, "CPU", "°C", theme::thermal(&p, cpu_t as f64, 35.0, 95.0), Some((cpu_load, p.cpu))))
        .push(g(gpu_t, 90.0, "GPU", "°C", theme::thermal(&p, gpu_t as f64, 30.0, 85.0), Some((gpu_load, p.gpu))))
        .push(g(cool_t, 50.0, "Coolant", "°C", theme::thermal(&p, cool_t as f64, 22.0, 45.0), None))
        .push(g(gpu_w, gpu_lim.max(1.0), "GPU power", "W", p.power, None))
        .wrap();

    let spark = |label: &str, data, min, max, color, value: String| {
        widgets::card(
            p,
            column![
                row![widgets::eyebrow(p, label), widgets::hfill(), widgets::mono(p, value, size::SMALL)].align_y(iced::Alignment::Center),
                canvas(Sparkline { palette: p, data, min, max, color, capacity: crate::app::HISTORY }).width(Length::Fill).height(Length::Fixed(64.0)),
            ]
            .spacing(space::SM),
        )
        .width(Length::Fill)
    };
    let cpu_mhz = snap.map(|s| s.cpu.max_core_mhz).unwrap_or(0.0);
    let gpu_mhz = snap.and_then(|s| s.nvidia.as_ref().and_then(|n| n.graphics_mhz)).unwrap_or(0);
    let sparks: Element<Message> = if compact {
        column![
            spark("CPU", &app.hist.cpu_load, 0.0, 100.0, p.cpu, format!("{:.0}% · {:.0} MHz", cpu_load * 100.0, cpu_mhz)),
            spark("GPU", &app.hist.gpu_load, 0.0, 100.0, p.gpu, format!("{:.0}% · {} MHz · {gpu_w:.0} W", gpu_load * 100.0, gpu_mhz)),
        ]
        .spacing(space::MD)
        .into()
    } else {
        row![
        spark("CPU load", &app.hist.cpu_load, 0.0, 100.0, p.cpu, format!("{:.0}% · {:.0} MHz", cpu_load * 100.0, cpu_mhz)),
        spark("GPU load", &app.hist.gpu_load, 0.0, 100.0, p.gpu, format!("{:.0}% · {} MHz", gpu_load * 100.0, gpu_mhz)),
        spark("Power", &app.hist.gpu_power, 0.0, gpu_lim.max(100.0), p.power, format!("{gpu_w:.0} W{}", snap.and_then(|s| s.cpu.package_w).map(|w| format!(" · {w:.0} W CPU")).unwrap_or_default())),
    ]
    .spacing(space::LG)
    .into()
    };

    let fans: Vec<Element<Message>> = snap
        .map(|s| {
            s.fans
                .iter()
                .filter(|f| f.rpm > 0 || f.label.starts_with("Pump"))
                .map(|f| {
                    let duty = f.duty.unwrap_or(0.0) as f32 / 100.0;
                    let frac = if f.duty.is_some() { duty } else { (f.rpm as f32 / 2400.0).min(1.0) };
                    row![
                        column![widgets::body(p, &f.label), widgets::dim(p, &f.device)].spacing(2.0).width(Length::FillPortion(3)),
                        widgets::bar(p, frac, p.fan),
                        widgets::mono(p, format!("{:>5} rpm", f.rpm), size::SMALL),
                        widgets::mono(p, f.duty.map(|d| format!("{d:>3.0}%")).unwrap_or_else(|| "  — ".into()), size::SMALL),
                    ]
                    .spacing(space::MD)
                    .align_y(iced::Alignment::Center)
                    .into()
                })
                .collect()
        })
        .unwrap_or_default();
    let fan_card = widgets::card(p, column![widgets::eyebrow(p, "Fans & pump"), Column::with_children(fans).spacing(space::SM)].spacing(space::MD)).width(Length::FillPortion(3));

    let temps: Vec<Element<Message>> = snap
        .map(|s| {
            let mut v: Vec<(String, f64)> = Vec::new();
            if let Some(t) = s.vrm_c { v.push(("VRM".into(), t)); }
            if let Some(t) = s.board_c { v.push(("Motherboard".into(), t)); }
            for (i, t) in s.cpu.ccd_c.iter().enumerate() { v.push((format!("CCD{}", i + 1), *t)); }
            for (i, t) in s.nvme_c.iter().enumerate() { v.push((format!("NVMe {}", i + 1), *t)); }
            for (i, t) in s.dimm_c.iter().enumerate() { v.push((format!("DIMM {}", i + 1), *t)); }
            for r in &s.temps { v.push((r.label.clone(), r.value)); }
            v.into_iter()
                .map(|(l, t)| {
                    row![widgets::dim(p, l), widgets::hfill(), iced::widget::text(format!("{t:.0}°")).size(size::BODY).font(theme::font::MONO).color(theme::thermal(&p, t, 30.0, 90.0))]
                        .align_y(iced::Alignment::Center)
                        .into()
                })
                .collect()
        })
        .unwrap_or_default();
    let temp_card = widgets::card(p, column![widgets::eyebrow(p, "Temperatures"), Column::with_children(temps).spacing(space::XS)].spacing(space::MD)).width(Length::FillPortion(2));

    let bottom: Element<Message> = if compact { column![fan_card.width(Length::Fill), temp_card.width(Length::Fill)].spacing(space::LG).into() } else { row![fan_card, temp_card].spacing(space::LG).into() };
    scrollable(
        column![hero, gauges, sparks, bottom]
            .spacing(space::LG)
            .padding(iced::Padding::from([0.0, space::XS]))
            .width(Length::Fill),
    )
    .into()
}
