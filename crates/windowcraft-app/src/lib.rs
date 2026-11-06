// Made by ycangignacy
//! WindowCraft desktop application and CLI.

pub mod actions;
pub mod app;
pub mod cli;
pub mod i18n;
pub mod icons;
pub mod platform;
pub mod press;
pub mod reveal;
pub mod smooth;
pub mod theme;
pub mod ui;
pub mod views;
pub mod vsync;

use app::App;
use clap::Parser;
use iced::{Size, window};
use std::process::ExitCode;
use std::sync::Arc;
use windowcraft_config::Catalog;
use windowcraft_engine::{Engine, sysinfo};

const ICON_RGBA: &[u8] = include_bytes!("../assets/icon-64.rgba");
const ICON_SIZE: u32 = 64;

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

fn run_gui(cli: cli::Cli) -> ExitCode {
    let lang = i18n::detect_language();

    // First, so a second launch never waits on the database lock.
    let Some(_instance) = platform::claim_single_instance() else {
        return ExitCode::SUCCESS;
    };

    let catalog = match Catalog::load() {
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
    let start = app::StartView { page, inspect: cli.inspect, profile, language: lang };
    let info = match sysinfo::query() {
        Ok(info) => info,
        Err(error) => return report_fatal(lang, &error.to_string()),
    };
    let engine = match Engine::system(catalog.clone()) {
        Ok(engine) => Arc::new(engine),
        Err(error) => return report_fatal(lang, &error.to_string()),
    };

    let accessibility = platform::accessibility();
    // Software rendering is the only path that meets the memory and startup
    // budgets, and its GDI surface keeps an alpha channel that DWM composites
    // over Mica.
    let translucent = accessibility.transparency && !accessibility.high_contrast;

    let settings = window::Settings {
        size: Size::new(1280.0, 800.0),
        min_size: Some(Size::new(980.0, 600.0)),
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
