//! Graphics page: NVIDIA power limit, clock lock/offsets, fan policy,
//! live telemetry; amdgpu performance level.

use crate::app::{App, Message};
use crate::pages::cpu::{slider_style, toggle};
use crate::theme::{self, size, space};
use crate::widgets::{self, gauge::Gauge, sparkline::Sparkline};
use iced::widget::{canvas, column, row, scrollable, slider, Column, Row};
use iced::{Element, Length};

#[derive(Debug, Clone)]
pub enum GpuMsg {
    PowerLimit(f64),
    LockClocks(bool),
    LockMax(f64),
    GpcOffset(f64),
    MemOffset(f64),
    FanManual(bool),
    FanPercent(f64),
    Persistence(bool),
    AmdLevel(String),
    Apply,
    Revert,
    SaveToProfile,
}

pub fn view(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    let snap = app.snapshot.as_ref();
    let nv = snap.and_then(|s| s.nvidia.clone());
    let Some(info) = app.nvidia_info.as_ref() else {
        return amd_only(app);
    };
    let edit = &app.gpu_edit;
    let dirty = app.gpu_dirty;

    let temp = nv.as_ref().and_then(|n| n.temp_c).unwrap_or(0) as f32;
    let util = nv.as_ref().and_then(|n| n.util_gpu).unwrap_or(0) as f32;
    let power = nv.as_ref().and_then(|n| n.power_w).unwrap_or(0.0) as f32;
    let limit = nv.as_ref().and_then(|n| n.power_limit_w).unwrap_or(info.power_default_mw as f64 / 1000.0) as f32;
    let clk = nv.as_ref().and_then(|n| n.graphics_mhz).unwrap_or(0);
    let mclk = nv.as_ref().and_then(|n| n.memory_mhz).unwrap_or(0);
    let vram = nv.as_ref().and_then(|n| n.vram_used_mb).unwrap_or(0);

    let header = Row::new().spacing(space::XL).align_y(iced::Alignment::Center)
        .push(column![
            widgets::eyebrow(p, "Graphics"),
            widgets::headline(p, info.name.replace("NVIDIA ", "")),
            widgets::dim(p, format!("driver {} · {} MiB · PCIe {}x{} · {} fans", info.driver_version, info.vram_total_mb, nv.as_ref().and_then(|n| n.pcie_gen).unwrap_or(0), nv.as_ref().and_then(|n| n.pcie_width).unwrap_or(0), info.num_fans)),
        ]
        .spacing(space::XS))
        .push(canvas(Gauge { palette: p, value: temp, min: 0.0, max: 90.0, label: "temp".into(), unit: "°C".into(), color: theme::thermal(&p, temp as f64, 30.0, 85.0), decimals: 0, inner: Some((util / 100.0, p.gpu)) }).width(Length::Fixed(120.0)).height(Length::Fixed(120.0)))
        .push(canvas(Gauge { palette: p, value: power, min: 0.0, max: limit.max(1.0), label: "power".into(), unit: "W".into(), color: p.power, decimals: 0, inner: None }).width(Length::Fixed(120.0)).height(Length::Fixed(120.0)))
        .wrap();

    let stats = Row::new()
        .spacing(space::MD)
        .push(widgets::card(p, widgets::metric(p, "Core clock", clk.to_string(), "MHz", p.gpu)))
        .push(widgets::card(p, widgets::metric(p, "Memory clock", mclk.to_string(), "MHz", p.gpu)))
        .push(widgets::card(p, widgets::metric(p, "VRAM used", format!("{:.1}", vram as f64 / 1024.0), "GiB", p.gpu)))
        .push(widgets::card(p, widgets::metric(p, "Fans", nv.as_ref().map(|n| n.fan_percent.iter().map(|f| format!("{f}%")).collect::<Vec<_>>().join(" / ")).unwrap_or_default(), "", p.fan)))
        .push(widgets::card(p, column![widgets::eyebrow(p, "Limiter"), Row::with_children(nv.as_ref().map(|n| n.throttle_reasons.iter().map(|r| widgets::pill(p, r, if r == "Idle" { p.text_dim } else { p.warn })).collect::<Vec<_>>()).unwrap_or_default()).spacing(space::XS).wrap()].spacing(space::XS)))
        .wrap();

    let pmin = (info.power_min_mw / 1000) as f64;
    let pmax = (info.power_max_mw / 1000) as f64;
    let pdef = (info.power_default_mw / 1000) as f64;
    let pl = edit.power_limit_w.map(|w| w as f64).unwrap_or(pdef);
    let mut ctl = Column::new()
        .spacing(space::LG)
        .push(row![widgets::title(p, "Tuning"), widgets::hfill(), if dirty { widgets::pill(p, "unsaved", p.warn) } else { widgets::pill(p, "live", p.ok) }].align_y(iced::Alignment::Center))
        .push(
            column![
                row![widgets::eyebrow(p, "Power limit"), widgets::hfill(), widgets::mono(p, format!("{pl:.0} W  (default {pdef:.0}, max {pmax:.0})"), size::SMALL)].align_y(iced::Alignment::Center),
                slider(pmin..=pmax, pl, |v| Message::Gpu(GpuMsg::PowerLimit(v))).step(5.0).style(slider_style(p)),
            ]
            .spacing(space::SM),
        );
    let lock_on = edit.locked_graphics_mhz.is_some();
    let lock_max = edit.locked_graphics_mhz.map(|(_, hi)| hi as f64).unwrap_or(info.max_graphics_mhz as f64);
    ctl = ctl.push(
        column![
            toggle(p, "Lock core clock (removes boost variance — steadier frame times)", lock_on, true, |b| Message::Gpu(GpuMsg::LockClocks(b))),
            if lock_on {
                column![
                    row![widgets::eyebrow(p, "Locked ceiling"), widgets::hfill(), widgets::mono(p, format!("{lock_max:.0} MHz"), size::SMALL)],
                    slider(210.0..=info.max_graphics_mhz as f64, lock_max, |v| Message::Gpu(GpuMsg::LockMax(v))).step(15.0).style(slider_style(p)),
                ]
                .spacing(space::SM)
            } else {
                column![]
            },
        ]
        .spacing(space::SM),
    );
    if info.supports_clock_offsets {
        let (glo, ghi) = info.gpc_offset_range_mhz.unwrap_or((-500, 500));
        let (mlo, mhi) = info.mem_offset_range_mhz.unwrap_or((-2000, 3000));
        let g = edit.gpc_offset_mhz.unwrap_or(0) as f64;
        let m = edit.mem_offset_mhz.unwrap_or(0) as f64;
        ctl = ctl.push(
            column![
                row![widgets::eyebrow(p, "Core clock offset"), widgets::hfill(), widgets::mono(p, format!("{g:+.0} MHz"), size::SMALL)],
                slider(glo as f64..=ghi as f64, g, |v| Message::Gpu(GpuMsg::GpcOffset(v))).step(15.0).style(slider_style(p)),
                row![widgets::eyebrow(p, "Memory clock offset"), widgets::hfill(), widgets::mono(p, format!("{m:+.0} MHz"), size::SMALL)],
                slider(mlo as f64..=mhi as f64, m, |v| Message::Gpu(GpuMsg::MemOffset(v))).step(50.0).style(slider_style(p)),
                widgets::dim(p, "Offsets apply to the P0 VF curve. Start small (+100 core / +500 memory) and validate stability; the helper requires admin authentication for these."),
            ]
            .spacing(space::SM),
        );
    }
    let fan_manual = edit.fan_percent.is_some();
    let fan_pct = edit.fan_percent.as_ref().and_then(|v| v.first().copied()).unwrap_or(50) as f64;
    ctl = ctl.push(
        column![
            row![
                toggle(p, "Manual fan speed", fan_manual, info.num_fans > 0, |b| Message::Gpu(GpuMsg::FanManual(b))),
                widgets::hfill(),
                toggle(p, "Persistence mode", edit.persistence.unwrap_or(false), true, |b| Message::Gpu(GpuMsg::Persistence(b))),
            ]
            .align_y(iced::Alignment::Center),
            if fan_manual {
                column![
                    row![widgets::eyebrow(p, "Fan duty"), widgets::hfill(), widgets::mono(p, format!("{fan_pct:.0}%"), size::SMALL)],
                    slider(0.0..=100.0, fan_pct, |v| Message::Gpu(GpuMsg::FanPercent(v))).step(5.0).style(slider_style(p)),
                ]
                .spacing(space::SM)
            } else {
                column![widgets::dim(p, "Automatic: the card's own curve (zero-RPM below ~50 °C). Use the Cooling page for a temperature curve.")]
            },
        ]
        .spacing(space::SM),
    );
    ctl = ctl.push(
        row![
            widgets::btn(p, "Apply", widgets::ButtonKind::Primary, dirty.then_some(Message::Gpu(GpuMsg::Apply))),
            widgets::btn(p, "Revert", widgets::ButtonKind::Ghost, dirty.then_some(Message::Gpu(GpuMsg::Revert))),
            widgets::hfill(),
            widgets::btn(p, "Save into active profile", widgets::ButtonKind::Ghost, Some(Message::Gpu(GpuMsg::SaveToProfile))),
        ]
        .spacing(space::SM),
    );
    let control = widgets::card(p, ctl).width(Length::Fill);

    let history = row![
        widgets::card(p, column![widgets::eyebrow(p, "Temperature"), canvas(Sparkline { palette: p, data: &app.hist.gpu_temp, min: 25.0, max: 90.0, color: p.accent, capacity: crate::app::HISTORY }).width(Length::Fill).height(Length::Fixed(80.0))].spacing(space::SM)).width(Length::Fill),
        widgets::card(p, column![widgets::eyebrow(p, "Load"), canvas(Sparkline { palette: p, data: &app.hist.gpu_load, min: 0.0, max: 100.0, color: p.gpu, capacity: crate::app::HISTORY }).width(Length::Fill).height(Length::Fixed(80.0))].spacing(space::SM)).width(Length::Fill),
        widgets::card(p, column![widgets::eyebrow(p, "Power"), canvas(Sparkline { palette: p, data: &app.hist.gpu_power, min: 0.0, max: pmax as f32, color: p.power, capacity: crate::app::HISTORY }).width(Length::Fill).height(Length::Fixed(80.0))].spacing(space::SM)).width(Length::Fill),
    ]
    .spacing(space::MD);

    let mut page = column![header, stats, control, history].spacing(space::LG);
    if let Some(a) = amd_card(app) {
        page = page.push(a);
    }
    scrollable(page.padding(iced::Padding::from([0.0, space::XS])).width(Length::Fill)).into()
}

fn amd_only(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    match amd_card(app) {
        Some(c) => scrollable(column![widgets::headline(p, "Graphics"), c].spacing(space::LG)).into(),
        None => widgets::dim(p, "No supported GPU detected."),
    }
}

fn amd_card(app: &App) -> Option<Element<'_, Message>> {
    let p = app.palette;
    let inv = app.inventory.as_ref()?;
    let g = inv.amd_gpus.first()?;
    let t = app.snapshot.as_ref().and_then(|s| s.amd.clone()).unwrap_or_default();
    let level = t.perf_level.clone().unwrap_or_else(|| "auto".into());
    let chips: Vec<Element<Message>> = oma_hw::amdgpu::PERF_LEVELS
        .iter()
        .map(|l| widgets::btn(p, *l, if *l == level { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Gpu(GpuMsg::AmdLevel(l.to_string())))))
        .collect();
    Some(
        widgets::card(
            p,
            column![
                row![widgets::title(p, if g.is_integrated { "Integrated graphics (amdgpu)" } else { "AMD graphics" }), widgets::hfill(), widgets::dim(p, &g.name)].align_y(iced::Alignment::Center),
                row![
                    widgets::metric(p, "Busy", t.busy_percent.map(|b| b.to_string()).unwrap_or_else(|| "–".into()), "%", p.gpu),
                    widgets::metric(p, "Clock", t.sclk_mhz.map(|c| format!("{c:.0}")).unwrap_or_else(|| "–".into()), "MHz", p.gpu),
                    widgets::metric(p, "Temp", t.edge_c.map(|c| format!("{c:.0}")).unwrap_or_else(|| "–".into()), "°C", p.gpu),
                    widgets::metric(p, "Power", t.power_w.map(|c| format!("{c:.0}")).unwrap_or_else(|| "–".into()), "W", p.power),
                ]
                .spacing(space::XL),
                widgets::eyebrow(p, "DPM performance level"),
                Row::with_children(chips).spacing(space::SM).wrap(),
            ]
            .spacing(space::MD),
        )
        .width(Length::Fill)
        .into(),
    )
}
