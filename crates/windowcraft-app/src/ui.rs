// Made by Nordan
//! Shared view building blocks: grouped inset lists, controls and text styles.

use crate::app::Message;
use crate::icons::Icon;
use crate::press::{corners, press};
use crate::smooth::smooth;
use crate::theme::*;
use iced::widget::text::{IntoFragment, LineHeight, Wrapping};
use iced::widget::{
    Column, Space, Text, checkbox, column, container, pick_list, progress_bar, row, scrollable, space, stack, text, toggler,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding, Shadow, Vector};
use windowcraft_config::NativeOp;

pub type El<'a> = Element<'a, Message>;

pub fn body<'a>(content: impl IntoFragment<'a>, color: Color) -> Text<'a> {
    text(content).size(13).color(color).font(SANS).line_height(LineHeight::Relative(1.45))
}

pub fn strong<'a>(content: impl IntoFragment<'a>, color: Color) -> Text<'a> {
    text(content).size(13).color(color).font(SANS_STRONG).line_height(LineHeight::Relative(1.45))
}

pub fn heading<'a>(content: impl IntoFragment<'a>, color: Color) -> Text<'a> {
    text(content).size(20).color(color).font(SANS_STRONG).line_height(LineHeight::Relative(1.15))
}

pub fn section<'a>(content: impl IntoFragment<'a>, color: Color) -> Text<'a> {
    text(content).size(12).color(color).font(SANS_STRONG).line_height(LineHeight::Relative(1.3))
}

pub fn caption<'a>(content: impl IntoFragment<'a>, color: Color) -> Text<'a> {
    text(content).size(12).color(color).font(SANS).line_height(LineHeight::Relative(1.35))
}

pub fn mono<'a>(content: impl IntoFragment<'a>, color: Color) -> Text<'a> {
    text(content).size(11).color(color).font(MONO).line_height(LineHeight::Relative(1.35))
}

fn hover_tint(p: Palette) -> Color {
    if p.dark { Color { a: 0.04, ..Color::WHITE } } else { Color { a: 0.035, ..Color::BLACK } }
}

fn press_tint(p: Palette) -> Color {
    p.selected
}

/// One-pixel row divider.
fn line<'a>(color: Color, left_inset: f32) -> El<'a> {
    container(
        container(Space::new().width(Length::Fill).height(1))
            .width(Length::Fill)
            .height(1)
            .style(move |_| container::background(color)),
    )
    .padding(Padding { left: left_inset, ..Padding::ZERO })
    .into()
}

/// Grouped inset list with hairlines aligned to the row text.
pub fn group<'a>(p: Palette, rows: Vec<El<'a>>, inset: f32) -> El<'a> {
    // No highlight line along the top: drawn straight across, it ran past the
    // rounded corners and read as a bar above every group.
    let mut list = Column::new();
    let last = rows.len().saturating_sub(1);
    // The hover tint of the first and last rows follows the rounded border.
    let edge = GROUP_RADIUS - 1.0;
    for (index, item) in rows.into_iter().enumerate() {
        if index > 0 {
            list = list.push(line(p.hairline, inset));
        }
        let top = if index == 0 { edge } else { 0.0 };
        let bottom = if index == last { edge } else { 0.0 };
        let radius = iced::border::Radius { top_left: top, top_right: top, bottom_right: bottom, bottom_left: bottom };
        list = list.push(corners(item, radius));
    }
    container(list)
        .width(Length::Fill)
        .clip(true)
        .style(move |_| container::Style {
            background: Some(p.surface.into()),
            border: Border { color: p.border, width: 1.0, radius: GROUP_RADIUS.into() },
            ..container::Style::default()
        })
        .into()
}

pub fn group_titled<'a>(p: Palette, title: impl IntoFragment<'a>, rows: Vec<El<'a>>, inset: f32) -> El<'a> {
    column![
        container(section(title, p.ink_secondary)).padding(Padding { left: 4.0, bottom: 6.0, ..Padding::ZERO }),
        group(p, rows, inset)
    ]
    .into()
}

/// A titled group with a badge after the title.
pub fn group_titled_badged<'a>(p: Palette, title: impl IntoFragment<'a>, tag: El<'a>, rows: Vec<El<'a>>, inset: f32) -> El<'a> {
    column![
        container(row![section(title, p.ink_secondary), tag].spacing(8).align_y(Alignment::Center))
            .padding(Padding { left: 4.0, bottom: 6.0, ..Padding::ZERO }),
        group(p, rows, inset)
    ]
    .into()
}

/// Standard row: optional leading control, title, monospace detail, trailing.
pub fn list_row<'a>(
    p: Palette,
    leading: Option<El<'a>>,
    title: impl IntoFragment<'a>,
    detail: impl IntoFragment<'a>,
    trailing: El<'a>,
    on_press: Option<Message>,
) -> El<'a> {
    let label = container(
        column![body(title, p.ink).wrapping(Wrapping::None), mono(detail, p.ink_tertiary).wrapping(Wrapping::None)].spacing(0),
    )
    .width(Length::Fill)
    .clip(true);
    let mut content = row![].spacing(12).align_y(Alignment::Center);
    if let Some(leading) = leading {
        content = content.push(leading);
    }
    content = content.push(label).push(trailing);
    press(container(content).width(Length::Fill).height(ROW_HEIGHT).padding([0.0, ROW_INSET]).center_y(ROW_HEIGHT))
        .colors(hover_tint(p), press_tint(p))
        .radius(0.0)
        .on_press_maybe(on_press)
        .into()
}

pub fn check<'a>(p: Palette, checked: bool, on_toggle: Message) -> El<'a> {
    checkbox(checked)
        .size(15)
        .on_toggle(move |_| on_toggle.clone())
        .style(move |_, _| checkbox::Style {
            background: if checked { p.accent.into() } else { Background::Color(Color::TRANSPARENT) },
            icon_color: p.on_accent,
            border: Border { color: if checked { p.accent } else { p.ink_tertiary }, width: 1.0, radius: 4.0.into() },
            text_color: None,
        })
        .into()
}

pub fn switch<'a>(p: Palette, on: bool, message: impl Fn(bool) -> Message + 'a) -> El<'a> {
    toggler(on)
        .size(16)
        .on_toggle(message)
        .style(move |_, _| toggler::Style {
            background: if on { p.accent.into() } else { p.selected.into() },
            background_border_width: 0.0,
            background_border_color: Color::TRANSPARENT,
            foreground: Color::WHITE.into(),
            foreground_border_width: 0.0,
            foreground_border_color: Color::TRANSPARENT,
            text_color: None,
            border_radius: None,
            padding_ratio: 0.12,
        })
        .into()
}

pub fn dot<'a>(color: Color) -> El<'a> {
    container(Space::new().width(6).height(6))
        .style(move |_| container::Style {
            background: Some(color.into()),
            border: Border { radius: 3.0.into(), ..Border::default() },
            ..container::Style::default()
        })
        .into()
}

pub fn badge<'a>(p: Palette, label: &'a str, color: Color) -> El<'a> {
    container(mono(label, color))
        .padding([1.0, 5.0])
        .style(move |_| container::Style {
            border: Border { color: Color { a: 0.35, ..color }, width: 1.0, radius: 4.0.into() },
            background: Some(p.elevated.into()),
            ..container::Style::default()
        })
        .into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Secondary,
    Destructive,
}

pub fn button<'a>(p: Palette, label: &'a str, icon: Option<Icon>, tone: Tone, on_press: Option<Message>) -> El<'a> {
    let enabled = on_press.is_some();
    let (fill, ink, border) = match tone {
        Tone::Primary => (p.accent, p.on_accent, p.accent),
        Tone::Secondary => (p.elevated, p.ink, p.border),
        Tone::Destructive => (p.elevated, p.red, p.border),
    };
    let ink = if enabled { ink } else { p.ink_tertiary };
    let fill = if enabled { fill } else { p.elevated };
    let mut content = row![].spacing(6).align_y(Alignment::Center);
    if let Some(icon) = icon {
        content = content.push(icon.view(14.0, ink));
    }
    content = content.push(text(label).size(12).color(ink).font(SANS_STRONG));
    press(container(content).padding([6.0, 11.0]).style(move |_| container::Style {
        background: Some(fill.into()),
        border: Border { color: border, width: 1.0, radius: CONTROL_RADIUS.into() },
        ..container::Style::default()
    }))
    .on_press_maybe(on_press)
    .into()
}

pub fn icon_button<'a>(p: Palette, icon: Icon, on_press: Message) -> El<'a> {
    press(container(icon.view(15.0, p.ink_secondary)).padding(5))
        .colors(hover_tint(p), press_tint(p))
        .on_press(on_press)
        .into()
}

pub fn chip<'a>(p: Palette, label: String, active: bool, on_press: Message) -> El<'a> {
    let (fill, ink) = if active { (p.accent, p.on_accent) } else { (p.elevated, p.ink_secondary) };
    press(container(text(label).size(12).color(ink).font(SANS)).padding([4.0, 10.0]).style(move |_| {
        container::Style {
            background: Some(fill.into()),
            border: Border { color: p.border, width: 1.0, radius: CONTROL_RADIUS.into() },
            ..container::Style::default()
        }
    }))
    .on_press(on_press)
    .into()
}

/// Segmented control whose highlight slides on a spring.
/// Width of each tab in a segmented control, wide enough for the longest
/// label on one line. Labels are 12 px text, which averages under 6 px per
/// character in Latin and Cyrillic; 6.6 leaves room for wide letters.
pub fn tab_width(labels: &[&str], minimum: f32) -> f32 {
    labels.iter().map(|label| label.chars().count() as f32 * 6.6 + 20.0).fold(minimum, f32::max).ceil()
}

pub fn tabs<'a>(p: Palette, labels: Vec<&'a str>, position: f64, width: f32, select: impl Fn(usize) -> Message) -> El<'a> {
    let total = width * labels.len() as f32;
    let pill = container(
        container(Space::new().width(width - 4.0).height(24.0)).style(move |_| container::Style {
            background: Some(p.selected.into()),
            border: Border { color: p.border, width: 1.0, radius: 5.0.into() },
            ..container::Style::default()
        }),
    )
    .padding(Padding { left: position as f32 * width + 2.0, top: 2.0, ..Padding::ZERO });

    let mut buttons = row![];
    for (index, label) in labels.into_iter().enumerate() {
        buttons = buttons.push(
            press(
                container(text(label).size(12).color(p.ink).font(SANS))
                    .width(width)
                    .height(28)
                    .center_x(width)
                    .center_y(28),
            )
            .on_press(select(index)),
        );
    }
    container(stack![buttons].push_under(pill))
        .width(total)
        .height(28)
        .style(move |_| container::Style {
            background: Some(p.elevated.into()),
            border: Border { color: p.border, width: 1.0, radius: CONTROL_RADIUS.into() },
            ..container::Style::default()
        })
        .into()
}

pub fn select<'a, T>(
    p: Palette,
    options: Vec<T>,
    selected: Option<T>,
    on_selected: impl Fn(T) -> Message + 'a,
) -> El<'a>
where
    T: ToString + PartialEq + Clone + 'a,
{
    pick_list(options, selected, on_selected)
        .text_size(12)
        .font(SANS)
        .padding([4.0, 8.0])
        .style(move |_, _| pick_list::Style {
            text_color: p.ink,
            placeholder_color: p.ink_tertiary,
            handle_color: p.ink_secondary,
            background: p.elevated.into(),
            border: Border { color: p.border, width: 1.0, radius: CONTROL_RADIUS.into() },
        })
        .menu_style(move |_| iced::overlay::menu::Style {
            background: p.elevated.into(),
            border: Border { color: p.border, width: 1.0, radius: CONTROL_RADIUS.into() },
            text_color: p.ink,
            selected_text_color: p.on_accent,
            selected_background: p.accent.into(),
            shadow: Shadow::default(),
        })
        .into()
}

/// Bands in the fade over scroll edges.
const FADE_BANDS: usize = 9;
const FADE_BAND_HEIGHT: f32 = 2.0;

/// Soft fade over scroll edges, replacing hard header borders. It is drawn
/// as solid bands of falling opacity: tiny-skia paints a gradient through its
/// slow high-precision pipeline, and this runs on every scrolled frame.
fn fade<'a>(p: Palette, top: bool) -> El<'a> {
    let mut bands = Column::new().width(Length::Fill);
    for index in 0..FADE_BANDS {
        let step = if top { index } else { FADE_BANDS - 1 - index };
        let alpha = 1.0 - (step as f32 + 0.5) / FADE_BANDS as f32;
        let color = Color { a: alpha * p.base.a, ..p.base };
        bands = bands.push(
            container(space()).width(Length::Fill).height(FADE_BAND_HEIGHT).style(move |_| container::background(color)),
        );
    }
    bands.into()
}

/// The thin scrollbar every scrolling list uses.
pub fn scroll_style(p: Palette) -> impl Fn(&iced::Theme, scrollable::Status) -> scrollable::Style {
    move |_, status| {
        let active = !matches!(status, scrollable::Status::Active { .. });
        let thumb = Color { a: if active { 0.4 } else { 0.2 }, ..p.ink_tertiary };
        let rail = scrollable::Rail {
            background: None,
            border: Border::default(),
            scroller: scrollable::Scroller {
                background: thumb.into(),
                border: Border { radius: 3.0.into(), ..Border::default() },
            },
        };
        scrollable::Style {
            container: container::Style::default(),
            vertical_rail: rail,
            horizontal_rail: rail,
            gap: None,
            auto_scroll: scrollable::AutoScroll {
                background: p.elevated.into(),
                border: Border::default(),
                shadow: Shadow::default(),
                icon: p.ink,
            },
        }
    }
}

pub fn scroll_area<'a>(p: Palette, content: impl Into<El<'a>>) -> El<'a> {
    // The credit ends every page, right-aligned under the last group.
    let credit = container(
        text(crate::i18n::t(crate::i18n::current(), crate::i18n::Msg::MadeBy)).size(11).color(p.ink_tertiary).font(SANS),
    )
    .width(Length::Fill)
    .align_x(Alignment::End);
    let content = column![content.into(), credit].spacing(14);
    let body = scrollable(container(content).padding(Padding { top: 8.0, right: 22.0, bottom: 96.0, left: 22.0 }).width(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(scroll_style(p));
    stack![
        smooth(body),
        column![fade(p, true), space().height(Length::Fill), fade(p, false)].width(Length::Fill).height(Length::Fill)
    ]
    .into()
}

pub fn floating_shadow() -> Shadow {
    Shadow { color: Color { a: 0.45, ..Color::BLACK }, offset: Vector::new(0.0, 8.0), blur_radius: 24.0 }
}

pub fn progress<'a>(p: Palette, fraction: f32) -> El<'a> {
    progress_bar(0.0..=1.0, fraction)
        .girth(3)
        .style(move |_| progress_bar::Style {
            background: p.selected.into(),
            bar: p.accent.into(),
            border: Border { radius: 2.0.into(), ..Border::default() },
        })
        .into()
}

pub fn describe_ops(ops: &[NativeOp]) -> Vec<String> {
    ops.iter().map(NativeOp::describe).collect()
}
