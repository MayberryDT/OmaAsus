//! Dashboard: hero profile card, thermal gauges, live sparklines, fans.

use crate::app::{App, Message};
use crate::theme::{self, size, space};
use crate::widgets::{self, gauge::Gauge, ridge::Ridge, sparkline::Sparkline};
use iced::widget::{canvas, column, container, responsive, row, Column, Row};
use iced::{Element, Length};

pub fn view(app: &App) -> Element<'_, Message> {
    responsive(move |size| view_inner(app, false, size)).into()
}

fn gauge<'a>(p: theme::Palette, value: f32, max: f32, label: &str, unit: &str, color: iced::Color, inner: Option<(f32, iced::Color)>) -> Element<'a, Message> {
    canvas(Gauge { palette: p, value, min: 0.0, max, label: label.into(), unit: unit.into(), color, decimals: 0, inner }).width(Length::Fill).height(Length::Fill).into()
}

/// A round scale a little above the most power seen: never less than 25 W.
fn power_scale(peak: f32) -> f32 {
    ((peak * 1.25).max(25.0) / 5.0).ceil() * 5.0
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
    let has_coolant = app.model.as_ref().is_some_and(|m| m.sensor_for(oma_hw::model::SensorRole::Coolant).is_some()) || snap.is_some_and(|s| s.coolant_c.is_some());
    let mut series = vec![(&app.hist.gpu_temp, 25.0, 90.0, p.gpu), (&app.hist.cpu_temp, 30.0, 95.0, p.accent)];
    if has_coolant {
        series.insert(0, (&app.hist.coolant, 20.0, 50.0, p.coolant));
    }
    let ridge = canvas(Ridge {
        palette: p,
        series,
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
        apply_status(app),
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
    let cpu_load = sm.cpu_load / 100.0;
    let gpu_load = sm.gpu_load / 100.0;
    let gpu = snap.and_then(|s| s.gpu());
    let gpu_name = if gpu.is_some_and(|g| !g.discrete) { "iGPU" } else { "GPU" };
    // Power: an awake discrete GPU's against the limit it reports, else the
    // package's against the most it has drawn.
    let nvidia = snap.is_some_and(|s| s.nvidia.is_some());
    let nv_limit = snap.and_then(|s| s.nvidia.as_ref().and_then(|n| n.power_limit_w)).map(|l| l as f32);
    let (power_name, power_max) = match (nvidia, nv_limit) {
        (true, Some(limit)) => ("GPU power", limit),
        (true, None) => ("GPU power", power_scale(sm.power_w)),
        (false, _) => ("Power", power_scale(sm.power_peak)),
    };
    let fastest = snap.and_then(|s| s.fans.iter().filter(|f| f.freshness == crate::telemetry::Freshness::Live).max_by_key(|f| f.rpm));
    let fan_max = fastest.map(|f| f.max_rpm.unwrap_or((f.peak_rpm as f64 * 1.1) as u64).max(1000) as f32).unwrap_or(1000.0);

    // Gauges for what this machine has.
    let mut dials: Vec<Element<Message>> = vec![gauge(p, sm.cpu_t, 95.0, "CPU", "°C", theme::thermal(&p, sm.cpu_t as f64, 35.0, 95.0), Some((cpu_load, p.cpu)))];
    if gpu.is_some() {
        dials.push(gauge(p, sm.gpu_t, 90.0, gpu_name, "°C", theme::thermal(&p, sm.gpu_t as f64, 30.0, 85.0), Some((gpu_load, p.gpu))));
    }
    if has_coolant {
        dials.push(gauge(p, sm.coolant, 50.0, "Coolant", "°C", theme::thermal(&p, sm.coolant as f64, 22.0, 45.0), None));
    } else if fastest.is_some() {
        dials.push(gauge(p, sm.fan_rpm, fan_max, "Fans", "rpm", p.fan, None));
    }
    dials.push(gauge(p, sm.power_w, power_max.max(1.0), power_name, "W", p.power, None));
    let gauges: Element<Message> = if compact {
        let mut rows = Column::new().spacing(space::SM).height(Length::Fill);
        let mut it = dials.into_iter();
        while let Some(a) = it.next() {
            let mut pair = Row::new().spacing(space::SM).height(Length::Fill).push(a);
            if let Some(b) = it.next() {
                pair = pair.push(b);
            }
            rows = rows.push(pair);
        }
        rows.into()
    } else {
        Row::with_children(dials).spacing(space::LG).width(Length::Fill).height(Length::Fill).into()
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
    let gpu_mhz = snap.and_then(|s| s.nvidia.as_ref().and_then(|n| n.graphics_mhz).map(f64::from).or_else(|| s.amd.as_ref().and_then(|a| a.sclk_mhz))).unwrap_or(0.0);
    let cpu_w = snap.and_then(|s| s.cpu.package_w);
    let gpu_label = format!("{gpu_name} load");
    let mut tiles: Vec<Element<Message>> = vec![spark("CPU load", &app.hist.cpu_load, 0.0, 100.0, p.cpu, widgets::fmt0(cpu_load * 100.0), "%", format!("{cpu_mhz:.0} MHz")).into()];
    if gpu.is_some() {
        tiles.push(spark(&gpu_label, &app.hist.gpu_load, 0.0, 100.0, p.gpu, widgets::fmt0(gpu_load * 100.0), "%", format!("{gpu_mhz:.0} MHz")).into());
    }
    if !compact {
        let caption = match (nvidia, nv_limit) {
            (true, Some(limit)) => format!("of {limit:.0} W{}", cpu_w.map(|w| format!(" · CPU {w:.0} W")).unwrap_or_default()),
            (true, None) => "discrete GPU".into(),
            (false, _) => "package".into(),
        };
        tiles.push(spark(power_name, if nvidia { &app.hist.gpu_power } else { &app.hist.power }, 0.0, power_max.max(1.0), p.power, widgets::fmt0(sm.power_w), "W", caption).into());
    }
    let sparks: Element<Message> = if compact { Column::with_children(tiles).spacing(space::MD).height(Length::Fill).into() } else { Row::with_children(tiles).spacing(space::LG).height(Length::Fill).into() };

    let fans: Vec<Element<Message>> = snap
        .map(|s| {
            s.fans
                .iter()
                .take(list_rows)
                .map(|f| widgets::fan_row(p, f))
                .collect()
        })
        .unwrap_or_default();
    let fans_el: Element<Message> = if fans.is_empty() { widgets::dim(p, "No tachometer signals yet.") } else { Column::with_children(fans).spacing(space::SM).into() };
    let fan_title = if snap.is_some_and(|s| s.pump_rpm.is_some()) { "Fans & pump" } else { "Fans" };
    let fan_card = widgets::card(p, column![widgets::eyebrow(p, fan_title), fans_el].spacing(space::MD).height(Length::Fill)).width(Length::FillPortion(3)).height(Length::Fill);

    let temps: Vec<Element<Message>> = snap
        .map(|s| {
            let mut v: Vec<(String, f64)> = Vec::new();
            if let Some(t) = s.vrm_c { v.push(("VRM".into(), t)); }
            if let Some(t) = s.board_c { v.push(("Motherboard".into(), t)); }
            for (i, t) in s.cpu.ccd_c.iter().enumerate() { v.push((format!("CCD{}", i + 1), *t)); }
            for (i, t) in s.nvme_c.iter().enumerate() { v.push((format!("Drive {}", i + 1), *t)); }
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

/// What the last profile apply did, or what is applying now: the requested
/// profile is the name above; this is whether the hardware followed.
fn apply_status(app: &App) -> Element<'_, Message> {
    use crate::coordinator::Status;
    let p = app.palette;
    let Some(status) = app.coord.status(app.now) else { return iced::widget::Space::new().height(0.0).into() };
    let age = |d: std::time::Duration| if d.as_secs() < 60 { format!("{}s ago", d.as_secs()) } else { format!("{}m ago", d.as_secs() / 60) };
    match status {
        Status::Applying { name, then } => {
            let text = match then {
                Some(next) => format!("applying {name}, then {next}"),
                None => format!("applying {name}…"),
            };
            row![widgets::pill(p, text, p.accent)].into()
        }
        Status::Ok { applied, skipped, age: a, .. } => {
            let mut line = format!("{applied} settings in place · {}", age(a));
            if !skipped.is_empty() {
                line.push_str(&format!(" · skipped {}", skipped.join(", ")));
            }
            widgets::dim(p, line)
        }
        Status::Failed { id, failed, applied, age: a, .. } => column![
            row![widgets::pill(p, format!("{} failed", failed.len()), p.danger), widgets::dim(p, format!("{applied} in place · {}", age(a))), widgets::btn(p, "Retry", widgets::ButtonKind::Ghost, Some(Message::ApplyProfile(id)))].spacing(space::XS).align_y(iced::Alignment::Center),
            widgets::dim(p, failed.join(" · ")),
        ]
        .spacing(space::XS)
        .into(),
    }
}
