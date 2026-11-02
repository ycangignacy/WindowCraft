//! The three-column workspace and its seven modules.

use crate::app::*;
use crate::icons::Icon;
use crate::press::press;
use crate::theme::*;
use crate::ui::*;
use iced::widget::{Column, Row, column, container, mouse_area, opaque, row, space, stack, text_input};
use iced::{Alignment, Border, Color, Length, Padding, mouse};
use windowcraft_config::{FeatureKind, Tweak, TweakGroup, TweakKind};
use windowcraft_engine::Detection;
use windowcraft_engine::services::is_protected;
use windowcraft_engine::autoruns::EntrySource;
use windowcraft_engine::gaming::InterruptPriority;
use windowcraft_engine::packages::PackageManager;
use windowcraft_engine::snapshot::{Change, SessionState, unix_now};

/// Wider than the visible hairline, so the edge is easy to grab.
const DRAWER_GRIP: f32 = 14.0;
const LOG_LINES_SHOWN: usize = 40;

pub fn view(app: &App) -> El<'_> {
    let p = app.palette();
    let drawer_width = app.timeline.channels[DRAWER].value().max(0.0) as f32;

    // Only the center and drawer paint opaque surfaces. The sidebar stays
    // translucent so the window backdrop shows through it.
    let workspace = container(center(app, p))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::background(p.base));
    let mut columns = Row::new().push(sidebar(app, p)).push(workspace);
    if drawer_width > 0.5 {
        columns = columns.push(
            container(inspector(app, p))
                .width(drawer_width)
                .height(Length::Fill)
                .clip(true)
                .style(move |_| container::background(p.surface)),
        );
    }

    let mut layers = stack![container(columns).width(Length::Fill).height(Length::Fill)];
    if app.confirm.is_some() {
        layers = layers.push(confirm_dialog(app, p));
    }
    layers.into()
}

fn sidebar(app: &App, p: Palette) -> El<'_> {
    const ICONS: [Icon; 7] = [
        Icon::Apps,
        Icon::Tweaks,
        Icon::Wrench,
        Icon::Refresh,
        Icon::Disc,
        Icon::Clock,
        Icon::Gauge,
    ];

    let elevation: El<'_> = if app.info.elevated {
        badge(p, "Administrator", p.green)
    } else {
        press(badge(p, "Standard user", p.amber)).on_press(Message::Relaunch).into()
    };
    let header = column![
        strong("WindowCraft", p.ink),
        mono(format!("Build {}", app.info.full_build()), p.ink_tertiary),
        caption("Made by ycangignacy", p.ink_tertiary),
        space().height(6),
        elevation,
    ]
    .spacing(1);

    let indicator_y = app.timeline.channels[NAV].value() as f32 * NAV_ROW_HEIGHT;
    let highlight = container(
        container(space().width(Length::Fill).height(NAV_ROW_HEIGHT - 2.0)).style(move |_| container::Style {
            background: Some(p.selected.into()),
            border: Border { radius: CONTROL_RADIUS.into(), ..Border::default() },
            ..container::Style::default()
        }),
    )
    .padding(Padding { top: indicator_y + 1.0, ..Padding::ZERO });

    let mut items = Column::new();
    for (page, icon) in Page::ALL.into_iter().zip(ICONS) {
        let active = app.page == page;
        let ink = if active { p.ink } else { p.ink_secondary };
        items = items.push(
            press(
                container(row![icon.view(16.0, ink), caption(page.title(), ink)].spacing(10).align_y(Alignment::Center))
                    .width(Length::Fill)
                    .height(NAV_ROW_HEIGHT)
                    .padding([0.0, 10.0])
                    .center_y(NAV_ROW_HEIGHT),
            )
            .on_press(Message::Navigate(page)),
        );
    }

    let appearance_icon = if p.dark { Icon::Moon } else { Icon::Sun };
    container(column![
        container(header).padding(Padding { top: 18.0, right: 14.0, bottom: 16.0, left: 14.0 }),
        container(stack![items].push_under(highlight)).padding([0.0, 8.0]),
        space().height(Length::Fill),
        container(
            row![icon_button(p, appearance_icon, Message::CycleAppearance), mono(concat!("v", env!("CARGO_PKG_VERSION")), p.ink_tertiary)]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .padding(10),
    ])
    .width(SIDEBAR_WIDTH)
    .height(Length::Fill)
    .style(move |_| container::background(p.sidebar))
    .into()
}

fn center(app: &App, p: Palette) -> El<'_> {
    let search = text_input("Search apps, tweaks, registry keys, services and fixes", &app.query)
        .id(search_id())
        .on_input(Message::Query)
        .size(13)
        .font(SANS)
        .padding([6.0, 10.0])
        .width(Length::Fixed(360.0))
        .style(move |_, _| text_input::Style {
            background: p.elevated.into(),
            border: Border { color: p.border, width: 1.0, radius: CONTROL_RADIUS.into() },
            icon: p.ink_secondary,
            placeholder: p.ink_tertiary,
            value: p.ink,
            selection: Color { a: 0.35, ..p.accent },
        });

    let toolbar = container(
        column![
            row![heading(app.page.title(), p.ink), space().width(Length::Fill), search, mono("Ctrl+K", p.ink_tertiary)]
                .spacing(10)
                .align_y(Alignment::Center),
            page_toolbar(app, p),
        ]
        .spacing(10),
    )
    .padding(Padding { top: 16.0, right: 22.0, bottom: 8.0, left: 22.0 });

    let body: El<'_> = if !app.query.trim().is_empty() {
        search_results(app, p)
    } else {
        match app.page {
            Page::Applications => applications(app, p),
            Page::Tweaks if app.tab == TweakTab::Appx => appx_page(app, p),
            Page::Tweaks => tweaks(app, p),
            Page::System => system(app, p),
            Page::Updates => updates(app, p),
            Page::Iso => iso_studio(app, p),
            Page::TimeMachine => time_machine(app, p),
            Page::Inspector => inspector_page(app, p),
        }
    };

    let mut main = Column::new().push(toolbar);
    if let Some(summary) = windowcraft_engine::Engine::drift_summary(&app.drift)
        && app.page != Page::TimeMachine
    {
        main = main.push(
            container(banner(p, Icon::Alert, p.amber, summary, vec![("Re-apply", Message::RepairDrift)]))
                .padding(Padding { right: 22.0, left: 22.0, bottom: 6.0, ..Padding::ZERO }),
        );
    }
    if let Some(notice) = &app.notice {
        let mut actions = vec![("Dismiss", Message::DismissNotice)];
        if app.needs_elevation {
            actions.insert(0, ("Restart as Administrator", Message::Relaunch));
        }
        main = main.push(
            container(banner(p, Icon::Check, p.ink_secondary, notice.clone(), actions))
                .padding(Padding { right: 22.0, left: 22.0, bottom: 6.0, ..Padding::ZERO }),
        );
    }
    main = main.push(body);

    let mut layers = stack![main.width(Length::Fill).height(Length::Fill)];
    let dock_progress = app.timeline.channels[DOCK].value() as f32;
    if dock_progress > 0.01 {
        layers = layers.push(dock(app, p, dock_progress));
    }
    layers.width(Length::Fill).height(Length::Fill).into()
}

fn banner<'a>(p: Palette, icon: Icon, color: Color, content: String, actions: Vec<(&'a str, Message)>) -> El<'a> {
    let mut line = row![icon.view(15.0, color), body(content, p.ink).width(Length::Fill)]
        .spacing(10)
        .align_y(Alignment::Center);
    for (label, message) in actions {
        line = line.push(button(p, label, None, Tone::Secondary, Some(message)));
    }
    container(line)
        .padding([6.0, 12.0])
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(p.surface.into()),
            border: Border { color: p.border, width: 1.0, radius: GROUP_RADIUS.into() },
            ..container::Style::default()
        })
        .into()
}

fn page_toolbar(app: &App, p: Palette) -> El<'_> {
    match app.page {
        Page::Applications => {
            let mut chips = Row::new().spacing(6);
            for category in app.catalog.app_categories() {
                let active = app.categories.contains(category);
                chips = chips.push(chip(p, category.to_owned(), active, Message::Category(category.to_owned())));
            }
            let manager = tabs(
                p,
                vec!["WinGet", "Chocolatey"],
                if app.manager == PackageManager::Winget { 0.0 } else { 1.0 },
                84.0,
                |index| Message::Manager(if index == 0 { PackageManager::Winget } else { PackageManager::Chocolatey }),
            );
            column![
                row![
                    manager,
                    button(
                        p,
                        if app.checking_upgrades { "Checking Versions" } else { "Show Installed" },
                        Some(Icon::Eye),
                        Tone::Secondary,
                        (!app.checking_upgrades).then_some(Message::ShowInstalled),
                    ),
                    button(p, "Upgrade All", Some(Icon::Refresh), Tone::Secondary, app.job.is_none().then_some(Message::UpgradeAll)),
                    button(p, "Clear Selection", None, Tone::Secondary, Some(Message::ClearApps)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                iced::widget::scrollable(chips)
                    .direction(iced::widget::scrollable::Direction::Horizontal(
                        iced::widget::scrollable::Scrollbar::new().width(0).scroller_width(0),
                    ))
                    .width(Length::Fill),
            ]
            .spacing(8)
            .into()
        }
        Page::Tweaks => {
            let bar = tabs(
                p,
                TweakTab::ALL.iter().map(|tab| tab.title()).collect(),
                app.timeline.channels[TAB].value(),
                104.0,
                |index| Message::Tab(TweakTab::ALL[index]),
            );
            let mut controls = row![bar].spacing(8).align_y(Alignment::Center);
            if app.tab != TweakTab::Appx {
                controls = controls
                    .push(button(p, "Minimal", None, Tone::Secondary, Some(Message::Preset(windowcraft_config::PresetName::Minimal))))
                    .push(button(p, "Standard", None, Tone::Secondary, Some(Message::Preset(windowcraft_config::PresetName::Standard))))
                    .push(button(p, "Advanced", None, Tone::Secondary, Some(Message::Preset(windowcraft_config::PresetName::Advanced))))
                    .push(button(p, "Detect Applied", None, Tone::Secondary, Some(Message::DetectApplied)))
                    .push(button(p, "Clear", None, Tone::Secondary, Some(Message::ClearTweaks)));
            } else {
                controls = controls.push(button(p, "Default Removals", None, Tone::Secondary, Some(Message::AppxDefaults)));
            }
            // Wraps instead of clipping when the inspector narrows this column.
            controls
                .push(button(p, "Import", None, Tone::Secondary, Some(Message::ImportProfile)))
                .push(button(p, "Export", None, Tone::Secondary, Some(Message::ExportProfile)))
                .wrap()
                .vertical_spacing(8)
                .into()
        }
        Page::Inspector => tabs(
            p,
            InspectorTab::ALL.iter().map(|tab| tab.title()).collect(),
            app.timeline.channels[TAB].value(),
            100.0,
            |index| Message::InspectorTab(InspectorTab::ALL[index]),
        ),
        _ => space().height(0).into(),
    }
}

fn dock<'a>(app: &'a App, p: Palette, progress_value: f32) -> El<'a> {
    let content: El<'a> = if let Some(job) = &app.job {
        column![
            row![
                strong(job.label.as_str(), p.ink).width(Length::Fill),
                mono(format!("{:>3.0}%", job.fraction * 100.0), p.ink_secondary),
                button(p, "Cancel", Some(Icon::Stop), Tone::Secondary, Some(Message::CancelJob)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            progress(p, job.fraction),
            mono(job.step.as_str(), p.ink_tertiary),
        ]
        .spacing(5)
        .into()
    } else {
        let (count, noun, actions): (usize, &str, Vec<El<'a>>) = match app.page {
            Page::Applications => (
                app.selected_apps.len(),
                "applications selected",
                vec![
                    button(p, "Uninstall", Some(Icon::Trash), Tone::Destructive, Some(Message::UninstallApps)),
                    button(p, "Install / Upgrade", Some(Icon::Download), Tone::Primary, Some(Message::InstallApps)),
                ],
            ),
            Page::Tweaks if app.tab == TweakTab::Appx => (
                app.selected_appx.len(),
                "packages selected",
                vec![
                    button(p, "Install", Some(Icon::Download), Tone::Secondary, Some(Message::InstallAppx)),
                    button(p, "Remove", Some(Icon::Trash), Tone::Destructive, Some(Message::RemoveAppx)),
                ],
            ),
            Page::System => (
                app.selected_features.len(),
                "features selected",
                vec![button(p, "Install Features", Some(Icon::Download), Tone::Primary, Some(Message::InstallFeatures))],
            ),
            _ => (
                app.selected_tweaks.len(),
                "tweaks queued",
                vec![button(p, "Run Tweaks", Some(Icon::Play), Tone::Primary, Some(Message::ApplyTweaks))],
            ),
        };
        let mut bar = row![
            mono(count.to_string(), p.ink),
            caption(noun, p.ink_secondary).width(Length::Fill)
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        for action in actions {
            bar = bar.push(action);
        }
        bar.into()
    };

    let lift = (1.0 - progress_value) * 90.0;
    container(
        container(opaque(content))
            .width(560)
            .padding([10.0, 14.0])
            .style(move |_| container::Style {
                background: Some(p.elevated.into()),
                border: Border { color: p.specular, width: 1.0, radius: GROUP_RADIUS.into() },
                shadow: floating_shadow(),
                ..container::Style::default()
            }),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Alignment::Center)
    .align_y(Alignment::End)
    .padding(Padding { bottom: 16.0 - lift, ..Padding::ZERO })
    .into()
}

fn search_results(app: &App, p: Palette) -> El<'_> {
    let rows: Vec<El<'_>> = app
        .hits
        .iter()
        .map(|hit| {
            list_row(
                p,
                None,
                hit.title.as_str(),
                hit.detail.as_str(),
                badge(p, hit.kind.label(), p.ink_secondary),
                Some(Message::SearchSelected(hit.kind, hit.id.clone())),
            )
        })
        .collect();
    let content: El<'_> = if rows.is_empty() {
        body("No matches.", p.ink_secondary).into()
    } else {
        group(p, rows, ROW_INSET)
    };
    scroll_area(p, content)
}

fn applications(app: &App, p: Palette) -> El<'_> {
    let mut sections = Column::new().spacing(16);
    for category in app.catalog.app_categories() {
        if !app.categories.is_empty() && !app.categories.contains(category) {
            continue;
        }
        let mut rows: Vec<El<'_>> = Vec::new();
        for item in app.catalog.apps.iter().filter(|item| item.category == category) {
            let selected = app.selected_apps.contains(&item.id);
            let package = match app.manager {
                PackageManager::Winget => item.winget.as_deref(),
                PackageManager::Chocolatey => item.choco.as_deref(),
            };
            let mut trailing = Row::new().spacing(8).align_y(Alignment::Center);
            if let Some(installed) = app.installed.get(&item.id) {
                let latest = item.winget.as_ref().and_then(|id| app.upgrades.get(id));
                let version = match latest {
                    Some(latest) => format!("{installed} -> {latest}"),
                    None => installed.clone(),
                };
                trailing = trailing.push(dot(if latest.is_some() { p.amber } else { p.green }));
                trailing = trailing.push(mono(version, p.ink_secondary));
            }
            if item.foss {
                trailing = trailing.push(badge(p, "FOSS", p.ink_tertiary));
            }
            trailing = trailing.push(icon_button(p, Icon::Eye, Message::Inspect(Inspected::App(item.id.clone()))));
            rows.push(list_row(
                p,
                Some(check(p, selected, Message::AppToggled(item.id.clone()))),
                item.name.as_str(),
                package.unwrap_or("not available from this source"),
                trailing.into(),
                Some(Message::AppToggled(item.id.clone())),
            ));
        }
        sections = sections.push(group_titled(p, category, rows, 41.0));
    }
    scroll_area(p, sections)
}

fn status_color(p: Palette, detection: Option<&Detection>) -> Color {
    match detection {
        Some(Detection::Applied) => p.green,
        Some(Detection::Partial { .. }) => p.amber,
        _ => p.ink_tertiary,
    }
}

fn tweak_detail(tweak: &Tweak) -> String {
    tweak.registry.first().map_or_else(
        || {
            tweak.services.first().map_or_else(
                || tweak.apply.first().map_or_else(String::new, windowcraft_config::NativeOp::describe),
                |rule| format!("Service {} -> {}", rule.name, rule.startup.label()),
            )
        },
        |entry| {
            let more = tweak.registry.len() - 1;
            let value = entry.value.as_deref().unwrap_or("per option");
            let suffix = if more > 0 { format!("  +{more}") } else { String::new() };
            format!("{}\\{} = {value}{suffix}", entry.path, entry.name)
        },
    )
}

fn tweak_row<'a>(app: &'a App, p: Palette, tweak: &'a Tweak) -> El<'a> {
    let detection = app.detections.get(&tweak.id);
    let inspect = icon_button(p, Icon::Eye, Message::Inspect(Inspected::Tweak(tweak.id.clone())));
    let id = tweak.id.clone();

    let (leading, control, on_press): (Option<El<'a>>, El<'a>, Option<Message>) = match tweak.kind {
        TweakKind::Check => {
            let selected = app.selected_tweaks.contains(&tweak.id);
            (
                Some(check(p, selected, Message::TweakChecked(id.clone()))),
                space().width(0).into(),
                Some(Message::TweakChecked(id)),
            )
        }
        TweakKind::Toggle => {
            let on = detection == Some(&Detection::Applied);
            (None, switch(p, on, move |value| Message::Toggle(id.clone(), value)), None)
        }
        TweakKind::Segmented => {
            let labels: Vec<String> = tweak.options.iter().map(|option| option.label.clone()).collect();
            let current = app.options.get(&tweak.id).cloned();
            (None, select(p, labels, current, move |value| Message::Segment(id.clone(), value)), None)
        }
        TweakKind::Action => (
            None,
            button(p, "Run", Some(Icon::Play), Tone::Secondary, app.job.is_none().then_some(Message::RunAction(id))),
            None,
        ),
        TweakKind::Dns => {
            let labels: Vec<String> = tweak.options.iter().map(|option| option.label.clone()).collect();
            (None, select(p, labels, Some(app.dns_choice.clone()), Message::Dns), None)
        }
    };

    let mut trailing = Row::new().spacing(10).align_y(Alignment::Center);
    if tweak.group == TweakGroup::Advanced {
        trailing = trailing.push(badge(p, "Caution", p.amber));
    }
    if tweak.is_detectable() {
        trailing = trailing.push(dot(status_color(p, detection)));
    }
    trailing = trailing.push(control).push(inspect);
    list_row(p, leading, tweak.name.as_str(), tweak_detail(tweak), trailing.into(), on_press)
}

fn tweaks(app: &App, p: Palette) -> El<'_> {
    let groups: &[TweakGroup] = match app.tab {
        TweakTab::Essential => &[TweakGroup::Essential],
        TweakTab::Advanced => &[TweakGroup::Advanced],
        _ => &[TweakGroup::Preferences, TweakGroup::Performance],
    };
    let mut sections = Column::new().spacing(16);
    for group_kind in groups {
        let rows: Vec<El<'_>> = app
            .catalog
            .tweaks
            .iter()
            .filter(|tweak| tweak.group == *group_kind)
            .map(|tweak| tweak_row(app, p, tweak))
            .collect();
        sections = sections.push(group_titled(p, group_kind.label(), rows, ROW_INSET));
    }
    scroll_area(p, sections)
}

fn appx_page(app: &App, p: Palette) -> El<'_> {
    let mut sections = Column::new().spacing(16);
    for category in app.catalog.appx_categories() {
        let rows: Vec<El<'_>> = app
            .catalog
            .appx
            .iter()
            .filter(|package| package.category == category)
            .map(|package| {
                let selected = app.selected_appx.contains(&package.id);
                let mut trailing = Row::new().spacing(8).align_y(Alignment::Center);
                if package.warning.is_some() {
                    trailing = trailing.push(badge(p, "Dependency", p.amber));
                }
                trailing = trailing.push(icon_button(p, Icon::Eye, Message::Inspect(Inspected::Appx(package.id.clone()))));
                list_row(
                    p,
                    Some(check(p, selected, Message::AppxChecked(package.id.clone()))),
                    package.name.as_str(),
                    package.package_id.as_str(),
                    trailing.into(),
                    Some(Message::AppxChecked(package.id.clone())),
                )
            })
            .collect();
        sections = sections.push(group_titled(p, category, rows, 41.0));
    }
    scroll_area(p, sections)
}

fn system(app: &App, p: Palette) -> El<'_> {
    let features = |kind: FeatureKind| app.catalog.features.iter().filter(move |feature| feature.kind == kind);

    let feature_rows: Vec<El<'_>> = features(FeatureKind::Feature)
        .filter(|feature| !feature.features.is_empty() || !feature.apply.is_empty() || !feature.registry.is_empty())
        .map(|feature| {
            let selected = app.selected_features.contains(&feature.id);
            let detail = if feature.features.is_empty() { feature.description.clone() } else { feature.features.join(", ") };
            list_row(
                p,
                Some(check(p, selected, Message::FeatureChecked(feature.id.clone()))),
                feature.name.as_str(),
                detail,
                icon_button(p, Icon::Eye, Message::Inspect(Inspected::Feature(feature.id.clone()))),
                Some(Message::FeatureChecked(feature.id.clone())),
            )
        })
        .collect();

    let action_rows = |kind: FeatureKind| -> Vec<El<'_>> {
        features(kind)
            .map(|feature| {
                let mut trailing = Row::new().spacing(8).align_y(Alignment::Center);
                if !feature.options.is_empty() {
                    let labels: Vec<String> = feature.options.iter().map(|option| option.label.clone()).collect();
                    let current = app.fix_modes.get("update").cloned().unwrap_or_else(|| "Standard".to_owned());
                    trailing = trailing.push(select(p, labels, Some(current), Message::FixMode));
                }
                trailing = trailing.push(button(
                    p,
                    "Run",
                    Some(Icon::Play),
                    Tone::Secondary,
                    app.job.is_none().then(|| Message::RunFix(feature.id.clone())),
                ));
                list_row(p, None, feature.name.as_str(), feature.description.as_str(), trailing.into(), None)
            })
            .collect()
    };

    let panel_rows: Vec<El<'_>> = features(FeatureKind::Panel)
        .map(|feature| {
            let target = feature.target.clone().unwrap_or_default();
            list_row(
                p,
                None,
                feature.name.as_str(),
                target.clone(),
                Icon::Chevron.view(14.0, p.ink_tertiary).into(),
                Some(Message::Launch(target)),
            )
        })
        .collect();

    let mut remote = action_rows(FeatureKind::Profile);
    remote.extend(action_rows(FeatureKind::Remote));

    scroll_area(
        p,
        column![
            group_titled(p, "Windows Optional Features", feature_rows, 41.0),
            group_titled(p, "One-Click Fixes", action_rows(FeatureKind::Fix), ROW_INSET),
            group_titled(p, "PowerShell Profile and OpenSSH", remote, ROW_INSET),
            group_titled(p, "Classic Windows Panels", panel_rows, ROW_INSET),
        ]
        .spacing(16),
    )
}

fn updates(app: &App, p: Palette) -> El<'_> {
    let rows: Vec<El<'_>> = app
        .catalog
        .updates
        .iter()
        .map(|mode| {
            let active = app.update_mode == mode.id;
            let trailing: El<'_> = if active {
                badge(p, "Active", p.green)
            } else {
                let tone = if mode.confirm { Tone::Destructive } else { Tone::Secondary };
                button(p, "Apply", None, tone, app.job.is_none().then(|| Message::UpdateMode(mode.id.clone())))
            };
            press(
                container(
                    row![
                        column![strong(mode.name.as_str(), p.ink), caption(mode.description.as_str(), p.ink_secondary)]
                            .spacing(2)
                            .width(Length::Fill),
                        trailing
                    ]
                    .spacing(16)
                    .align_y(Alignment::Center),
                )
                .padding([12.0, ROW_INSET]),
            )
            .into()
        })
        .collect();
    scroll_area(p, group(p, rows, ROW_INSET))
}

fn iso_option<'a>(p: Palette, title: &'a str, detail: &'a str, flag: IsoFlag, value: bool) -> El<'a> {
    list_row(p, None, title, detail, switch(p, value, move |on| Message::IsoFlag(flag, on)), None)
}

fn input<'a>(p: Palette, placeholder: &'a str, value: &'a str, on_input: fn(String) -> Message) -> El<'a> {
    text_input(placeholder, value)
        .on_input(on_input)
        .size(12)
        .font(MONO)
        .padding([6.0, 10.0])
        .style(move |_, _| text_input::Style {
            background: p.elevated.into(),
            border: Border { color: p.border, width: 1.0, radius: CONTROL_RADIUS.into() },
            icon: p.ink_secondary,
            placeholder: p.ink_tertiary,
            value: p.ink,
            selection: Color { a: 0.35, ..p.accent },
        })
        .into()
}

fn iso_studio(app: &App, p: Palette) -> El<'_> {
    let iso = &app.iso;
    let ready = app.job.is_none();

    let source = container(
        column![
            row![
                input(p, r"C:\Downloads\Win11.iso", &iso.path, Message::IsoPath),
                button(p, "Browse", Some(Icon::Folder), Tone::Secondary, Some(Message::IsoBrowse)),
                button(p, "Open", None, Tone::Primary, (ready && !iso.path.is_empty()).then_some(Message::IsoOpen)),
            ]
            .spacing(8),
            row![
                input(p, "Direct download URL from microsoft.com/software-download/windows11", &iso.url, Message::IsoDownloadUrl),
                input(p, "Expected SHA-256 (optional)", &iso.hash, Message::IsoDownloadHash),
                button(p, "Download", Some(Icon::Download), Tone::Secondary, (ready && !iso.url.is_empty()).then_some(Message::IsoDownload)),
            ]
            .spacing(8),
        ]
        .spacing(8),
    )
    .padding(ROW_INSET);

    let editions: Vec<El<'_>> = iso
        .images
        .iter()
        .map(|image| {
            let active = iso.selected == Some(image.index);
            list_row(
                p,
                Some(check(p, active, Message::IsoEdition(image.index))),
                image.name.as_str(),
                format!("Index {}  {}  Build {}", image.index, image.architecture, image.build),
                space().width(0).into(),
                Some(Message::IsoEdition(image.index)),
            )
        })
        .collect();

    let options = vec![
        iso_option(p, "Bypass hardware checks", "TPM 2.0, Secure Boot, CPU, RAM and storage", IsoFlag::Bypass, iso.options.bypass_requirements),
        iso_option(p, "Local account setup", "BypassNRO, hides online account screens", IsoFlag::LocalAccount, iso.options.local_account),
        iso_option(p, "Dark mode by default", "OOBE Themes", IsoFlag::DarkMode, iso.options.dark_mode),
        iso_option(p, "Disable device encryption", "PreventDeviceEncryption = 1", IsoFlag::NoEncryption, iso.options.disable_device_encryption),
        iso_option(p, "Pause updates during OOBE", "NoAutoUpdate, removed at first boot", IsoFlag::PauseUpdates, iso.options.pause_updates_during_oobe),
        iso_option(p, "Inject system drivers", "SCSIAdapter and HDC into boot.wim, the rest into install.wim", IsoFlag::InjectDrivers, iso.inject_drivers),
        iso_option(p, "Bundle selected applications", "Embeds a WindowCraft profile for first boot", IsoFlag::BundleApps, iso.bundle_apps),
    ];

    let configured = !iso.images.is_empty();
    let mut export_rows: Vec<El<'_>> = vec![
        container(
            row![
                input(p, r"C:\Output\Windows11.iso", &iso.output, Message::IsoOutput),
                button(p, "Browse", Some(Icon::Folder), Tone::Secondary, Some(Message::IsoBrowseOutput)),
                button(p, "Build ISO", Some(Icon::Disc), Tone::Primary, (ready && configured && !iso.output.is_empty()).then_some(Message::IsoBuild)),
            ]
            .spacing(8),
        )
        .padding(ROW_INSET)
        .into(),
        list_row(
            p,
            None,
            "Write to USB drive",
            "GPT, one FAT32 partition, install.wim split above 3.8 GB",
            button(p, "Find Drives", None, Tone::Secondary, Some(Message::IsoRefreshDisks)),
            None,
        ),
    ];
    for disk in &iso.disks {
        export_rows.push(list_row(
            p,
            None,
            disk.model.as_str(),
            format!("Disk {}  {} GB  {}", disk.number, disk.bytes >> 30, disk.bus.label()),
            button(p, "Erase and Write", Some(Icon::Alert), Tone::Destructive, (ready && configured).then_some(Message::IsoWriteUsb(disk.number))),
            None,
        ));
    }

    let mut sections = column![group_titled(p, "1. Source", vec![source.into()], ROW_INSET)].spacing(16);
    if !editions.is_empty() {
        sections = sections.push(group_titled(p, "2. Edition", editions, 41.0));
    }
    sections = sections.push(group_titled(p, "3. Configuration", options, ROW_INSET));
    sections = sections.push(group_titled(p, "4. Export", export_rows, ROW_INSET));
    scroll_area(p, sections)
}

fn relative_time(unix: u64) -> String {
    let seconds = unix_now().saturating_sub(unix);
    match seconds {
        0..=59 => "Just now".to_owned(),
        60..=3_599 => format!("{} min ago", seconds / 60),
        3_600..=86_399 => format!("{} h ago", seconds / 3_600),
        _ => format!("{} d ago", seconds / 86_400),
    }
}

fn change_row<'a>(p: Palette, view: &'a ChangeView) -> El<'a> {
    let (before, after) = match &view.record.change {
        Change::Registry { before, after, .. } => (
            before.as_ref().map_or_else(|| "(not set)".to_owned(), windowcraft_engine::RegValue::display),
            after.as_ref().map_or_else(|| "(removed)".to_owned(), windowcraft_engine::RegValue::display),
        ),
        Change::Service { before, after, .. } => {
            (before.map_or_else(|| "(absent)".to_owned(), |value| value.label().to_owned()), after.label().to_owned())
        }
        Change::Task { before, after, .. } => (before.to_string(), after.to_string()),
        Change::Native { .. } => (String::new(), String::new()),
    };
    let reversible = !matches!(view.record.change, Change::Native { .. }) && !view.record.rolled_back;
    let trailing: El<'a> = if view.record.rolled_back {
        badge(p, "Rolled back", p.ink_tertiary)
    } else if reversible {
        icon_button(p, Icon::Undo, Message::RollbackChange(view.record.session, view.record.index))
    } else {
        space().width(0).into()
    };
    let drifted = !view.live.is_empty() && view.live != after && !view.record.rolled_back;
    let line = if view.live.is_empty() {
        before.clone()
    } else {
        format!("{before}  ->  {after}  ->  live {}", view.live)
    };
    container(
        row![
            column![mono(view.record.change.target(), p.ink), mono(line, if drifted { p.amber } else { p.ink_tertiary })]
                .spacing(2)
                .width(Length::Fill),
            trailing,
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .height(52)
    .padding([0.0, ROW_INSET])
    .center_y(52)
    .into()
}

fn time_machine(app: &App, p: Palette) -> El<'_> {
    let mut sections = Column::new().spacing(16);
    if let Some(summary) = windowcraft_engine::Engine::drift_summary(&app.drift) {
        sections = sections.push(banner(p, Icon::Alert, p.amber, summary, vec![("Re-apply", Message::RepairDrift)]));
        let rows: Vec<El<'_>> = app
            .drift
            .iter()
            .flat_map(|drift| {
                drift.entries.iter().map(move |(target, expected, live)| {
                    list_row(p, None, drift.tweak_name.as_str(), format!("{target}: expected {expected}, found {live}"), dot(p.amber), None)
                })
            })
            .collect();
        sections = sections.push(group_titled(p, "Configuration Drift", rows, ROW_INSET));
    }

    if app.sessions.is_empty() {
        sections = sections.push(body("No changes recorded yet. Every apply creates a snapshot here.", p.ink_secondary));
    } else {
        let mut rows: Vec<El<'_>> = Vec::new();
        for session in &app.sessions {
            let state_color = match session.state {
                SessionState::Committed => p.green,
                SessionState::RolledBack => p.ink_tertiary,
                SessionState::Failed | SessionState::Open | SessionState::PartlyRolledBack => p.amber,
            };
            let mut trailing = row![dot(state_color), caption(session.state.label(), p.ink_secondary)]
                .spacing(6)
                .align_y(Alignment::Center);
            // An interrupted session is exactly what the write-ahead record is for.
            if matches!(session.state, SessionState::Committed | SessionState::Open) {
                trailing = trailing.push(button(
                    p,
                    "Roll Back",
                    Some(Icon::Undo),
                    Tone::Secondary,
                    app.job.is_none().then_some(Message::Ask(Confirm::RollbackSession(session.id))),
                ));
            }
            rows.push(list_row(
                p,
                None,
                session.label.as_str(),
                format!("Session {}  {}", session.id, relative_time(session.created_unix)),
                trailing.into(),
                Some(Message::SelectSession(session.id)),
            ));
            if app.open_session == Some(session.id) {
                let height = app.timeline.channels[ACCORDION].value() as f32;
                let mut changes = Column::new();
                for view in &app.changes {
                    changes = changes.push(change_row(p, view));
                }
                rows.push(container(changes).height(height).clip(true).width(Length::Fill).into());
            }
        }
        sections = sections.push(group_titled(p, "Snapshot Timeline", rows, ROW_INSET));
    }
    sections = sections.push(button(p, "Export Environment Report", Some(Icon::Terminal), Tone::Secondary, app.job.is_none().then_some(Message::ExportReport)));
    if !app.last_log.is_empty() {
        let mut lines = Column::new().spacing(2);
        for line in app.last_log.iter().rev().take(LOG_LINES_SHOWN).rev() {
            lines = lines.push(mono(line.as_str(), p.ink_secondary));
        }
        let log: El<'_> = container(lines).padding(ROW_INSET).width(Length::Fill).into();
        sections = sections.push(group_titled(p, app.last_log_label.as_str(), vec![log], ROW_INSET));
        sections = sections.push(caption("Full history: %LocalAppData%\\WindowCraft\\windowcraft.log", p.ink_tertiary));
    }
    scroll_area(p, sections)
}

fn inspector_page(app: &App, p: Palette) -> El<'_> {
    let content: El<'_> = match app.inspector_tab {
        InspectorTab::Interrupts => interrupts(app, p),
        InspectorTab::Display => display(app, p),
        InspectorTab::Autoruns => autoruns_view(app, p),
        InspectorTab::Cleaner => cleaner_view(app, p),
        InspectorTab::Dns => dns_view(app, p),
    };
    scroll_area(p, content)
}

fn interrupts(app: &App, p: Palette) -> El<'_> {
    if app.devices.is_empty() {
        return body("No supported PCIe devices were found.", p.ink_secondary).into();
    }
    let rows: Vec<El<'_>> = app
        .devices
        .iter()
        .enumerate()
        .map(|(index, device)| {
            let trailing = row![
                badge(p, device.class.label(), p.ink_secondary),
                select(p, InterruptPriority::ALL.iter().map(|value| value.label().to_owned()).collect(), Some(device.priority.label().to_owned()), move |label: String| {
                    let priority = InterruptPriority::ALL.into_iter().find(|value| value.label() == label).unwrap_or(InterruptPriority::Undefined);
                    Message::SetPriority(index, priority)
                }),
                switch(p, device.msi_enabled, move |on| Message::SetMsi(index, on)),
            ]
            .spacing(10)
            .align_y(Alignment::Center);
            list_row(p, None, device.name.as_str(), device.instance.as_str(), trailing.into(), None)
        })
        .collect();
    group_titled(p, "PCIe Devices. Changes apply after a restart.", rows, ROW_INSET)
}

fn display(app: &App, p: Palette) -> El<'_> {
    let toggles: Vec<El<'_>> = app
        .display_toggles
        .iter()
        .map(|tweak| {
            let on = app.detections.get(&tweak.id) == Some(&Detection::Applied);
            let id = tweak.id.clone();
            list_row(p, None, tweak.name.as_str(), tweak.description.as_str(), switch(p, on, move |value| Message::Toggle(id.clone(), value)), None)
        })
        .collect();
    let timer: Vec<El<'_>> = app.timer.map_or_else(Vec::new, |timer| {
        vec![
            list_row(p, None, "Current", "NtQueryTimerResolution", mono(format!("{:>7.3} ms", timer.current_ms), p.ink).into(), None),
            list_row(p, None, "Finest", "Maximum resolution", mono(format!("{:>7.3} ms", timer.finest_ms), p.ink).into(), None),
            list_row(p, None, "Coarsest", "Minimum resolution", mono(format!("{:>7.3} ms", timer.coarsest_ms), p.ink).into(), None),
        ]
    });
    column![group_titled(p, "GPU and Display", toggles, ROW_INSET), group_titled(p, "Kernel Timer", timer, ROW_INSET)].spacing(16).into()
}

fn autoruns_view(app: &App, p: Palette) -> El<'_> {
    if app.autoruns.is_empty() {
        return body("Scanning startup entries.", p.ink_secondary).into();
    }
    let mut sections = Column::new().spacing(16);
    for source in [EntrySource::RunKey, EntrySource::StartupFolder, EntrySource::Task, EntrySource::Service] {
        let rows: Vec<El<'_>> = app
            .autoruns
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.source == source)
            .map(|(index, entry)| {
                let publisher = entry.publisher.clone().unwrap_or_else(|| "Unsigned".to_owned());
                // The engine refuses these anyway, so no control pretends otherwise.
                let control: El<'_> = if entry.source == EntrySource::Service && is_protected(&entry.name) {
                    badge(p, "Protected", p.ink_tertiary)
                } else {
                    switch(p, entry.enabled, move |on| Message::SetAutorun(index, on))
                };
                let trailing = row![dot(if entry.signed { p.green } else { p.amber }), mono(publisher, p.ink_secondary), control]
                    .spacing(8)
                    .align_y(Alignment::Center);
                list_row(p, None, entry.name.as_str(), entry.command.as_str(), trailing.into(), None)
            })
            .collect();
        if !rows.is_empty() {
            sections = sections.push(group_titled(p, source.label(), rows, ROW_INSET));
        }
    }
    sections.into()
}

fn format_bytes(bytes: u64) -> String {
    match bytes {
        0..=1_048_575 => format!("{:>8.1} KB", bytes as f64 / 1_024.0),
        1_048_576..=1_073_741_823 => format!("{:>8.1} MB", bytes as f64 / 1_048_576.0),
        _ => format!("{:>8.2} GB", bytes as f64 / 1_073_741_824.0),
    }
}

fn cleaner_view(app: &App, p: Palette) -> El<'_> {
    let actions = row![
        button(p, "Scan", Some(Icon::Search), Tone::Secondary, Some(Message::CleanScan)),
        button(
            p,
            "Clean Selected",
            Some(Icon::Trash),
            Tone::Destructive,
            (app.job.is_none() && !(app.clean_selected.is_empty() && app.selected_orphans.is_empty()))
                .then_some(Message::CleanRun),
        ),
    ]
    .spacing(8);
    let rows: Vec<El<'_>> = app
        .clean_results
        .iter()
        .map(|(target, result)| {
            list_row(
                p,
                Some(check(p, app.clean_selected.contains(target.id), Message::CleanToggled(target.id))),
                target.name,
                format!("{}  {} files", target.group, result.files),
                mono(format_bytes(result.bytes), p.ink).into(),
                Some(Message::CleanToggled(target.id)),
            )
        })
        .collect();
    let orphans: Vec<El<'_>> = app
        .orphans
        .iter()
        .map(|orphan| {
            list_row(
                p,
                Some(check(p, app.selected_orphans.contains(&orphan.path), Message::OrphanToggled(orphan.path.clone()))),
                orphan.path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
                orphan.path.display().to_string(),
                mono(format_bytes(orphan.bytes), p.ink).into(),
                Some(Message::OrphanToggled(orphan.path.clone())),
            )
        })
        .collect();
    let mut sections = column![actions].spacing(14);
    if !rows.is_empty() {
        sections = sections.push(group(p, rows, 41.0));
    }
    if !orphans.is_empty() {
        sections = sections.push(group_titled(p, "Possibly Orphaned Folders. Review each one: names are not proof.", orphans, 41.0));
    }
    sections.into()
}

fn dns_view(app: &App, p: Palette) -> El<'_> {
    let run = button(p, "Run Benchmark", Some(Icon::Play), Tone::Secondary, (!app.bench_running).then_some(Message::BenchRun));
    let mut rows: Vec<El<'_>> = Vec::new();
    for (index, result) in app.bench.iter().enumerate() {
        let fraction = app.timeline.channels.get(DNS_BARS + index).map_or(0.0, |channel| channel.value() as f32).clamp(0.0, 1.0);
        let figures = result
            .measurements
            .iter()
            .map(|measure| {
                if measure.average_ms.is_finite() {
                    format!("{} {:>6.1} / {:>6.1} / {:>5.1} ms", measure.protocol.label(), measure.min_ms, measure.average_ms, measure.jitter_ms)
                } else {
                    format!("{} failed", measure.protocol.label())
                }
            })
            .collect::<Vec<_>>()
            .join("   ");
        let track = container(
            container(space().height(3).width(Length::FillPortion((fraction * 1000.0) as u16 + 1)))
                .style(move |_| container::background(p.accent)),
        );
        rows.push(
            container(
                column![
                    row![body(result.id.as_str(), p.ink).width(Length::Fill), mono(result.score().map_or_else(|| "no answer".to_owned(), |score| format!("{score:>7.1} ms")), p.ink)],
                    track.width(Length::Fixed(fraction * 520.0)),
                    mono(figures, p.ink_tertiary),
                ]
                .spacing(4),
            )
            .padding([9.0, ROW_INSET])
            .into(),
        );
    }
    let mut sections = column![run, caption("Each protocol shows min / average / jitter.", p.ink_secondary)].spacing(10);
    if !rows.is_empty() {
        sections = sections.push(group(p, rows, ROW_INSET));
    }
    sections.into()
}

fn inspector(app: &App, p: Palette) -> El<'_> {
    let grip = mouse_area(
        container(container(space()).width(1).height(Length::Fill).style(move |_| container::background(p.hairline)))
            .width(DRAWER_GRIP)
            .height(Length::Fill),
    )
    .interaction(mouse::Interaction::ResizingHorizontally)
    .on_enter(Message::GripEnter)
    .on_exit(Message::GripExit)
    .on_press(Message::GripPressed);

    // The extra 12 keeps card borders clear of the overlay scrollbar.
    let mut content = Column::new().spacing(14).width(INSPECTOR_WIDTH - DRAWER_GRIP - 30.0);
    match &app.inspected {
        Some(Inspected::Tweak(id)) => {
            if let Some(tweak) = app.find_tweak(id) {
                content = content.push(strong(tweak.name.clone(), p.ink));
                content = content.push(caption(tweak.description.clone(), p.ink_secondary));
                if let Some(notice) = tweak.notice.clone() {
                    content = content.push(banner(p, Icon::Alert, p.amber, notice, Vec::new()));
                }
                for row_data in &app.inspect_rows {
                    content = content.push(
                        container(
                            column![
                                mono(row_data.target.as_str(), p.ink).wrapping(iced::widget::text::Wrapping::Glyph),
                                compare(p, "Current", row_data.current.as_str(), if row_data.matches { p.green } else { p.ink }),
                                compare(p, "Target", row_data.desired.as_str(), p.ink),
                                compare(p, "Undo", row_data.undo.as_str(), p.ink_secondary),
                                mono(row_data.kind.as_str(), p.ink_tertiary),
                            ]
                            .spacing(3),
                        )
                        .padding(10)
                        .width(Length::Fill)
                        .style(move |_| container::Style {
                            background: Some(p.elevated.into()),
                            border: Border { color: p.border, width: 1.0, radius: GROUP_RADIUS.into() },
                            ..container::Style::default()
                        }),
                    );
                }
                let operations = describe_ops(&tweak.prepare).into_iter().chain(describe_ops(&tweak.apply));
                let mut native = Column::new().spacing(3);
                let mut any = false;
                for operation in operations {
                    any = true;
                    native = native.push(mono(operation, p.ink_secondary));
                }
                if any {
                    content = content.push(section("Native Operations", p.ink_secondary)).push(native);
                }
                // Undo on something never applied would write catalog defaults
                // over whatever this machine really has.
                let state = app.detections.get(&tweak.id);
                let undoable = matches!(state, Some(Detection::Applied | Detection::Partial { .. }))
                    || (state == Some(&Detection::Unknown) && !tweak.undo.is_empty());
                if tweak.irreversible {
                    content = content.push(caption("This change cannot be undone automatically.", p.amber));
                } else if undoable {
                    content = content.push(button(
                        p,
                        "Undo This Tweak",
                        Some(Icon::Undo),
                        Tone::Secondary,
                        app.job.is_none().then(|| Message::Undo(tweak.id.clone())),
                    ));
                }
            }
        }
        Some(Inspected::App(id)) => {
            if let Some(item) = app.catalog.app(id) {
                content = content.push(strong(item.name.as_str(), p.ink));
                content = content.push(caption(item.description.as_str(), p.ink_secondary));
                content = content.push(compare(p, "WinGet", item.winget.as_deref().unwrap_or("not available"), p.ink));
                content = content.push(compare(p, "Chocolatey", item.choco.as_deref().unwrap_or("not available"), p.ink));
                content = content.push(compare(p, "License", if item.foss { "Open source" } else { "Proprietary" }, p.ink));
                content = content.push(mono(item.link.as_str(), p.ink_tertiary));
            }
        }
        Some(Inspected::Feature(id)) => {
            if let Some(feature) = app.catalog.feature(id) {
                content = content.push(strong(feature.name.as_str(), p.ink));
                content = content.push(caption(feature.description.as_str(), p.ink_secondary));
                for name in &feature.features {
                    content = content.push(mono(format!("DismEnableFeature {name}"), p.ink_secondary));
                }
                for entry in &feature.registry {
                    content = content.push(mono(format!("{}\\{} = {}", entry.path, entry.name, entry.value.as_deref().unwrap_or_default()), p.ink_secondary));
                }
                for operation in describe_ops(&feature.apply) {
                    content = content.push(mono(operation, p.ink_secondary));
                }
            }
        }
        Some(Inspected::Appx(id)) => {
            if let Some(package) = app.catalog.appx_package(id) {
                content = content.push(strong(package.name.as_str(), p.ink));
                content = content.push(caption(package.description.as_str(), p.ink_secondary));
                content = content.push(compare(p, "Package", package.package_id.as_str(), p.ink));
                content = content.push(compare(p, "Store ID", package.store_id.as_deref().unwrap_or("none"), p.ink));
                if let Some(warning) = &package.warning {
                    content = content.push(banner(p, Icon::Alert, p.amber, warning.clone(), Vec::new()));
                }
            }
        }
        None => {}
    }

    let header = row![section("X-Ray Inspector", p.ink_secondary).width(Length::Fill), icon_button(p, Icon::Close, Message::CloseInspector)]
        .align_y(Alignment::Center);

    let drawer = row![
        grip,
        container(column![header, iced::widget::scrollable(content).height(Length::Fill)].spacing(10))
            .padding(Padding { top: 14.0, right: 14.0, bottom: 14.0, left: 2.0 })
            .width(INSPECTOR_WIDTH - DRAWER_GRIP)
    ]
    .width(INSPECTOR_WIDTH);
    // A scrollable gives the content its full width while the parent animates.
    // Without it the drawer would re-wrap its text on every frame.
    iced::widget::scrollable(drawer)
        .direction(iced::widget::scrollable::Direction::Horizontal(
            iced::widget::scrollable::Scrollbar::new().width(0).scroller_width(0),
        ))
        .height(Length::Fill)
        .into()
}

fn compare<'a>(p: Palette, label: &'a str, value: &'a str, color: Color) -> El<'a> {
    row![caption(label, p.ink_tertiary).width(72), mono(value, color).wrapping(iced::widget::text::Wrapping::Glyph)].spacing(6).into()
}

fn confirm_dialog(app: &App, p: Palette) -> El<'_> {
    let (title, detail, action): (String, String, &str) = match &app.confirm {
        Some(Confirm::DisableUpdates) => (
            "Disable all Windows Updates?".to_owned(),
            "BITS, wuauserv and UsoSvc stop and SoftwareDistribution is purged. The system stops receiving security fixes.".to_owned(),
            "Disable Updates",
        ),
        Some(Confirm::RollbackSession(id)) => (
            format!("Roll back session {id}?"),
            "Every recorded value returns to its pre-change state. Native operations run their undo step.".to_owned(),
            "Roll Back",
        ),
        Some(Confirm::EraseDisk { number, model }) => (
            format!("Erase disk {number}?"),
            format!("{model} will be repartitioned. All data on it is destroyed. This cannot be undone."),
            "Erase and Write",
        ),
        Some(Confirm::IrreversibleTweak(id)) => (
            "This selection cannot be undone".to_owned(),
            format!("{} has no automatic undo. Continue with the queued tweaks?", app.catalog.tweak(id).map_or(id.as_str(), |tweak| tweak.name.as_str())),
            "Continue",
        ),
        Some(Confirm::QuitDuringJob) => (
            "A job is still running".to_owned(),
            format!(
                "{} stops now, and what it already changed stays changed. A USB drive stopped mid-write has to be written again.",
                app.job.as_ref().map_or("The job", |job| job.label.as_str())
            ),
            "Quit Anyway",
        ),
        None => (String::new(), String::new(), ""),
    };
    let dialog = container(
        column![
            strong(title, p.ink),
            caption(detail, p.ink_secondary),
            row![
                space().width(Length::Fill),
                button(p, "Cancel", None, Tone::Secondary, Some(Message::Answer(false))),
                button(p, action, None, Tone::Destructive, Some(Message::Answer(true))),
            ]
            .spacing(8),
        ]
        .spacing(10),
    )
    .width(420)
    .padding(18)
    .style(move |_| container::Style {
        background: Some(p.elevated.into()),
        border: Border { color: p.specular, width: 1.0, radius: GROUP_RADIUS.into() },
        shadow: floating_shadow(),
        ..container::Style::default()
    });
    opaque(
        container(dialog)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_| container::background(Color { a: 0.5, ..Color::BLACK })),
    )
}
