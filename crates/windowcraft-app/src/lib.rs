// Made by Nordan
//! WindowCraft desktop application and CLI.

pub mod actions;
pub mod app;
pub mod background;
pub mod cli;
pub mod i18n;
pub mod icons;
pub mod outside;
pub mod platform;
pub mod press;
pub mod remote;
pub mod reveal;
pub mod settings;
pub mod smooth;
pub mod theme;
pub mod ui;
pub mod views;
pub mod vsync;

use app::App;
use clap::Parser;
use iced::{Point, Rectangle, Size, window};
use std::process::ExitCode;
use std::sync::Arc;
use windowcraft_config::Catalog;
use windowcraft_engine::{Engine, sysinfo};

const ICON_RGBA: &[u8] = include_bytes!("../assets/icon-64.rgba");
const ICON_SIZE: u32 = 64;

/// The size the window opens at, and the smallest it may be dragged to.
const PREFERRED: Size = Size::new(1280.0, 800.0);
const SMALLEST: Size = Size::new(980.0, 600.0);
/// What the title bar and borders add around the client area on Windows 11,
/// measured in the VM: 1008x703 outside for 992x664 inside.
const FRAME: Size = Size::new(16.0, 39.0);
/// Space kept free around the window when the screen is small.
const GAP: f32 = 8.0;

/// Where the first window goes on a screen whose work area (the screen
/// without the taskbar) is `work`, in logical pixels: the client size, the
/// smallest client size, and the outer position. On 1024x768 or 1366x768 the
/// preferred size would run under the taskbar, so it shrinks to what fits,
/// and the window is centred in the work area rather than on the whole
/// monitor, which would push its bottom edge under the taskbar again.
fn window_layout(work: Option<Rectangle>) -> (Size, Size, Option<Point>) {
    let Some(work) = work else {
        return (PREFERRED, SMALLEST, None);
    };
    let room = Size::new(
        (work.width - FRAME.width - 2.0 * GAP).max(1.0),
        (work.height - FRAME.height - 2.0 * GAP).max(1.0),
    );
    let size = Size::new(PREFERRED.width.min(room.width), PREFERRED.height.min(room.height));
    let min = Size::new(SMALLEST.width.min(size.width), SMALLEST.height.min(size.height));
    let x = work.x + ((work.width - size.width - FRAME.width) / 2.0).max(0.0);
    let y = work.y + ((work.height - size.height - FRAME.height) / 2.0).max(0.0);
    (size, min, Some(Point::new(x.round(), y.round())))
}

fn report_fatal(lang: i18n::Language, message: &str) -> ExitCode {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::HSTRING;
    let formatted = i18n::format_1(i18n::t(lang, i18n::Msg::FatalErrorMessage), message);
    let title = i18n::t(lang, i18n::Msg::FatalErrorTitle);
    // SAFETY: Both strings are null-terminated and outlive the call.
    unsafe {
        MessageBoxW(None, &HSTRING::from(&formatted), &HSTRING::from(title), MB_OK | MB_ICONERROR);
    }
    ExitCode::FAILURE
}

/// Picks the renderer before iced starts. Returns true when it may be the GPU.
///
/// iced tries wgpu first and tiny-skia second, or what `ICED_BACKEND` names.
/// Without a Direct3D 12 GPU this sets `ICED_BACKEND=tiny-skia`, see
/// [`platform::hardware_gpu`]. A value the user set is left alone.
fn choose_renderer() -> bool {
    if let Ok(backend) = std::env::var("ICED_BACKEND") {
        return !backend.eq_ignore_ascii_case("tiny-skia");
    }
    if platform::hardware_gpu() {
        return true;
    }
    // SAFETY: Runs first in `run_gui`, before this process starts any thread
    // that could read the environment at the same time.
    unsafe { std::env::set_var("ICED_BACKEND", "tiny-skia") };
    false
}

fn run_gui(cli: cli::Cli) -> ExitCode {
    let gpu = choose_renderer();
    vsync::set_presenter_waits(gpu);
    let lang = i18n::detect_language();
    i18n::set_current(lang);
    i18n::install_job_texts();

    // First, so a second launch never waits on the database lock.
    let Some(_instance) = platform::claim_single_instance() else {
        return ExitCode::SUCCESS;
    };

    // The app shows the catalog in its language; the engine keeps the English
    // one, so the log and the snapshot records read the same everywhere.
    let catalog = match Catalog::load_in(lang.tag()) {
        Ok(catalog) => Arc::new(catalog),
        Err(error) => return report_fatal(lang, &error),
    };
    let english = match Catalog::load() {
        Ok(catalog) => Arc::new(catalog),
        Err(error) => return report_fatal(lang, &error),
    };
    let profile = match cli::load_selection(&cli, &catalog) {
        Ok(selection) => selection.map(|resolved| resolved.profile),
        Err(error) => return report_fatal(lang, &error),
    };
    let page = match (cli.open, &profile) {
        (Some(page), _) => page.0,
        (None, Some(_)) => app::Page::Tweaks,
        (None, None) => app::Page::Applications,
    };
    let accessibility = platform::accessibility();
    // Only tiny-skia presents through a GDI surface that keeps its alpha
    // channel, so only it can show Mica. The GPU path draws solid surfaces.
    let mica = !gpu && accessibility.transparency && !accessibility.high_contrast;
    let guided = cli.open.is_none() && cli.inspect.is_none() && profile.is_none();
    let start = app::StartView { page, inspect: cli.inspect, profile, language: lang, mica, guided };
    let info = match sysinfo::query() {
        Ok(info) => info,
        Err(error) => return report_fatal(lang, &error.to_string()),
    };
    let engine = match Engine::system(english) {
        Ok(engine) => Arc::new(engine),
        Err(error) => return report_fatal(lang, &error.to_string()),
    };
    let translucent = mica;

    let (size, min_size, position) = window_layout(platform::work_area());
    let settings = window::Settings {
        size,
        min_size: Some(min_size),
        position: position.map_or(window::Position::Centered, window::Position::Specific),
        icon: window::icon::from_rgba(ICON_RGBA.to_vec(), ICON_SIZE, ICON_SIZE).ok(),
        transparent: translucent,
        // Closing during a job asks first, see `Message::CloseRequested`.
        exit_on_close_request: false,
        ..window::Settings::default()
    };

    let result = iced::application(
        move || App::new(catalog.clone(), engine.clone(), info.clone(), start.clone()),
        App::update,
        views::view,
    )
    .title("WindowCraft")
    .subscription(App::subscription)
    .theme(|app: &App| if app.palette().dark { iced::Theme::Dark } else { iced::Theme::Light })
    .style(move |app: &App, _theme| {
        let palette = app.palette();
        iced::theme::Style {
            background_color: if translucent { iced::Color::TRANSPARENT } else { palette.base },
            text_color: palette.ink,
        }
    })
    .window(settings)
    .default_font(theme::SANS)
    .antialiasing(true)
    .run();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_fatal(lang, &error.to_string()),
    }
}

/// Runs the CLI when flags ask for it, otherwise the desktop interface.
pub fn entry() -> ExitCode {
    platform::harden_dll_search();
    let has_arguments = std::env::args_os().len() > 1;
    if has_arguments {
        platform::attach_parent_console();
    }
    let cli = match cli::Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            // Clap handles --help and --version through this path too.
            let _ = error.print();
            return if error.use_stderr() { ExitCode::FAILURE } else { ExitCode::SUCCESS };
        }
    };
    if cli.is_headless() { cli::execute(&cli) } else { run_gui(cli) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(x: f32, y: f32, width: f32, height: f32) -> Option<Rectangle> {
        Some(Rectangle { x, y, width, height })
    }

    #[test]
    fn the_first_window_fits_the_work_area_and_keeps_its_size_on_large_screens() {
        assert_eq!(window_layout(None), (PREFERRED, SMALLEST, None));
        // 2560x1440 with a 48 px taskbar: the preferred size, centred.
        let (size, min, position) = window_layout(area(0.0, 0.0, 2560.0, 1392.0));
        assert_eq!((size, min), (PREFERRED, SMALLEST));
        assert_eq!(position, Some(Point::new(632.0, 277.0)));
        // 1024x768 as in the VM: 1008x703 outside, 8 px from the top and the taskbar.
        let (size, min, position) = window_layout(area(0.0, 0.0, 1024.0, 720.0));
        assert_eq!((size, min), (Size::new(992.0, 665.0), SMALLEST));
        assert_eq!(position, Some(Point::new(8.0, 8.0)));
        // 1366x768 at 150 % leaves 910x480 logical pixels; the minimum follows.
        let (size, min, _) = window_layout(area(0.0, 0.0, 910.0, 480.0));
        assert_eq!(size, Size::new(878.0, 425.0));
        assert_eq!(min, size);
        // A taskbar on the left moves the work area, and the window with it.
        let (_, _, position) = window_layout(area(48.0, 0.0, 1318.0, 768.0));
        assert_eq!(position, Some(Point::new(59.0, 8.0)));
    }
}
