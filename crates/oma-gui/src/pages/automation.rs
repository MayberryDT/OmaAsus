//! Automation page: manual vs automatic mode, rules that pick a profile when a
//! game runs (GameMode, fullscreen, window class, process), thermal and time
//! triggers; live status of the detection engine.

use crate::app::{App, Message};
use crate::pages::cpu::{slider_style, toggle};
use crate::theme::{self, size, space};
use crate::widgets;
use iced::widget::{column, container, row, scrollable, slider, Column, Row};
use iced::{Element, Length};
use oma_hw::profile::{Mode, Rule, Trigger};

#[derive(Debug, Clone)]
pub enum AutomationMsg {
    Mode(Mode),
    Toggle(uuid::Uuid, bool),
    Delete(uuid::Uuid),
    Add(&'static str),
    SetProfile(uuid::Uuid, uuid::Uuid),
    Priority(uuid::Uuid, f64),
    Hold(uuid::Uuid, f64),
    Text(uuid::Uuid, String),
    Threshold(uuid::Uuid, f64),
    DefaultProfile(uuid::Uuid),
}

pub fn trigger_label(t: &Trigger) -> String {
    match t {
        Trigger::GameMode => "GameMode client registered".into(),
        Trigger::FullscreenGame => "Fullscreen game window focused".into(),
        Trigger::WindowClass(c) => format!("Window class matches “{c}”"),
        Trigger::Process(n) => format!("Process “{n}” running"),
        Trigger::CpuHot { above_c, for_s } => format!("CPU above {above_c:.0} °C for {for_s}s"),
        Trigger::GpuHot { above_c, for_s } => format!("GPU above {above_c:.0} °C for {for_s}s"),
        Trigger::Time { from, to } => format!("Between {:02}:{:02} and {:02}:{:02}", from.0, from.1, to.0, to.1),
        Trigger::Idle { for_s } => format!("Idle for {for_s}s"),
    }
}

pub fn view(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    let auto = app.config.mode == Mode::Automatic;
    let st = &app.auto_state;
    let status_pills: Vec<Element<Message>> = vec![
        widgets::pill(p, if auto { "Automatic" } else { "Manual" }, if auto { p.ok } else { p.text_dim }),
        widgets::pill(p, format!("GameMode: {}", st.gamemode_clients.map(|n| n.to_string()).unwrap_or_else(|| "n/a".into())), if st.gamemode_clients.unwrap_or(0) > 0 { p.ok } else { p.text_dim }),
        widgets::pill(p, format!("Focused: {}{}", if st.window_class.is_empty() { "—" } else { &st.window_class }, if st.fullscreen { " (fullscreen)" } else { "" }), if st.looks_like_game { p.accent } else { p.text_dim }),
        widgets::pill(p, st.matched_rule.clone().map(|r| format!("Rule: {r}")).unwrap_or_else(|| "No rule matched".into()), if st.matched_rule.is_some() { p.accent_2 } else { p.text_dim }),
    ];
    let header = widgets::card(
        p,
        column![
            row![widgets::title(p, "Automation"), widgets::hfill(), widgets::btn(p, "Manual", if !auto { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Automation(AutomationMsg::Mode(Mode::Manual)))), widgets::btn(p, "Automatic", if auto { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Automation(AutomationMsg::Mode(Mode::Automatic))))].spacing(space::SM).align_y(iced::Alignment::Center),
            widgets::dim(p, "In automatic mode the highest-priority matching rule picks the profile; when nothing matches, the default profile is restored after the hold time. Detection uses Feral GameMode, Hyprland's window events, process names and temperatures."),
            Row::with_children(status_pills).spacing(space::SM).wrap(),
            row![
                widgets::eyebrow(p, "Default profile"),
                Row::with_children(app.config.profiles.iter().map(|pr| widgets::btn(p, &pr.name, if pr.id == app.config.default_profile { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Automation(AutomationMsg::DefaultProfile(pr.id))))).collect::<Vec<_>>()).spacing(space::XS).wrap(),
            ]
            .spacing(space::MD)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(space::MD),
    )
    .width(Length::Fill);

    let rules: Vec<Element<Message>> = app.config.rules.iter().map(|r| rule_card(app, r)).collect();
    let add = widgets::card(
        p,
        column![
            widgets::eyebrow(p, "Add a rule"),
            Row::with_children(vec![
                widgets::btn(p, "GameMode", widgets::ButtonKind::Ghost, Some(Message::Automation(AutomationMsg::Add("gamemode")))),
                widgets::btn(p, "Fullscreen game", widgets::ButtonKind::Ghost, Some(Message::Automation(AutomationMsg::Add("fullscreen")))),
                widgets::btn(p, "Window class", widgets::ButtonKind::Ghost, Some(Message::Automation(AutomationMsg::Add("class")))),
                widgets::btn(p, "Process name", widgets::ButtonKind::Ghost, Some(Message::Automation(AutomationMsg::Add("process")))),
                widgets::btn(p, "CPU hot", widgets::ButtonKind::Ghost, Some(Message::Automation(AutomationMsg::Add("cpuhot")))),
                widgets::btn(p, "GPU hot", widgets::ButtonKind::Ghost, Some(Message::Automation(AutomationMsg::Add("gpuhot")))),
                widgets::btn(p, "Night hours", widgets::ButtonKind::Ghost, Some(Message::Automation(AutomationMsg::Add("night")))),
            ])
            .spacing(space::SM)
            .wrap(),
        ]
        .spacing(space::MD),
    )
    .width(Length::Fill);

    let rules_el: Element<Message> = if rules.is_empty() { widgets::card(p, widgets::dim(p, "No rules yet — add one below.")).into() } else { scrollable(Column::with_children(rules).spacing(space::MD)).height(Length::Fill).into() };
    column![header, container(rules_el).height(Length::Fill), add].spacing(space::LG).height(Length::Fill).into()
}

fn rule_card<'a>(app: &'a App, r: &'a Rule) -> Element<'a, Message> {
    let p = app.palette;
    let id = r.id;
    let profile_chips = Row::with_children(app.config.profiles.iter().map(|pr| widgets::btn(p, &pr.name, if pr.id == r.profile { widgets::ButtonKind::Primary } else { widgets::ButtonKind::Ghost }, Some(Message::Automation(AutomationMsg::SetProfile(id, pr.id))))).collect::<Vec<_>>()).spacing(space::XS).wrap();
    let text_edit: Element<Message> = match &r.trigger {
        Trigger::WindowClass(v) | Trigger::Process(v) => iced::widget::text_input("pattern", v)
            .on_input(move |s| Message::Automation(AutomationMsg::Text(id, s)))
            .font(theme::font::MONO)
            .size(size::BODY)
            .width(Length::Fixed(260.0))
            .style(move |_, _| iced::widget::text_input::Style { background: iced::Background::Color(p.glass), border: iced::Border { color: p.line_strong, width: 1.0, radius: theme::radius::SM.into() }, icon: p.text_dim, placeholder: p.text_faint, value: p.text, selection: p.accent_soft })
            .into(),
        Trigger::CpuHot { above_c, .. } | Trigger::GpuHot { above_c, .. } => column![row![widgets::eyebrow(p, "Threshold"), widgets::hfill(), widgets::mono(p, format!("{above_c:.0} °C"), size::SMALL)], slider(50.0..=100.0, *above_c, move |v| Message::Automation(AutomationMsg::Threshold(id, v))).step(1.0).style(slider_style(p))].spacing(space::XS).width(Length::Fixed(260.0)).into(),
        _ => iced::widget::Space::new().into(),
    };
    let matched = app.auto_state.matched_rule.as_deref() == Some(r.name.as_str());
    widgets::card(
        p,
        column![
            row![
                toggle(p, "", r.enabled, true, move |b| Message::Automation(AutomationMsg::Toggle(id, b))),
                column![widgets::body(p, &r.name), widgets::dim(p, trigger_label(&r.trigger))].spacing(2.0).width(Length::Fill),
                if matched { widgets::pill(p, "matching now", p.ok) } else { iced::widget::Space::new().into() },
                widgets::btn(p, "Remove", widgets::ButtonKind::Danger, Some(Message::Automation(AutomationMsg::Delete(id)))),
            ]
            .spacing(space::MD)
            .align_y(iced::Alignment::Center),
            row![widgets::eyebrow(p, "Profile"), profile_chips].spacing(space::MD).align_y(iced::Alignment::Center),
            row![
                text_edit,
                column![row![widgets::eyebrow(p, "Priority"), widgets::hfill(), widgets::mono(p, r.priority.to_string(), size::SMALL)], slider(0.0..=200.0, r.priority as f64, move |v| Message::Automation(AutomationMsg::Priority(id, v))).step(10.0).style(slider_style(p))].spacing(space::XS).width(Length::Fixed(200.0)),
                column![row![widgets::eyebrow(p, "Hold after clear"), widgets::hfill(), widgets::mono(p, format!("{}s", r.hold_s), size::SMALL)], slider(0.0..=300.0, r.hold_s as f64, move |v| Message::Automation(AutomationMsg::Hold(id, v))).step(5.0).style(slider_style(p))].spacing(space::XS).width(Length::Fixed(200.0)),
            ]
            .spacing(space::LG)
            .wrap(),
        ]
        .spacing(space::MD),
    )
    .width(Length::Fill)
    .into()
}
