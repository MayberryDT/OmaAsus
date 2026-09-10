//! Dashboard: hero profile card, thermal gauges, live sparklines, fans.

use crate::app::{App, Message};
use crate::theme::{self, size, space};
use crate::widgets::{self, gauge::Gauge, ridge::Ridge, sparkline::Sparkline};
use iced::widget::{canvas, column, container, responsive, row, Column, Row};
use iced::{Element, Length};

pub fn view(app: &App) -> Element<'_, Message> {
    responsive(move |size| view_inner(app, false, size)).into()
}

/// Narrow layout for the overlay panel.
pub fn view_compact(app: &App) -> Element<'_, Message> {
    responsive(move |size| view_inner(app, true, size)).into()
}

/// Everything below derives its scale from the real viewport, so the page
/// composes itself for any window without scrolling.
fn view_inner(app: &App, compact: bool, size_avail: iced::Size) -> Element<'_, Message> {
    let p = app.palette;
    let snap = app.snapshot.as_ref();
    let h = size_avail.height.max(300.0);
    let w = size_avail.width.max(300.0);
    let hero_pt = if compact { (h * 0.045).clamp(30.0, 44.0) } else { (h * 0.072).clamp(38.0, 84.0) };
    let list_rows = (((h * (if compact { 0.16 } else { 0.22 })) - 40.0) / 26.0).floor().max(2.0) as usize;

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
    let ridge = canvas(Ridge {
        palette: p,
        series: vec![(&app.hist.coolant, 20.0, 50.0, p.coolant), (&app.hist.gpu_temp, 25.0, 90.0, p.gpu), (&app.hist.cpu_temp, 30.0, 95.0, p.accent)],
        capacity: crate::app::HISTORY,
        phase: app.now.duration_since(app.t0).as_secs_f32() * 0.08,
    })
    .width(Length::Fill)
    .height(Length::Fill);
    let name = profile.map(|pr| pr.name.clone()).unwrap_or_else(|| "—".into());
    let cell = (hero_pt / 6.5).clamp(5.0, 13.0);
    let title_block = column![
        widgets::eyebrow(p, "Active profile"),
        widgets::pixel::pixel_label(p, name, cell),
        row![
            widgets::pill(p, mode, p.accent),
            widgets::pill(p, if app.controller_ready { "helper connected" } else { "read-only" }, if app.controller_ready { p.ok } else { p.warn }),
        ]
        .spacing(space::XS),
        widgets::dim(p, &app.inventory_title),
    ]
    .spacing(space::SM);
    let hero_body: Element<Message> = if compact {
        column![title_block, ridge, quick].spacing(space::MD).width(Length::Fill).into()
    } else {
        column![
            row![
                title_block.width(Length::FillPortion(3)),
                column![widgets::eyebrow(p, "Thermal signature"), ridge].spacing(space::SM).width(Length::FillPortion(4)).height(Length::Fill),
            ]
            .spacing(space::XL)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced::Alignment::End),
            quick,
        ]
        .spacing(space::LG)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    };
    let hero = widgets::glow_card(p, hero_body).width(Length::Fill);

    let sm = app.smooth;
    let cpu_t = sm.cpu_t;
    let gpu_t = sm.gpu_t;
    let cool_t = sm.coolant;
    let cpu_load = sm.cpu_load / 100.0;
    let gpu_load = sm.gpu_load / 100.0;
    let gpu_w = sm.gpu_w;
    let gpu_lim = snap.and_then(|s| s.nvidia.as_ref().and_then(|n| n.power_limit_w)).unwrap_or(450.0) as f32;

    let g = move |value: f32, max: f32, label: &str, unit: &str, color, inner: Option<(f32, iced::Color)>| {
        canvas(Gauge { palette: p, value, min: 0.0, max, label: label.into(), unit: unit.into(), color, decimals: 0, inner }).width(Length::Fill).height(Length::Fill)
    };
    let gauges_row = Row::new()
        .spacing(space::LG)
        .push(g(cpu_t, 95.0, "CPU", "°C", theme::thermal(&p, cpu_t as f64, 35.0, 95.0), Some((cpu_load, p.cpu))))
        .push(g(gpu_t, 90.0, "GPU", "°C", theme::thermal(&p, gpu_t as f64, 30.0, 85.0), Some((gpu_load, p.gpu))))
        .push(g(cool_t, 50.0, "Coolant", "°C", theme::thermal(&p, cool_t as f64, 22.0, 45.0), None))
        .push(g(gpu_w, gpu_lim.max(1.0), "GPU power", "W", p.power, None));
    let gauges: Element<Message> = if compact {
        column![
            row![g(cpu_t, 95.0, "CPU", "°C", theme::thermal(&p, cpu_t as f64, 35.0, 95.0), Some((cpu_load, p.cpu))), g(gpu_t, 90.0, "GPU", "°C", theme::thermal(&p, gpu_t as f64, 30.0, 85.0), Some((gpu_load, p.gpu)))].spacing(space::SM).height(Length::Fill),
            row![g(cool_t, 50.0, "Coolant", "°C", theme::thermal(&p, cool_t as f64, 22.0, 45.0), None), g(gpu_w, gpu_lim.max(1.0), "GPU power", "W", p.power, None)].spacing(space::SM).height(Length::Fill),
        ]
        .spacing(space::SM)
        .height(Length::Fill)
        .into()
    } else {
        gauges_row.width(Length::Fill).height(Length::Fill).into()
    };

    // Sparkline tiles: big numeral, caption, live line.
    let spark = |label: &str, data, min, max, color, big: String, unit: &str, caption: String| {
        widgets::card(
            p,
            column![
                widgets::eyebrow(p, label),
                row![
                    iced::widget::text(big).size(size::DISPLAY).font(theme::font::DISPLAY_LIGHT).color(color).line_height(1.0),
                    iced::widget::text(unit.to_string()).size(size::SMALL).font(theme::font::BODY_MEDIUM).color(p.text_dim),
                ]
                .spacing(space::XS)
                .align_y(iced::Alignment::End),
                widgets::dim(p, caption),
                canvas(Sparkline { palette: p, data, min, max, color, capacity: crate::app::HISTORY }).width(Length::Fill).height(Length::Fill),
            ]
            .spacing(space::SM)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
    };
    let cpu_mhz = snap.map(|s| s.cpu.max_core_mhz).unwrap_or(0.0);
    let gpu_mhz = snap.and_then(|s| s.nvidia.as_ref().and_then(|n| n.graphics_mhz)).unwrap_or(0);
    let cpu_w = snap.and_then(|s| s.cpu.package_w);
    let sparks: Element<Message> = if compact {
        column![
            spark("CPU load", &app.hist.cpu_load, 0.0, 100.0, p.cpu, format!("{:.0}", cpu_load * 100.0), "%", format!("{cpu_mhz:.0} MHz")),
            spark("GPU load", &app.hist.gpu_load, 0.0, 100.0, p.gpu, format!("{:.0}", gpu_load * 100.0), "%", format!("{gpu_mhz} MHz")),
        ]
        .spacing(space::MD)
        .height(Length::Fill)
        .into()
    } else {
        row![
            spark("CPU load", &app.hist.cpu_load, 0.0, 100.0, p.cpu, format!("{:.0}", cpu_load * 100.0), "%", format!("{cpu_mhz:.0} MHz")),
            spark("GPU load", &app.hist.gpu_load, 0.0, 100.0, p.gpu, format!("{:.0}", gpu_load * 100.0), "%", format!("{gpu_mhz} MHz")),
            spark("GPU power", &app.hist.gpu_power, 0.0, gpu_lim.max(100.0), p.power, format!("{gpu_w:.0}"), "W", format!("of {gpu_lim:.0} W{}", cpu_w.map(|w| format!(" · CPU {w:.0} W")).unwrap_or_default())),
        ]
        .spacing(space::LG)
        .height(Length::Fill)
        .into()
    };

    let fans: Vec<Element<Message>> = snap
        .map(|s| {
            s.fans
                .iter()
                .filter(|f| f.rpm > 0 || f.label.starts_with("Pump") || f.freshness != crate::telemetry::Freshness::Live)
                .take(list_rows)
                .map(|f| widgets::fan_row(p, f))
                .collect()
        })
        .unwrap_or_default();
    let fans_el: Element<Message> = if fans.is_empty() { widgets::dim(p, "No tachometer signals yet.") } else { Column::with_children(fans).spacing(space::SM).into() };
    let fan_card = widgets::card(p, column![widgets::eyebrow(p, "Fans & pump"), fans_el].spacing(space::MD).height(Length::Fill)).width(Length::FillPortion(3)).height(Length::Fill);

    let temps: Vec<Element<Message>> = snap
        .map(|s| {
            let mut v: Vec<(String, f64)> = Vec::new();
            if let Some(t) = s.vrm_c { v.push(("VRM".into(), t)); }
            if let Some(t) = s.board_c { v.push(("Motherboard".into(), t)); }
            for (i, t) in s.cpu.ccd_c.iter().enumerate() { v.push((format!("CCD{}", i + 1), *t)); }
            for (i, t) in s.nvme_c.iter().enumerate() { v.push((format!("NVMe {}", i + 1), *t)); }
            for (i, t) in s.dimm_c.iter().enumerate() { v.push((format!("DIMM {}", i + 1), *t)); }
            let mut rows: Vec<Element<Message>> = v
                .into_iter()
                .map(|(l, t)| {
                    row![widgets::dim(p, l), widgets::hfill(), iced::widget::text(format!("{t:.0}°")).size(size::BODY).font(theme::font::MONO).color(theme::thermal(&p, t, 30.0, 90.0))]
                        .align_y(iced::Alignment::Center)
                        .into()
                })
                .collect();
            for r in &s.temps {
                rows.push(widgets::temp_row(p, r));
            }
            rows.truncate(list_rows);
            rows
        })
        .unwrap_or_default();
    let temp_card = widgets::card(p, column![widgets::eyebrow(p, "Temperatures"), Column::with_children(temps).spacing(space::XS)].spacing(space::MD).height(Length::Fill)).width(Length::FillPortion(2)).height(Length::Fill);

    if compact {
        let two_col = w > 420.0;
        let lists: Element<Message> = if two_col { row![fan_card.width(Length::FillPortion(3)), temp_card.width(Length::FillPortion(2))].spacing(space::MD).height(Length::Fill).into() } else { fan_card.width(Length::Fill).height(Length::Fill).into() };
        return column![
            hero.height(Length::FillPortion(12)),
            container(gauges).height(Length::FillPortion(10)).width(Length::Fill).center_x(Length::Fill),
            container(sparks).height(Length::FillPortion(9)).width(Length::Fill),
            container(lists).height(Length::FillPortion(8)).width(Length::Fill),
        ]
        .spacing(space::MD)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    }
    // Fixed composition: everything fits the viewport, rows share height proportionally.
    column![
        hero.height(Length::FillPortion(11)),
        container(gauges).height(Length::FillPortion(13)).width(Length::Fill),
        container(sparks).height(Length::FillPortion(9)).width(Length::Fill),
        row![fan_card, temp_card].spacing(space::LG).height(Length::FillPortion(11)),
    ]
    .spacing(space::LG)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
