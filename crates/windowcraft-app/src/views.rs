// Made by WindowCraft Dev Team
//! The three-column workspace and its eight modules.

use crate::app::*;
use crate::i18n::{self, Language, Msg, format_1, format_2, format_3, t};
use crate::settings::CheckInterval;
use windowcraft_engine::convert::{Family, Target};
use windowcraft_engine::tools::Extension;
use windowcraft_engine::youtube::Format;
use crate::icons::Icon;
use crate::press::press;
use crate::theme::*;
use crate::ui::*;
use iced::widget::{Column, Row, column, container, mouse_area, opaque, row, scrollable, space, stack, text_input};
use iced::{Alignment, Border, Color, Length, Padding, mouse};
use windowcraft_config::{Area, CleanScope, Depth, FeatureKind, Goal, SearchKind, Tweak, TweakGroup, TweakKind};
use windowcraft_engine::Detection;
use windowcraft_engine::autoruns::EntrySource;
use windowcraft_engine::gaming::InterruptPriority;
use windowcraft_engine::packages::PackageManager;
use windowcraft_engine::services::is_protected;
use windowcraft_engine::snapshot::{Change, SessionState, unix_now};

/// Wider than the visible hairline, so the edge is easy to grab.
const DRAWER_GRIP: f32 = 14.0;
const LOG_LINES_SHOWN: usize = 40;
/// Marks the extensions whose code is WindowCraft's own.
const OWN_CODE_BLUE: Color = Color::from_rgb8(0x15, 0x77, 0xA8);

/// Sidebar icons, in the order of `Page::ALL`.
const PAGE_ICONS: [Icon; 8] = [
    Icon::Apps,
    Icon::Tweaks,
    Icon::Wrench,
    Icon::Refresh,
    Icon::Disc,
    Icon::Clock,
    Icon::Gauge,
    Icon::Layers,
];

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

    // Layers come after the workspace, so the workspace keeps its place in
    // the tree and its scroll offset whether a tip or a dialog is up or not.
    let mut layers = stack![container(columns).width(Length::Fill).height(Length::Fill)];
    if let Some(area) = app.tip.filter(|_| app.wizard.is_none() && app.confirm.is_none()) {
        // Under the card: a press anywhere else closes the tip and still
        // reaches the page.
        layers = layers.push(crate::outside::outside(Message::TipDismiss));
        layers = layers.push(tip_card(app, p, area, drawer_width));
    }
    if let Some(wizard) = app.wizard {
        layers = layers.push(wizard_view(app, p, wizard));
    }
    if app.confirm.is_some() {
        layers = layers.push(confirm_dialog(app, p));
    }
    layers.into()
}

fn tip_text(lang: Language, area: Area) -> &'static str {
    t(
        lang,
        match area {
            Area::Applications => Msg::TipApplications,
            Area::Tweaks => Msg::TipTweaks,
            Area::System => Msg::TipSystem,
            Area::Updates => Msg::TipUpdates,
            Area::Iso => Msg::TipIso,
            Area::TimeMachine => Msg::TipTimeMachine,
            Area::Inspector => Msg::TipInspector,
            Area::Extensions => Msg::TipExtensions,
        },
    )
}

/// The width a tip card takes when there is room for it.
pub const TIP_WIDTH: f32 = 340.0;

/// A card in the lower right corner of the page column. It moves up while the
/// dock is out and left while the drawer is open, so it covers neither.
fn tip_card(app: &App, p: Palette, area: Area, drawer_width: f32) -> El<'_> {
    let lang = app.language;
    // "Next" opens the next page with a tip; on the last one the button only closes.
    let next_label = if app.next_tip(area).is_some() { Msg::TipNext } else { Msg::TipDone };
    let card = container(
        column![
            row![PAGE_ICONS[Page::of_area(area).index()].view(13.0, p.accent), caption(t(lang, Msg::TipLabel), p.accent)].spacing(6).align_y(Alignment::Center),
            strong(Page::of_area(area).title_in(lang), p.ink),
            body(tip_text(lang, area), p.ink_secondary),
            row![
                space().width(Length::Fill),
                button(p, t(lang, Msg::TipNever), None, Tone::Secondary, Some(Message::TipNever)),
                button(p, t(lang, next_label), None, Tone::Primary, Some(Message::TipNext)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(6),
    )
    .max_width(TIP_WIDTH)
    .padding(14)
    .style(move |_| container::Style {
        background: Some(p.elevated.into()),
        border: Border { color: p.specular, width: 1.0, radius: GROUP_RADIUS.into() },
        shadow: floating_shadow(),
        ..container::Style::default()
    });
    let dock = app.timeline.channels[DOCK].value().clamp(0.0, 1.0) as f32;
    container(opaque(card))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::End)
        .align_y(Alignment::End)
        .padding(Padding {
            top: 16.0,
            right: drawer_width + 22.0,
            bottom: 22.0 + dock * 84.0,
            left: SIDEBAR_WIDTH + 16.0,
        })
        .into()
}

/// One large answer on a question screen, with the number key that picks it.
fn choice<'a>(p: Palette, number: usize, title: &'a str, detail: String, selected: bool, on_press: Message) -> El<'a> {
    let (fill, edge, width) = if selected { (p.selected, p.accent, 2.0) } else { (p.surface, p.border, 1.0) };
    let key = container(mono(number.to_string(), if selected { p.accent } else { p.ink_tertiary }))
        .center_x(22)
        .padding([2.0, 0.0])
        .style(move |_| container::Style {
            border: Border { color: if selected { p.accent } else { p.border }, width: 1.0, radius: 5.0.into() },
            ..container::Style::default()
        });
    press(
        container(
            row![key, column![strong(title, p.ink), caption(detail, p.ink_secondary)].spacing(2).width(Length::Fill)]
                .spacing(12)
                .align_y(Alignment::Center),
        )
            .width(Length::Fill)
            .padding([11.0, 16.0])
            .style(move |_| container::Style {
                background: Some(fill.into()),
                border: Border { color: edge, width, radius: GROUP_RADIUS.into() },
                ..container::Style::default()
            }),
    )
    .colors(Color { a: 0.05, ..p.ink }, p.selected)
    .radius(GROUP_RADIUS)
    .on_press(on_press)
    .into()
}

/// The rows of the summary screen: where each part of the plan is, and what it holds.
fn plan_rows(app: &App, answers: windowcraft_config::Answers) -> Vec<(String, String)> {
    let lang = app.language;
    let plan = answers.plan();
    let inspector = |tab: InspectorTab| format!("{} · {}", Page::Inspector.title_in(lang), tab.title_in(lang));
    let mut rows = vec![(
        Page::Tweaks.title_in(lang).to_owned(),
        format_1(t(lang, Msg::OnbPlanTweaks), &app.catalog.preset(plan.preset).len().to_string()),
    )];
    if plan.appx_defaults {
        rows.push((
            format!("{} · {}", Page::Tweaks.title_in(lang), TweakTab::Appx.title_in(lang)),
            format_1(t(lang, Msg::OnbPlanAppx), &app.catalog.presets.appx_default.len().to_string()),
        ));
    }
    if plan.interrupts {
        rows.push((inspector(InspectorTab::Interrupts), t(lang, Msg::OnbPlanInterrupts).to_owned()));
    }
    let clean = match plan.clean {
        CleanScope::TempOnly => Msg::OnbPlanCleanTemp,
        CleanScope::Caches => Msg::OnbPlanCleanCaches,
    };
    rows.push((inspector(InspectorTab::Cleaner), t(lang, clean).to_owned()));
    if plan.iso {
        rows.push((Page::Iso.title_in(lang).to_owned(), t(lang, Msg::OnbPlanIso).to_owned()));
    }
    rows.push((Page::TimeMachine.title_in(lang).to_owned(), t(lang, Msg::OnbPlanTimeMachine).to_owned()));
    rows
}

/// Height of everything on the question card except its body: the padding,
/// the gaps, the header row and the footer row.
const WIZARD_CHROME: f32 = 22.0 * 2.0 + 16.0 * 2.0 + 34.0 * 2.0;

/// The first-start questions. Every screen has Skip, and every screen after
/// the first has Back. The body scrolls when the window is too low for it,
/// while the header and the footer stay in view.
fn wizard_view(app: &App, p: Palette, wizard: Wizard) -> El<'_> {
    opaque(
        container(iced::widget::responsive(move |size| {
            let body = (size.height - WIZARD_CHROME).max(48.0);
            container(wizard_card(app, p, wizard, body)).center_x(Length::Fill).center_y(Length::Fill).into()
        }))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(16)
        .style(|_| container::background(Color { a: 0.5, ..Color::BLACK })),
    )
}

fn wizard_card(app: &App, p: Palette, wizard: Wizard, body_height: f32) -> El<'_> {
    let lang = app.language;
    let answers = wizard.answers;
    let step_label = format_2(t(lang, Msg::OnbStep), &(wizard.step + 1).to_string(), &WIZARD_STEPS.to_string());
    let header = row![
        container(progress(p, (wizard.step + 1) as f32 / WIZARD_STEPS as f32)).width(Length::Fill),
        mono(step_label, p.ink_tertiary),
        button(p, t(lang, Msg::OnbSkip), None, Tone::Secondary, Some(Message::Onboarding(Onboard::Skip))),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let preset_size = |depth: Depth| app.catalog.preset(depth.preset()).len().to_string();
    let mut content = Column::new().spacing(10);
    // Answers are numbered; keys 1 to 4 pick them, see `App::on_keyboard`.
    let mut number = 0;
    match wizard.step {
        0 => {
            content = content.push(heading(t(lang, Msg::OnbGoalQuestion), p.ink)).push(caption(t(lang, Msg::OnbIntro), p.ink_secondary));
            for (goal, title, detail) in [
                (Goal::CleanInstall, Msg::OnbGoalClean, Msg::OnbGoalCleanDetail),
                (Goal::Privacy, Msg::OnbGoalPrivacy, Msg::OnbGoalPrivacyDetail),
                (Goal::Gaming, Msg::OnbGoalGaming, Msg::OnbGoalGamingDetail),
                (Goal::Tidy, Msg::OnbGoalTidy, Msg::OnbGoalTidyDetail),
            ] {
                let selected = answers.goal == Some(goal);
                content = content.push(choice(p, { number += 1; number }, t(lang, title), t(lang, detail).to_owned(), selected, Message::Onboarding(Onboard::Goal(goal))));
            }
        }
        1 => {
            content = content.push(heading(t(lang, Msg::OnbFilesQuestion), p.ink)).push(caption(t(lang, Msg::OnbFilesHint), p.ink_secondary));
            for (keep, title, detail) in [(true, Msg::OnbFilesYes, Msg::OnbFilesYesDetail), (false, Msg::OnbFilesNo, Msg::OnbFilesNoDetail)] {
                let selected = answers.keep_files == Some(keep);
                content = content.push(choice(p, { number += 1; number }, t(lang, title), t(lang, detail).to_owned(), selected, Message::Onboarding(Onboard::KeepFiles(keep))));
            }
        }
        2 => {
            content = content.push(heading(t(lang, Msg::OnbIsoQuestion), p.ink));
            let yes = format_1(t(lang, Msg::OnbIsoYesDetail), Page::Iso.title_in(lang));
            for (build, title, detail) in [(true, Msg::OnbIsoYes, yes), (false, Msg::OnbIsoNo, t(lang, Msg::OnbIsoNoDetail).to_owned())] {
                let selected = answers.build_iso == Some(build);
                content = content.push(choice(p, { number += 1; number }, t(lang, title), detail, selected, Message::Onboarding(Onboard::BuildIso(build))));
            }
        }
        3 => {
            content = content.push(heading(t(lang, Msg::OnbDepthQuestion), p.ink));
            for (depth, title, detail) in [
                (Depth::Careful, Msg::OnbDepthCareful, Msg::OnbDepthCarefulDetail),
                (Depth::Standard, Msg::OnbDepthStandard, Msg::OnbDepthStandardDetail),
                (Depth::Everything, Msg::OnbDepthEverything, Msg::OnbDepthEverythingDetail),
            ] {
                let selected = answers.depth == Some(depth);
                let detail = format_1(t(lang, detail), &preset_size(depth));
                content = content.push(choice(p, { number += 1; number }, t(lang, title), detail, selected, Message::Onboarding(Onboard::Depth(depth))));
            }
        }
        _ => {
            let rows: Vec<El<'_>> = plan_rows(app, answers)
                .into_iter()
                .map(|(title, detail)| {
                    container(column![strong(title, p.ink), caption(detail, p.ink_secondary)].spacing(1))
                        .width(Length::Fill)
                        .padding([9.0, ROW_INSET])
                        .into()
                })
                .collect();
            content = content
                .push(heading(t(lang, Msg::OnbSummaryTitle), p.ink))
                .push(group(p, rows, ROW_INSET))
                .push(caption(t(lang, Msg::OnbSummaryNote), p.ink_secondary));
        }
    }

    let mut footer = Row::new().spacing(8).align_y(Alignment::Center);
    if wizard.step > 0 {
        footer = footer.push(button(p, t(lang, Msg::OnbBack), None, Tone::Secondary, Some(Message::Onboarding(Onboard::Back))));
    }
    footer = footer.push(space().width(Length::Fill));
    if wizard.step == SUMMARY_STEP {
        footer = footer.push(button(p, t(lang, Msg::OnbStart), Some(Icon::Play), Tone::Primary, Some(Message::Onboarding(Onboard::Start))));
    }

    let card = container(
        column![
            header,
            container(crate::smooth::smooth(
                scrollable(container(content).padding(Padding { right: 12.0, ..Padding::ZERO }))
                    .height(Length::Shrink)
                    .style(scroll_style(p)),
            ))
            .max_height(body_height),
            footer
        ]
        .spacing(16),
    )
    .max_width(560)
    .padding(22)
    .style(move |_| container::Style {
        background: Some(p.elevated.into()),
        border: Border { color: p.specular, width: 1.0, radius: GROUP_RADIUS.into() },
        shadow: floating_shadow(),
        ..container::Style::default()
    });
    card.into()
}

fn sidebar(app: &App, p: Palette) -> El<'_> {
    let elevation: El<'_> = if app.info.elevated {
        badge(p, t(app.language, Msg::Administrator), p.green)
    } else {
        press(badge(p, t(app.language, Msg::StandardUser), p.amber)).on_press(Message::Relaunch).into()
    };
    let header = column![
        strong("WindowCraft", p.ink),
        mono(format_1(t(app.language, Msg::BuildFormat), &app.info.full_build()), p.ink_tertiary),
        space().height(6),
        elevation,
    ]
    .spacing(1);

    let indicator_y = app.timeline.channels[NAV].value() as f32 * NAV_ROW_HEIGHT;
    let on_settings = app.page == Page::Settings;
    let highlight = container(
        container(space().width(Length::Fill).height(NAV_ROW_HEIGHT - 2.0)).style(move |_| container::Style {
            background: (!on_settings).then(|| p.selected.into()),
            border: Border { radius: CONTROL_RADIUS.into(), ..Border::default() },
            ..container::Style::default()
        }),
    )
    .padding(Padding { top: indicator_y + 1.0, ..Padding::ZERO });

    let mut items = Column::new();
    for (page, icon) in Page::ALL.into_iter().zip(PAGE_ICONS) {
        let active = app.page == page;
        let ink = if active { p.ink } else { p.ink_secondary };
        items = items.push(
            press(
                container(row![icon.view(16.0, ink), caption(page.title_in(app.language), ink)].spacing(10).align_y(Alignment::Center))
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
            row![
                icon_button(p, appearance_icon, Message::CycleAppearance),
                selected_icon_button(p, Icon::Gear, on_settings, Message::Navigate(Page::Settings)),
                mono(concat!("v", env!("CARGO_PKG_VERSION")), p.ink_tertiary)
            ]
            .spacing(6)
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
    let search = text_input(t(app.language, Msg::SearchPlaceholder), &app.query)
        .id(search_id())
        .on_input(Message::Query)
        .size(13)
        .font(SANS)
        .padding([6.0, 10.0])
        .width(Length::Fill)
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
            // The field takes the room the heading leaves, up to 460 px, so
            // the longer placeholders fit in a wide window and it still
            // shrinks with the inspector open.
            row![
                heading(app.page.title_in(app.language), p.ink),
                container(container(search).max_width(460)).width(Length::Fill).align_x(Alignment::End),
                mono("Ctrl+K", p.ink_tertiary)
            ]
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
            Page::Extensions => extensions_page(app, p),
            Page::TimeMachine => time_machine(app, p),
            Page::Inspector => inspector_page(app, p),
            Page::Settings => settings_page(app, p),
        }
    };

    // The drift banner and the notice always hold their own slot, empty or
    // not. Iced matches children by position, so a banner that came and went
    // in front of the body would hand the body a fresh state and throw the
    // page back to the top.
    let drift = i18n::drift_summary(app.language, &app.drift)
        .filter(|_| app.page != Page::TimeMachine)
        .map(|summary| banner(p, Icon::Alert, p.amber, summary, vec![(t(app.language, Msg::ReApply), Message::RepairDrift)]));
    let notice = app.notice.as_ref().map(|notice| {
        let mut actions = vec![(t(app.language, Msg::Dismiss), Message::DismissNotice)];
        if app.needs_elevation {
            actions.insert(0, (t(app.language, Msg::RestartAsAdministrator), Message::Relaunch));
        }
        if app.notice_failed.as_ref() == Some(notice) {
            banner(p, Icon::Alert, p.amber, notice.clone(), actions)
        } else {
            banner(p, Icon::Check, p.ink_secondary, notice.clone(), actions)
        }
    });
    let mut main = Column::new().push(toolbar).push(banner_slot(drift)).push(banner_slot(notice));
    // A new page or tab rises into place by a few pixels. The key changes only
    // when the visible module does, so typing or toggling never replays it.
    let module = match app.page {
        Page::Tweaks => app.tab.index(),
        Page::Inspector => app.inspector_tab.index(),
        _ => 0,
    };
    let key = (app.page.index() as u64) << 8 | (module as u64) << 1 | u64::from(!app.query.trim().is_empty());
    main = main.push(crate::reveal::reveal(body, key));

    let mut layers = stack![main.width(Length::Fill).height(Length::Fill)];
    let dock_progress = app.timeline.channels[DOCK].value() as f32;
    if dock_progress > 0.01 {
        layers = layers.push(dock(app, p, dock_progress));
    }
    layers.width(Length::Fill).height(Length::Fill).into()
}

/// A banner, or nothing, in a container that stays in the tree either way.
/// Column::push drops children of zero fixed size, so an empty space alone
/// would not hold the slot.
fn banner_slot(content: Option<El<'_>>) -> El<'_> {
    match content {
        Some(content) => container(content).padding(Padding { right: 22.0, left: 22.0, bottom: 6.0, ..Padding::ZERO }).into(),
        None => container(space()).into(),
    }
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
                app.timeline.channels[TAB].value(),
                84.0,
                |index| Message::Manager(if index == 0 { PackageManager::Winget } else { PackageManager::Chocolatey }),
            );
            column![
                row![
                    manager,
                    button(
                        p,
                        if app.checking_upgrades { t(app.language, Msg::CheckingVersions) } else { t(app.language, Msg::ShowInstalled) },
                        Some(Icon::Eye),
                        Tone::Secondary,
                        (!app.checking_upgrades).then_some(Message::ShowInstalled),
                    ),
                    button(p, t(app.language, Msg::UpgradeAll), Some(Icon::Refresh), Tone::Secondary, app.job.is_none().then_some(Message::UpgradeAll)),
                    button(p, t(app.language, Msg::ClearSelection), None, Tone::Secondary, Some(Message::ClearApps)),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                // A narrow window with the inspector open moves buttons to a
                // second line instead of breaking their labels.
                .wrap()
                .vertical_spacing(8),
                // Every category stays reachable: the chips wrap instead of
                // running off the right edge.
                chips.wrap().vertical_spacing(6),
            ]
            .spacing(8)
            .into()
        }
        Page::Tweaks => {
            // Sized from the labels, as in the Inspector, so a long translation fits.
            let labels: Vec<&str> = TweakTab::ALL.iter().map(|tab| tab.title_in(app.language)).collect();
            let width = tab_width(&labels, 104.0);
            let bar = tabs(p, labels, app.timeline.channels[TAB].value(), width, |index| Message::Tab(TweakTab::ALL[index]));
            let mut controls = row![bar].spacing(8).align_y(Alignment::Center);
            if app.tab != TweakTab::Appx {
                controls = controls
                    .push(button(p, t(app.language, Msg::PresetMinimal), None, Tone::Secondary, Some(Message::Preset(windowcraft_config::PresetName::Minimal))))
                    .push(button(p, t(app.language, Msg::PresetStandard), None, Tone::Secondary, Some(Message::Preset(windowcraft_config::PresetName::Standard))))
                    .push(button(p, t(app.language, Msg::PresetAdvanced), None, Tone::Secondary, Some(Message::Preset(windowcraft_config::PresetName::Advanced))))
                    .push(button(p, t(app.language, Msg::DetectApplied), None, Tone::Secondary, Some(Message::DetectApplied)))
                    .push(button(p, t(app.language, Msg::Clear), None, Tone::Secondary, Some(Message::ClearTweaks)));
            } else {
                controls = controls.push(button(p, t(app.language, Msg::DefaultRemovals), None, Tone::Secondary, Some(Message::AppxDefaults)));
            }
            // Wraps instead of clipping when the inspector narrows this column.
            controls
                .push(button(p, t(app.language, Msg::Import), None, Tone::Secondary, Some(Message::ImportProfile)))
                .push(button(p, t(app.language, Msg::Export), None, Tone::Secondary, Some(Message::ExportProfile)))
                .wrap()
                .vertical_spacing(8)
                .into()
        }
        Page::Inspector => {
            let labels: Vec<&str> = InspectorTab::ALL.iter().map(|tab| tab.title_in(app.language)).collect();
            let width = tab_width(&labels, 100.0);
            tabs(p, labels, app.timeline.channels[TAB].value(), width, |index| Message::InspectorTab(InspectorTab::ALL[index]))
        }
        _ => space().height(0).into(),
    }
}

fn dock<'a>(app: &'a App, p: Palette, progress_value: f32) -> El<'a> {
    let content: El<'a> = if let Some(job) = &app.job {
        column![
            row![
                strong(job.label.as_str(), p.ink).width(Length::Fill),
                mono(format!("{:>3.0}%", job.fraction * 100.0), p.ink_secondary),
                button(p, t(app.language, Msg::Cancel), Some(Icon::Stop), Tone::Secondary, Some(Message::CancelJob)),
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
            Page::Applications => {
                let count = app.selected_apps.len();
                let noun = app.language.plural(
                    count,
                    t(app.language, Msg::AppsSelectedOne),
                    t(app.language, Msg::AppsSelectedFew),
                    t(app.language, Msg::AppsSelectedMany),
                );
                (
                    count,
                    noun,
                    vec![
                        button(p, t(app.language, Msg::Uninstall), Some(Icon::Trash), Tone::Destructive, Some(Message::UninstallApps)),
                        button(p, t(app.language, Msg::InstallUpgrade), Some(Icon::Download), Tone::Primary, Some(Message::InstallApps)),
                    ],
                )
            }
            Page::Tweaks if app.tab == TweakTab::Appx => {
                let count = app.selected_appx.len();
                let noun = app.language.plural(
                    count,
                    t(app.language, Msg::PackagesSelectedOne),
                    t(app.language, Msg::PackagesSelectedFew),
                    t(app.language, Msg::PackagesSelectedMany),
                );
                (
                    count,
                    noun,
                    vec![
                        button(p, t(app.language, Msg::Install), Some(Icon::Download), Tone::Secondary, Some(Message::InstallAppx)),
                        button(p, t(app.language, Msg::Remove), Some(Icon::Trash), Tone::Destructive, Some(Message::RemoveAppx)),
                    ],
                )
            }
            Page::System => {
                let count = app.selected_features.len();
                let noun = app.language.plural(
                    count,
                    t(app.language, Msg::FeaturesSelectedOne),
                    t(app.language, Msg::FeaturesSelectedFew),
                    t(app.language, Msg::FeaturesSelectedMany),
                );
                (
                    count,
                    noun,
                    vec![button(p, t(app.language, Msg::InstallFeatures), Some(Icon::Download), Tone::Primary, Some(Message::InstallFeatures))],
                )
            }
            _ => {
                let count = app.selected_tweaks.len();
                let noun = app.language.plural(
                    count,
                    t(app.language, Msg::TweaksQueuedOne),
                    t(app.language, Msg::TweaksQueuedFew),
                    t(app.language, Msg::TweaksQueuedMany),
                );
                (
                    count,
                    noun,
                    vec![button(p, t(app.language, Msg::RunTweaks), Some(Icon::Play), Tone::Primary, Some(Message::ApplyTweaks))],
                )
            }
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
            // The catalog gives English context; tweaks and fixes get the
            // tab or page name in the chosen language instead.
            let detail = match hit.kind {
                SearchKind::Tweak => app
                    .catalog
                    .tweak(&hit.id)
                    .map_or(hit.detail.as_str(), |tweak| i18n::tweak_group(app.language, tweak.group)),
                SearchKind::Fix => Page::System.title_in(app.language),
                _ => hit.detail.as_str(),
            };
            list_row(
                p,
                None,
                hit.title.as_str(),
                detail,
                badge(p, i18n::search_kind(app.language, hit.kind), p.ink_secondary),
                Some(Message::SearchSelected(hit.kind, hit.id.clone())),
            )
        })
        .collect();
    let content: El<'_> = if rows.is_empty() {
        body(t(app.language, Msg::NoMatches), p.ink_secondary).into()
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
                trailing = trailing.push(badge(p, t(app.language, Msg::FossBadge), p.ink_tertiary));
            }
            trailing = trailing.push(icon_button(p, Icon::Eye, Message::Inspect(Inspected::App(item.id.clone()))));
            rows.push(list_row(
                p,
                Some(check(p, selected, Message::AppToggled(item.id.clone()))),
                item.name.as_str(),
                package.unwrap_or(t(app.language, Msg::NotAvailableFromSource)),
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

fn tweak_detail(lang: Language, tweak: &Tweak) -> String {
    tweak.registry.first().map_or_else(
        || {
            tweak.services.first().map_or_else(
                || tweak.apply.first().map_or_else(String::new, windowcraft_config::NativeOp::describe),
                |rule| format_2(t(lang, Msg::ServiceRuleFormat), &rule.name, i18n::service_startup(lang, rule.startup)),
            )
        },
        |entry| {
            let more = tweak.registry.len() - 1;
            let value = entry.value.as_deref().unwrap_or(t(lang, Msg::PerOption));
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
            let options = choices(&tweak.options);
            let current = chosen(&options, app.options.get(&tweak.id).map(String::as_str));
            (None, select(p, options, current, move |choice: Choice| Message::Segment(id.clone(), choice.value)), None)
        }
        TweakKind::Action => (
            None,
            button(p, t(app.language, Msg::Run), Some(Icon::Play), Tone::Secondary, app.job.is_none().then_some(Message::RunAction(id))),
            None,
        ),
        TweakKind::Dns => {
            let options = choices(&tweak.options);
            let current = chosen(&options, Some(&app.dns_choice));
            (None, select(p, options, current, |choice: Choice| Message::Dns(choice.value)), None)
        }
    };

    let mut trailing = Row::new().spacing(10).align_y(Alignment::Center);
    if tweak.group == TweakGroup::Advanced {
        trailing = trailing.push(badge(p, t(app.language, Msg::CautionBadge), p.amber));
    }
    if tweak.is_detectable() {
        trailing = trailing.push(dot(status_color(p, detection)));
    }
    trailing = trailing.push(control).push(inspect);
    list_row(p, leading, tweak.name.as_str(), tweak_detail(app.language, tweak), trailing.into(), on_press)
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
        sections = sections.push(group_titled(p, i18n::tweak_group(app.language, *group_kind), rows, ROW_INSET));
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
                    trailing = trailing.push(badge(p, t(app.language, Msg::DependencyBadge), p.amber));
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
                    let options = choices(&feature.options);
                    let current = chosen(&options, Some(app.fix_modes.get("update").map_or("Standard", String::as_str)));
                    trailing = trailing.push(select(p, options, current, |choice: Choice| Message::FixMode(choice.value)));
                }
                trailing = trailing.push(button(
                    p,
                    t(app.language, Msg::Run),
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
            group_titled(p, t(app.language, Msg::WindowsOptionalFeatures), feature_rows, 41.0),
            group_titled(p, t(app.language, Msg::OneClickFixes), action_rows(FeatureKind::Fix), ROW_INSET),
            group_titled(p, t(app.language, Msg::PowerShellProfileAndOpenSsh), remote, ROW_INSET),
            group_titled(p, t(app.language, Msg::ClassicWindowsPanels), panel_rows, ROW_INSET),
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
                badge(p, t(app.language, Msg::ActiveBadge), p.green)
            } else {
                let tone = if mode.confirm { Tone::Destructive } else { Tone::Secondary };
                button(p, t(app.language, Msg::Apply), None, tone, app.job.is_none().then(|| Message::UpdateMode(mode.id.clone())))
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
                button(p, t(app.language, Msg::Browse), Some(Icon::Folder), Tone::Secondary, Some(Message::IsoBrowse)),
                button(p, t(app.language, Msg::Open), None, Tone::Primary, (ready && !iso.path.is_empty()).then_some(Message::IsoOpen)),
            ]
            .spacing(8),
            row![
                input(p, t(app.language, Msg::IsoDirectDownloadUrlPlaceholder), &iso.url, Message::IsoDownloadUrl),
                input(p, t(app.language, Msg::IsoExpectedSha256Placeholder), &iso.hash, Message::IsoDownloadHash),
                button(p, t(app.language, Msg::Download), Some(Icon::Download), Tone::Secondary, (ready && !iso.url.is_empty()).then_some(Message::IsoDownload)),
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
                format_3(t(app.language, Msg::IsoEditionFormat), &image.index.to_string(), &image.architecture, &image.build),
                space().width(0).into(),
                Some(Message::IsoEdition(image.index)),
            )
        })
        .collect();

    let options = vec![
        iso_option(p, t(app.language, Msg::IsoBypassHardware), t(app.language, Msg::IsoBypassHardwareDetail), IsoFlag::Bypass, iso.options.bypass_requirements),
        iso_option(p, t(app.language, Msg::IsoLocalAccount), t(app.language, Msg::IsoLocalAccountDetail), IsoFlag::LocalAccount, iso.options.local_account),
        iso_option(p, t(app.language, Msg::IsoDarkMode), t(app.language, Msg::IsoDarkModeDetail), IsoFlag::DarkMode, iso.options.dark_mode),
        iso_option(p, t(app.language, Msg::IsoDisableEncryption), t(app.language, Msg::IsoDisableEncryptionDetail), IsoFlag::NoEncryption, iso.options.disable_device_encryption),
        iso_option(p, t(app.language, Msg::IsoPauseUpdates), t(app.language, Msg::IsoPauseUpdatesDetail), IsoFlag::PauseUpdates, iso.options.pause_updates_during_oobe),
        iso_option(p, t(app.language, Msg::IsoInjectDrivers), t(app.language, Msg::IsoInjectDriversDetail), IsoFlag::InjectDrivers, iso.inject_drivers),
        iso_option(p, t(app.language, Msg::IsoBundleApps), t(app.language, Msg::IsoBundleAppsDetail), IsoFlag::BundleApps, iso.bundle_apps),
    ];

    let configured = !iso.images.is_empty();
    let mut export_rows: Vec<El<'_>> = vec![
        container(
            row![
                input(p, r"C:\Output\Windows11.iso", &iso.output, Message::IsoOutput),
                button(p, t(app.language, Msg::Browse), Some(Icon::Folder), Tone::Secondary, Some(Message::IsoBrowseOutput)),
                button(p, t(app.language, Msg::IsoBuildIso), Some(Icon::Disc), Tone::Primary, (ready && configured && !iso.output.is_empty()).then_some(Message::IsoBuild)),
            ]
            .spacing(8),
        )
        .padding(ROW_INSET)
        .into(),
        list_row(
            p,
            None,
            t(app.language, Msg::IsoWriteToUsbDrive),
            t(app.language, Msg::IsoUsbPartitionDetail),
            button(p, t(app.language, Msg::IsoFindDrives), None, Tone::Secondary, Some(Message::IsoRefreshDisks)),
            None,
        ),
    ];
    for disk in &iso.disks {
        export_rows.push(list_row(
            p,
            None,
            disk.model.as_str(),
            format_3(t(app.language, Msg::IsoDiskFormat), &disk.number.to_string(), &(disk.bytes >> 30).to_string(), &i18n::bus_type(app.language, disk.bus)),
            button(p, t(app.language, Msg::IsoEraseAndWrite), Some(Icon::Alert), Tone::Destructive, (ready && configured).then_some(Message::IsoWriteUsb(disk.number))),
            None,
        ));
    }

    let mut sections = column![group_titled(p, t(app.language, Msg::IsoSourceTitle), vec![source.into()], ROW_INSET)].spacing(16);
    if !editions.is_empty() {
        sections = sections.push(group_titled(p, t(app.language, Msg::IsoEditionTitle), editions, 41.0));
    }
    sections = sections.push(group_titled(p, t(app.language, Msg::IsoConfigurationTitle), options, ROW_INSET));
    sections = sections.push(group_titled(p, t(app.language, Msg::IsoExportTitle), export_rows, ROW_INSET));
    scroll_area(p, sections)
}

/// A full-width caption row inside a group.
fn note_row<'a>(text: &'a str, color: Color) -> El<'a> {
    container(caption(text, color)).width(Length::Fill).padding([10.0, ROW_INSET]).into()
}

/// Last row of an installed extension: what uninstalling does, and the button.
fn installed_row(app: &App, p: Palette, extension: Extension) -> El<'_> {
    let lang = app.language;
    container(
        row![
            caption(t(lang, if extension.tools().is_empty() { Msg::ExtInstalledBuiltIn } else { Msg::ExtInstalled }), p.ink_tertiary)
                .width(Length::Fill),
            button(p, t(lang, Msg::ExtUninstall), Some(Icon::Trash), Tone::Secondary, app.job.is_none().then_some(Message::UninstallExtension(extension))),
        ]
        .spacing(16)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([10.0, ROW_INSET])
    .into()
}

/// Rows of an extension that is not installed: what it does, and Install.
fn install_rows<'a>(app: &'a App, p: Palette, extension: Extension, title: &'a str, detail: &'a str, needs: &'a str) -> Vec<El<'a>> {
    let lang = app.language;
    vec![
        setting_row(
            p,
            title,
            detail,
            button(p, t(lang, Msg::ExtInstall), Some(Icon::Download), Tone::Primary, app.job.is_none().then_some(Message::InstallExtension(extension))),
        ),
        note_row(needs, p.ink_tertiary),
    ]
}

fn stripper_rows(app: &App, p: Palette) -> Vec<El<'_>> {
    let lang = app.language;
    if !app.extensions.has(Extension::Stripper) {
        return install_rows(app, p, Extension::Stripper, t(lang, Msg::MiscStripperTitle), t(lang, Msg::MiscStripperDetail), t(lang, Msg::ExtStripperNeeds));
    }
    vec![
        setting_row(
            p,
            t(lang, Msg::MiscStripperTitle),
            t(lang, Msg::MiscStripperDetail),
            button(p, t(lang, Msg::MiscChooseFiles), Some(Icon::Folder), Tone::Primary, app.job.is_none().then_some(Message::StripPick)),
        ),
        note_row(t(lang, Msg::MiscStripperFormats), p.ink_tertiary),
        installed_row(app, p, Extension::Stripper),
    ]
}

fn converter_rows(app: &App, p: Palette) -> Vec<El<'_>> {
    let lang = app.language;
    if !app.extensions.has(Extension::Converter) {
        return install_rows(app, p, Extension::Converter, t(lang, Msg::ConverterTitle), t(lang, Msg::ConverterDetail), t(lang, Msg::ExtStripperNeeds));
    }
    let mut families = Column::new().spacing(8);
    for (family, msg) in [(Family::Image, Msg::ConverterImage), (Family::Video, Msg::ConverterVideo), (Family::Audio, Msg::ConverterAudio)] {
        let mut chips = Row::new().spacing(6);
        for target in Target::ALL.into_iter().filter(|target| target.family() == family) {
            chips = chips.push(chip(p, target.label().to_owned(), app.extensions.target == target, Message::ConvertTarget(target)));
        }
        families = families.push(
            row![caption(t(lang, msg), p.ink_secondary).width(Length::Fixed(90.0)), chips.wrap().vertical_spacing(6)]
                .spacing(8)
                .align_y(Alignment::Center),
        );
    }
    vec![
        setting_row(
            p,
            t(lang, Msg::ConverterTitle),
            t(lang, Msg::ConverterDetail),
            button(p, t(lang, Msg::MiscChooseFiles), Some(Icon::Folder), Tone::Primary, app.job.is_none().then_some(Message::ConvertPick)),
        ),
        container(families).width(Length::Fill).padding([10.0, ROW_INSET]).into(),
        note_row(if app.extensions.ffmpeg { t(lang, Msg::ConverterCopyNote) } else { t(lang, Msg::ConverterNativeNote) }, p.ink_tertiary),
        installed_row(app, p, Extension::Converter),
    ]
}

fn ffmpeg_rows(app: &App, p: Palette) -> Vec<El<'_>> {
    let lang = app.language;
    if !app.extensions.has(Extension::Ffmpeg) {
        return install_rows(app, p, Extension::Ffmpeg, t(lang, Msg::FfmpegTitle), t(lang, Msg::FfmpegDetail), t(lang, Msg::ExtFfmpegNeeds));
    }
    vec![setting_row(p, t(lang, Msg::FfmpegTitle), t(lang, Msg::FfmpegDetail), space().width(0).into()), installed_row(app, p, Extension::Ffmpeg)]
}

fn youtube_rows(app: &App, p: Palette) -> Vec<El<'_>> {
    let lang = app.language;
    let state = &app.extensions;
    if !state.has(Extension::YouTube) {
        let mut rows = install_rows(app, p, Extension::YouTube, t(lang, Msg::YoutubeTitle), t(lang, Msg::YoutubeDetail), t(lang, Msg::ExtYoutubeNeeds));
        rows.push(note_row(t(lang, Msg::MiscPrivacyNote), p.ink_tertiary));
        return rows;
    }
    let formats = tabs(p, vec!["MP4", "MP3"], app.timeline.channels[TAB].value(), 64.0, |index| {
        Message::VideoFormat(if index == 0 { Format::Mp4 } else { Format::Mp3 })
    });
    let format_detail = match state.format {
        Format::Mp4 => t(lang, Msg::MiscFormatDetailMp4),
        Format::Mp3 => t(lang, Msg::MiscFormatDetailMp3),
    };
    let can_download = app.job.is_none() && !state.url.trim().is_empty();
    vec![
        container(
            column![
                row![
                    input(p, t(lang, Msg::MiscUrlPlaceholder), &state.url, Message::VideoUrl),
                    formats,
                    button(p, t(lang, Msg::Download), Some(Icon::Download), Tone::Primary, can_download.then_some(Message::VideoDownload)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                caption(format_detail, p.ink_tertiary),
            ]
            .spacing(6),
        )
        .padding(ROW_INSET)
        .into(),
        container(
            row![
                caption(t(lang, Msg::MiscSaveTo), p.ink_secondary),
                input(p, "", &state.folder, Message::VideoFolder),
                button(p, t(lang, Msg::Browse), Some(Icon::Folder), Tone::Secondary, Some(Message::VideoBrowseFolder)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([10.0, ROW_INSET])
        .into(),
        note_row(t(lang, Msg::MiscPrivacyNote), p.ink_secondary),
        installed_row(app, p, Extension::YouTube),
    ]
}

fn extensions_page(app: &App, p: Palette) -> El<'_> {
    let lang = app.language;
    scroll_area(
        p,
        column![
            container(caption(t(lang, Msg::ExtIntro), p.ink_secondary)).padding(Padding { left: 4.0, ..Padding::ZERO }),
            group_titled_badged(p, t(lang, Msg::MiscStripperGroup), badge(p, t(lang, Msg::ExtOwnCode), OWN_CODE_BLUE), stripper_rows(app, p), ROW_INSET),
            group_titled_badged(p, t(lang, Msg::ConverterGroup), badge(p, t(lang, Msg::ExtOwnCode), OWN_CODE_BLUE), converter_rows(app, p), ROW_INSET),
            group_titled(p, t(lang, Msg::FfmpegGroup), ffmpeg_rows(app, p), ROW_INSET),
            group_titled(p, t(lang, Msg::MiscDownloaderGroup), youtube_rows(app, p), ROW_INSET),
            container(caption(t(lang, Msg::MiscComingSoon), p.ink_tertiary)).padding(Padding { left: 4.0, ..Padding::ZERO }),
        ]
        .spacing(16),
    )
}

fn relative_time(lang: Language, unix: u64) -> String {
    let seconds = unix_now().saturating_sub(unix);
    match seconds {
        0..=59 => t(lang, Msg::JustNow).to_owned(),
        60..=3_599 => format_1(t(lang, Msg::MinAgo), &(seconds / 60).to_string()),
        3_600..=86_399 => format_1(t(lang, Msg::HoursAgo), &(seconds / 3_600).to_string()),
        _ => format_1(t(lang, Msg::DaysAgo), &(seconds / 86_400).to_string()),
    }
}

fn change_row<'a>(lang: Language, p: Palette, view: &'a ChangeView) -> El<'a> {
    let (before, after) = match &view.record.change {
        Change::Registry { before, after, .. } => (
            before.as_ref().map_or_else(|| t(lang, Msg::NotSet).to_owned(), windowcraft_engine::RegValue::display),
            after.as_ref().map_or_else(|| t(lang, Msg::Removed).to_owned(), windowcraft_engine::RegValue::display),
        ),
        Change::Service { before, after, .. } => (
            before.map_or_else(|| t(lang, Msg::Absent).to_owned(), |value| i18n::service_startup(lang, value).to_owned()),
            i18n::service_startup(lang, *after).to_owned(),
        ),
        Change::Task { before, after, .. } => (before.to_string(), after.to_string()),
        Change::Native { .. } => (String::new(), String::new()),
    };
    let reversible = !matches!(view.record.change, Change::Native { .. }) && !view.record.rolled_back;
    let trailing: El<'a> = if view.record.rolled_back {
        badge(p, t(lang, Msg::RolledBackBadge), p.ink_tertiary)
    } else if reversible {
        icon_button(p, Icon::Undo, Message::RollbackChange(view.record.session, view.record.index))
    } else {
        space().width(0).into()
    };
    let drifted = !view.live.is_empty() && view.live != after && !view.record.rolled_back;
    let line = if view.live.is_empty() {
        before.clone()
    } else {
        format_3(t(lang, Msg::LiveFormat), &before, &after, &view.live)
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
    if let Some(summary) = i18n::drift_summary(app.language, &app.drift) {
        sections = sections.push(banner(p, Icon::Alert, p.amber, summary, vec![(t(app.language, Msg::ReApply), Message::RepairDrift)]));
        let rows: Vec<El<'_>> = app
            .drift
            .iter()
            .flat_map(|drift| {
                drift.entries.iter().map(move |(target, expected, live)| {
                    let detail = format_3(t(app.language, Msg::DriftDetailFormat), target, expected, live);
                    let name = app.catalog.tweak(&drift.tweak_id).map_or(drift.tweak_name.as_str(), |tweak| tweak.name.as_str());
                    list_row(p, None, name, detail, dot(p.amber), None)
                })
            })
            .collect();
        sections = sections.push(group_titled(p, t(app.language, Msg::ConfigurationDrift), rows, ROW_INSET));
    }

    if app.sessions.is_empty() {
        sections = sections.push(body(t(app.language, Msg::NoChangesRecorded), p.ink_secondary));
    } else {
        let mut rows: Vec<El<'_>> = Vec::new();
        for session in &app.sessions {
            let state_color = match session.state {
                SessionState::Committed => p.green,
                SessionState::RolledBack => p.ink_tertiary,
                SessionState::Failed | SessionState::Open | SessionState::PartlyRolledBack => p.amber,
            };
            let mut trailing = row![dot(state_color), caption(i18n::session_state(app.language, session.state), p.ink_secondary)]
                .spacing(6)
                .align_y(Alignment::Center);
            // An interrupted or failed session is exactly what the write-ahead
            // record is for: a feature set can fail after some of it was enabled.
            if matches!(session.state, SessionState::Committed | SessionState::Open | SessionState::Failed) {
                trailing = trailing.push(button(
                    p,
                    t(app.language, Msg::RollBack),
                    Some(Icon::Undo),
                    Tone::Secondary,
                    app.job.is_none().then_some(Message::Ask(Confirm::RollbackSession(session.id))),
                ));
            }
            rows.push(list_row(
                p,
                None,
                session.label.as_str(),
                format_2(t(app.language, Msg::SessionFormat), &session.id.to_string(), &relative_time(app.language, session.created_unix)),
                trailing.into(),
                Some(Message::SelectSession(session.id)),
            ));
            if app.open_session == Some(session.id) {
                let height = app.timeline.channels[ACCORDION].value() as f32;
                let mut changes = Column::new();
                for view in &app.changes {
                    changes = changes.push(change_row(app.language, p, view));
                }
                rows.push(container(changes).height(height).clip(true).width(Length::Fill).into());
            }
        }
        sections = sections.push(group_titled(p, t(app.language, Msg::SnapshotTimeline), rows, ROW_INSET));
    }
    sections = sections.push(button(p, t(app.language, Msg::ExportEnvironmentReport), Some(Icon::Terminal), Tone::Secondary, app.job.is_none().then_some(Message::ExportReport)));
    if !app.last_log.is_empty() {
        let mut lines = Column::new().spacing(2);
        for line in app.last_log.iter().rev().take(LOG_LINES_SHOWN).rev() {
            lines = lines.push(mono(line.as_str(), p.ink_secondary));
        }
        let log: El<'_> = container(lines).padding(ROW_INSET).width(Length::Fill).into();
        sections = sections.push(group_titled(p, app.last_log_label.as_str(), vec![log], ROW_INSET));
        sections = sections.push(caption(t(app.language, Msg::FullHistoryLog), p.ink_tertiary));
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
        return body(t(app.language, Msg::NoPcieDevicesFound), p.ink_secondary).into();
    }
    let rows: Vec<El<'_>> = app
        .devices
        .iter()
        .enumerate()
        .map(|(index, device)| {
            let trailing = row![
                badge(p, i18n::device_class(app.language, device.class), p.ink_secondary),
                select(
                    p,
                    InterruptPriority::ALL.iter().map(|value| i18n::interrupt_priority(app.language, *value).to_owned()).collect(),
                    Some(i18n::interrupt_priority(app.language, device.priority).to_owned()),
                    move |label: String| {
                        let priority = InterruptPriority::ALL
                            .into_iter()
                            .find(|value| i18n::interrupt_priority(app.language, *value) == label)
                            .unwrap_or(InterruptPriority::Undefined);
                        Message::SetPriority(index, priority)
                    },
                ),
                switch(p, device.msi_enabled, move |on| Message::SetMsi(index, on)),
            ]
            .spacing(10)
            .align_y(Alignment::Center);
            list_row(p, None, device.name.as_str(), device.instance.as_str(), trailing.into(), None)
        })
        .collect();
    group_titled(p, t(app.language, Msg::PcieDevicesTitle), rows, ROW_INSET)
}

fn display(app: &App, p: Palette) -> El<'_> {
    let toggles: Vec<El<'_>> = app
        .display_toggles
        .iter()
        .map(|tweak| {
            let on = app.detections.get(&tweak.id) == Some(&Detection::Applied);
            let id = tweak.id.clone();
            let (name, detail) = i18n::display_toggle(app.language, &tweak.id).unwrap_or((tweak.name.as_str(), tweak.description.as_str()));
            list_row(p, None, name, detail, switch(p, on, move |value| Message::Toggle(id.clone(), value)), None)
        })
        .collect();
    let timer: Vec<El<'_>> = app.timer.map_or_else(Vec::new, |timer| {
        vec![
            list_row(p, None, t(app.language, Msg::CurrentTimer), "NtQueryTimerResolution", mono(format!("{:>7.3} ms", timer.current_ms), p.ink).into(), None),
            list_row(p, None, t(app.language, Msg::FinestTimer), t(app.language, Msg::TimerMaxResolution), mono(format!("{:>7.3} ms", timer.finest_ms), p.ink).into(), None),
            list_row(p, None, t(app.language, Msg::CoarsestTimer), t(app.language, Msg::TimerMinResolution), mono(format!("{:>7.3} ms", timer.coarsest_ms), p.ink).into(), None),
        ]
    });
    column![
        group_titled(p, t(app.language, Msg::GpuAndDisplay), toggles, ROW_INSET),
        group_titled(p, t(app.language, Msg::KernelTimer), timer, ROW_INSET),
    ]
    .spacing(16)
    .into()
}

fn autoruns_view(app: &App, p: Palette) -> El<'_> {
    if app.autoruns.is_empty() {
        return body(t(app.language, Msg::ScanningStartupEntries), p.ink_secondary).into();
    }
    let mut sections = Column::new().spacing(16);
    for source in [EntrySource::RunKey, EntrySource::StartupFolder, EntrySource::Task, EntrySource::Service] {
        let rows: Vec<El<'_>> = app
            .autoruns
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.source == source)
            .map(|(index, entry)| {
                let publisher = entry.publisher.clone().unwrap_or_else(|| t(app.language, Msg::UnsignedPublisher).to_owned());
                // The engine refuses these anyway, so no control pretends otherwise.
                let control: El<'_> = if entry.source == EntrySource::Service && is_protected(&entry.name) {
                    badge(p, t(app.language, Msg::ProtectedBadge), p.ink_tertiary)
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
            sections = sections.push(group_titled(p, i18n::entry_source(app.language, source), rows, ROW_INSET));
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
        button(p, t(app.language, Msg::CleanScan), Some(Icon::Search), Tone::Secondary, Some(Message::CleanScan)),
        button(
            p,
            t(app.language, Msg::CleanSelected),
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
            let files_noun = app.language.plural(
                result.files as usize,
                t(app.language, Msg::FilesCountOne),
                t(app.language, Msg::FilesCountFew),
                t(app.language, Msg::FilesCountMany),
            );
            list_row(
                p,
                Some(check(p, app.clean_selected.contains(target.id), Message::CleanToggled(target.id))),
                i18n::clean_target(app.language, target).0,
                format_2(files_noun, i18n::clean_target(app.language, target).1, &result.files.to_string()),
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
        sections = sections.push(group_titled(p, t(app.language, Msg::PossiblyOrphanedFolders), orphans, 41.0));
    }
    sections.into()
}

fn dns_view(app: &App, p: Palette) -> El<'_> {
    let run = button(p, t(app.language, Msg::DnsRunBenchmark), Some(Icon::Play), Tone::Secondary, (!app.bench_running).then_some(Message::BenchRun));
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
                    format_1(t(app.language, Msg::DnsProtocolFailedFormat), measure.protocol.label())
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
                    row![
                        body(result.id.as_str(), p.ink).width(Length::Fill),
                        mono(result.score().map_or_else(|| t(app.language, Msg::DnsNoAnswer).to_owned(), |score| format!("{score:>7.1} ms")), p.ink)
                    ],
                    track.width(Length::Fixed(fraction * 520.0)),
                    mono(figures, p.ink_tertiary),
                ]
                .spacing(4),
            )
            .padding([9.0, ROW_INSET])
            .into(),
        );
    }
    let mut sections = column![run, caption(t(app.language, Msg::DnsMinAvgJitterCaption), p.ink_secondary)].spacing(10);
    if !rows.is_empty() {
        sections = sections.push(group(p, rows, ROW_INSET));
    }
    sections.into()
}

/// Icon button that keeps a filled background while its page is open.
fn selected_icon_button<'a>(p: Palette, icon: Icon, selected: bool, on_press: Message) -> El<'a> {
    let ink = if selected { p.ink } else { p.ink_secondary };
    press(container(icon.view(15.0, ink)).padding(5).style(move |_| container::Style {
        background: selected.then(|| p.selected.into()),
        border: Border { radius: CONTROL_RADIUS.into(), ..Border::default() },
        ..container::Style::default()
    }))
    .colors(Color { a: 0.06, ..p.ink }, p.selected)
    .on_press(on_press)
    .into()
}

/// Settings row: a title with a wrapped explanation and a control on the right.
fn setting_row<'a>(p: Palette, title: &'a str, detail: &'a str, control: El<'a>) -> El<'a> {
    let mut label = column![body(title, p.ink)].spacing(1).width(Length::Fill);
    if !detail.is_empty() {
        label = label.push(caption(detail, p.ink_tertiary));
    }
    container(row![label, control].spacing(16).align_y(Alignment::Center))
        .width(Length::Fill)
        .padding([10.0, ROW_INSET])
        .into()
}

/// A catalog option in a pick list: shown by its label in the chosen
/// language, compared and sent by its English value.
#[derive(Debug, Clone)]
struct Choice {
    value: String,
    label: String,
}

impl PartialEq for Choice {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn choices(options: &[windowcraft_config::ChoiceOption]) -> Vec<Choice> {
    options.iter().map(|option| Choice { value: option.value().to_owned(), label: option.label.clone() }).collect()
}

fn chosen(options: &[Choice], value: Option<&str>) -> Option<Choice> {
    value.and_then(|value| options.iter().find(|choice| choice.value == value).cloned())
}

const SUPPORT_EMAIL: &str = "support@windowcraft.dev";
const WEBSITE_URL: &str = "https://windowcraft.dev";

fn interval_label(lang: Language, interval: CheckInterval) -> String {
    let msg = match interval {
        CheckInterval::Off => Msg::IntervalOff,
        CheckInterval::Daily => Msg::IntervalDaily,
        CheckInterval::Weekly => Msg::IntervalWeekly,
        CheckInterval::Monthly => Msg::IntervalMonthly,
    };
    t(lang, msg).to_owned()
}

fn settings_page(app: &App, p: Palette) -> El<'_> {
    let lang = app.language;
    let mut themes = Row::new().spacing(6);
    for (appearance, msg) in
        [(Appearance::System, Msg::ThemeSystem), (Appearance::Dark, Msg::ThemeDark), (Appearance::Light, Msg::ThemeLight)]
    {
        themes = themes.push(chip(p, t(lang, msg).to_owned(), app.appearance == appearance, Message::SetAppearance(appearance)));
    }
    let general = vec![
        setting_row(
            p,
            t(lang, Msg::SettingsLanguage),
            t(lang, Msg::SettingsLanguageDetail),
            select(p, Language::ALL.to_vec(), Some(lang), Message::SetLanguage),
        ),
        setting_row(p, t(lang, Msg::SettingsTheme), t(lang, Msg::SettingsThemeDetail), themes.into()),
    ];

    let mut intervals = Row::new().spacing(6);
    for interval in CheckInterval::ALL {
        intervals = intervals.push(chip(
            p,
            interval_label(lang, interval),
            app.settings.interval == interval,
            Message::SetInterval(interval),
        ));
    }
    let last = if app.settings.last_check == 0 {
        t(lang, Msg::SettingsNeverChecked).to_owned()
    } else {
        format_1(t(lang, Msg::SettingsLastCheck), &relative_time(lang, app.settings.last_check))
    };
    let checks = vec![
        setting_row(p, t(lang, Msg::SettingsInterval), t(lang, Msg::SettingsIntervalDetail), intervals.into()),
        setting_row(
            p,
            t(lang, Msg::SettingsAfterUpdates),
            t(lang, Msg::SettingsAfterUpdatesDetail),
            switch(p, app.settings.after_updates, Message::SetAfterUpdates),
        ),
        container(
            row![
                body(last, p.ink_secondary).width(Length::Fill),
                button(p, t(lang, Msg::SettingsCheckNow), Some(Icon::Refresh), Tone::Secondary, Some(Message::CheckDriftNow))
            ]
            .spacing(16)
            .align_y(Alignment::Center),
        )
        .padding([10.0, ROW_INSET])
        .into(),
    ];

    let guide = vec![
        setting_row(
            p,
            t(lang, Msg::SettingsOnboardingAgain),
            t(lang, Msg::SettingsOnboardingAgainDetail),
            button(p, t(lang, Msg::SettingsOnboardingAgainButton), None, Tone::Secondary, Some(Message::ShowOnboardingAgain)),
        ),
        setting_row(
            p,
            t(lang, Msg::SettingsTipsAgain),
            t(lang, Msg::SettingsTipsAgainDetail),
            button(p, t(lang, Msg::SettingsTipsAgainButton), None, Tone::Secondary, Some(Message::ShowTipsAgain)),
        ),
    ];

    // The addresses are not translated, so the buttons show them as they are.
    let about = vec![
        setting_row(
            p,
            t(lang, Msg::SettingsSupport),
            t(lang, Msg::SettingsSupportDetail),
            button(p, SUPPORT_EMAIL, None, Tone::Secondary, Some(Message::Launch(format!("mailto:{SUPPORT_EMAIL}")))),
        ),
        setting_row(
            p,
            t(lang, Msg::SettingsWebsite),
            t(lang, Msg::SettingsWebsiteDetail),
            button(p, "windowcraft.dev", None, Tone::Secondary, Some(Message::Launch(WEBSITE_URL.to_owned()))),
        ),
    ];

    scroll_area(
        p,
        column![
            group_titled(p, t(lang, Msg::SettingsGeneral), general, ROW_INSET),
            group_titled(p, t(lang, Msg::SettingsDriftGroup), checks, ROW_INSET),
            group_titled(p, t(lang, Msg::SettingsGuideGroup), guide, ROW_INSET),
            group_titled(p, t(lang, Msg::SettingsAboutGroup), about, ROW_INSET),
        ]
        .spacing(16),
    )
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

    // The overlay scrollbar sits on the right edge of the scrolled content,
    // so the content keeps 12 px clear for it (see the padding below).
    let mut content = Column::new().spacing(14).width(INSPECTOR_WIDTH - DRAWER_GRIP - 30.0);
    match &app.inspected {
        Some(Inspected::Tweak(id)) => {
            if let Some(tweak) = app.find_tweak(id) {
                let (name, detail) = i18n::display_toggle(app.language, id).unwrap_or((tweak.name.as_str(), tweak.description.as_str()));
                content = content.push(strong(name.to_owned(), p.ink));
                content = content.push(caption(detail.to_owned(), p.ink_secondary));
                if let Some(notice) = tweak.notice.clone() {
                    content = content.push(banner(p, Icon::Alert, p.amber, notice, Vec::new()));
                }
                for row_data in &app.inspect_rows {
                    content = content.push(
                        container(
                            column![
                                mono(row_data.target.as_str(), p.ink).wrapping(iced::widget::text::Wrapping::Glyph),
                                compare(p, t(app.language, Msg::InspectorCurrent), row_data.current.as_str(), if row_data.matches { p.green } else { p.ink }),
                                compare(p, t(app.language, Msg::InspectorTarget), row_data.desired.as_str(), p.ink),
                                compare(p, t(app.language, Msg::InspectorUndo), row_data.undo.as_str(), p.ink_secondary),
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
                    content = content.push(section(t(app.language, Msg::NativeOperations), p.ink_secondary)).push(native);
                }
                // Undo on something never applied would write catalog defaults
                // over whatever this machine really has.
                let state = app.detections.get(&tweak.id);
                let undoable = matches!(state, Some(Detection::Applied | Detection::Partial { .. }))
                    || (state == Some(&Detection::Unknown) && !tweak.undo.is_empty());
                if tweak.irreversible {
                    content = content.push(caption(t(app.language, Msg::CannotUndoAutomatically), p.amber));
                } else if undoable {
                    content = content.push(button(
                        p,
                        t(app.language, Msg::UndoThisTweak),
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
                content = content.push(compare(p, "WinGet", item.winget.as_deref().unwrap_or(t(app.language, Msg::NotAvailableFromSource)), p.ink));
                content = content.push(compare(p, "Chocolatey", item.choco.as_deref().unwrap_or(t(app.language, Msg::NotAvailableFromSource)), p.ink));
                content = content.push(compare(p, t(app.language, Msg::InspectorLicense), if item.foss { t(app.language, Msg::OpenSource) } else { t(app.language, Msg::Proprietary) }, p.ink));
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
                content = content.push(compare(p, t(app.language, Msg::InspectorPackage), package.package_id.as_str(), p.ink));
                content = content.push(compare(p, t(app.language, Msg::InspectorStoreId), package.store_id.as_deref().unwrap_or(t(app.language, Msg::InspectorNone)), p.ink));
                if let Some(warning) = &package.warning {
                    content = content.push(banner(p, Icon::Alert, p.amber, warning.clone(), Vec::new()));
                }
            }
        }
        None => {}
    }

    let header = row![section(t(app.language, Msg::XRayInspector), p.ink_secondary).width(Length::Fill), icon_button(p, Icon::Close, Message::CloseInspector)]
        .align_y(Alignment::Center);

    let drawer = row![
        grip,
        container(
            column![
                header,
                crate::smooth::smooth(
                    iced::widget::scrollable(container(content).padding(Padding { right: 12.0, ..Padding::ZERO }))
                        .height(Length::Fill)
                )
            ]
            .spacing(10),
        )
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
    // Wider than the 72 it was, for labels such as "Valeur actuelle" or
    // "Identyfikator ze sklepu" in the translations.
    row![caption(label, p.ink_tertiary).width(110), mono(value, color).wrapping(iced::widget::text::Wrapping::Glyph)].spacing(6).into()
}

fn confirm_dialog(app: &App, p: Palette) -> El<'_> {
    let (title, detail, action): (String, String, &str) = match &app.confirm {
        Some(Confirm::DisableUpdates) => (
            t(app.language, Msg::ConfirmDisableUpdatesTitle).to_owned(),
            t(app.language, Msg::ConfirmDisableUpdatesDetail).to_owned(),
            t(app.language, Msg::ConfirmDisableUpdatesAction),
        ),
        Some(Confirm::RollbackSession(id)) => (
            format_1(t(app.language, Msg::ConfirmRollbackSessionTitle), &id.to_string()),
            t(app.language, Msg::ConfirmRollbackSessionDetail).to_owned(),
            t(app.language, Msg::ConfirmRollbackSessionAction),
        ),
        Some(Confirm::EraseDisk { number, disk }) => (
            format_1(t(app.language, Msg::ConfirmEraseDiskTitle), &number.to_string()),
            format_1(t(app.language, Msg::ConfirmEraseDiskDetail), &disk.model),
            t(app.language, Msg::ConfirmEraseDiskAction),
        ),
        Some(Confirm::IrreversibleTweak(id)) => (
            t(app.language, Msg::ConfirmIrreversibleTweakTitle).to_owned(),
            format_1(
                t(app.language, Msg::ConfirmIrreversibleTweakDetail),
                app.catalog.tweak(id).map_or(id.as_str(), |tweak| tweak.name.as_str()),
            ),
            t(app.language, Msg::ConfirmIrreversibleTweakAction),
        ),
        Some(Confirm::QuitDuringJob) => (
            t(app.language, Msg::ConfirmQuitDuringJobTitle).to_owned(),
            format_1(
                t(app.language, Msg::ConfirmQuitDuringJobDetail),
                app.job.as_ref().map_or(t(app.language, Msg::UnnamedJob), |job| job.label.as_str()),
            ),
            t(app.language, Msg::ConfirmQuitDuringJobAction),
        ),
        None => (String::new(), String::new(), ""),
    };
    let dialog = container(
        column![
            strong(title, p.ink),
            caption(detail, p.ink_secondary),
            row![
                space().width(Length::Fill),
                button(p, t(app.language, Msg::Cancel), None, Tone::Secondary, Some(Message::Answer(false))),
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
