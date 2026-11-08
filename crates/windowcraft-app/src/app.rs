// Made by ycangignacy
//! Application state and message handling.

use crate::actions::{self, detect_all, detect_options};
use crate::i18n::{self, Language, Msg, format_1, format_2, t};
use crate::platform::{self, Accessibility};
use crate::settings::{self, CheckInterval, Settings};
use crate::theme::{INSPECTOR_WIDTH, NAV_ROW_HEIGHT, Palette};
use iced::futures::stream;
use iced::keyboard::{self, Key, Modifiers, key};
use iced::{Event, Point, Subscription, Task, event, mouse, widget, window};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use windowcraft_config::{Catalog, Feature, NativeOp, PresetName, Profile, SearchHit, SearchKind, Tweak, TweakKind};
use windowcraft_engine::autoruns::{self, AutorunEntry};
use windowcraft_engine::cleaner::{self, CleanTarget, OrphanCandidate, ScanResult};
use windowcraft_engine::dns_bench::{self, BenchResult, Target};
use windowcraft_engine::error::Result as EngineResult;
use windowcraft_engine::fixes;
use windowcraft_engine::gaming::{self, InterruptPriority, PciDevice, TimerResolution};
use windowcraft_engine::jobs::{self, CancelToken, JobContext, JobEvent, LogRing};
use windowcraft_engine::metadata;
use windowcraft_engine::convert;
use windowcraft_engine::tools::{self, Extension};
use windowcraft_engine::youtube::{self, Format, Stage};
use windowcraft_engine::packages::{self, InstalledProgram, PackageManager};
use windowcraft_engine::snapshot::{ChangeRecord, Session};
use windowcraft_engine::sysinfo::{self, SystemInfo};
use windowcraft_engine::{Detection, Drift, Engine, EngineError, InspectRow, RegPath, TweakAction};
use windowcraft_iso::usb::DiskFacts;
use windowcraft_iso::{AnswerOptions, ImageInfo, StudioSession, pipeline, usb};
use windowcraft_physics::{Axis, DragGesture, MotionMode, Timeline, rubberband};

pub const NAV: usize = 0;
pub const DRAWER: usize = 1;
pub const DOCK: usize = 2;
pub const TAB: usize = 3;
pub const ACCORDION: usize = 4;
pub const DNS_BARS: usize = 5;

/// Motion smaller than this, in logical pixels, is not drawn differently.
const VISIBLE_PIXEL: f64 = 0.25;
/// Bars in the DNS benchmark are about this wide at full length.
const DNS_BAR_PIXELS: f64 = 300.0;

/// Below this release speed the drawer settles by position instead.
const FLICK_THRESHOLD: f64 = 60.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Applications,
    Tweaks,
    System,
    Updates,
    Iso,
    TimeMachine,
    Inspector,
    /// Optional tools, each installed on its own. Last in the sidebar.
    Extensions,
    /// Opened from the gear at the bottom of the sidebar, so not in `ALL`.
    Settings,
}

impl Page {
    pub const ALL: [Self; 8] = [
        Self::Applications,
        Self::Tweaks,
        Self::System,
        Self::Updates,
        Self::Iso,
        Self::TimeMachine,
        Self::Inspector,
        Self::Extensions,
    ];

    pub fn title(self) -> &'static str {
        self.title_in(Language::En)
    }

    pub fn title_in(self, lang: Language) -> &'static str {
        match self {
            Self::Applications => t(lang, Msg::PageApplications),
            Self::Tweaks => t(lang, Msg::PageTweaks),
            Self::System => t(lang, Msg::PageSystem),
            Self::Updates => t(lang, Msg::PageUpdates),
            Self::Iso => t(lang, Msg::PageIso),
            Self::Extensions => t(lang, Msg::PageExtensions),
            Self::TimeMachine => t(lang, Msg::PageTimeMachine),
            Self::Inspector => t(lang, Msg::PageInspector),
            Self::Settings => t(lang, Msg::PageSettings),
        }
    }

    /// Position in the sidebar. Settings comes after every listed page.
    pub(crate) fn index(self) -> usize {
        Self::ALL.iter().position(|page| *page == self).unwrap_or(Self::ALL.len())
    }
}

impl std::str::FromStr for Page {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "applications" | "apps" => Ok(Self::Applications),
            "tweaks" => Ok(Self::Tweaks),
            "system" => Ok(Self::System),
            "updates" => Ok(Self::Updates),
            "iso" => Ok(Self::Iso),
            "extensions" | "misc" => Ok(Self::Extensions),
            "timemachine" => Ok(Self::TimeMachine),
            "inspector" => Ok(Self::Inspector),
            "settings" => Ok(Self::Settings),
            other => Err(format!(
                "unknown module '{other}'; expected applications, tweaks, system, updates, iso, timemachine, inspector, extensions or settings"
            )),
        }
    }
}

/// Where the window opens, from `--open` and `--inspect`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartView {
    pub page: Page,
    pub inspect: Option<String>,
    /// Selection loaded from `--config` or `--preset`, ready to review.
    pub profile: Option<Profile>,
    /// Detected once at startup, see `i18n::detect_language`.
    pub language: Language,
    /// The window is transparent and shows Mica, which only tiny-skia can do.
    pub mica: bool,
}

impl Default for StartView {
    fn default() -> Self {
        Self { page: Page::Applications, inspect: None, profile: None, language: Language::En, mica: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TweakTab {
    Essential,
    Advanced,
    Preferences,
    Appx,
}

impl TweakTab {
    pub const ALL: [Self; 4] = [Self::Essential, Self::Advanced, Self::Preferences, Self::Appx];

    pub fn title(self) -> &'static str {
        self.title_in(Language::En)
    }

    pub fn title_in(self, lang: Language) -> &'static str {
        match self {
            Self::Essential => t(lang, Msg::TabEssential),
            Self::Advanced => t(lang, Msg::TabAdvanced),
            Self::Preferences => t(lang, Msg::TabPreferences),
            Self::Appx => t(lang, Msg::TabAppx),
        }
    }

    pub(crate) fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InspectorTab {
    Interrupts,
    Display,
    Autoruns,
    Cleaner,
    Dns,
}

impl InspectorTab {
    pub const ALL: [Self; 5] = [Self::Interrupts, Self::Display, Self::Autoruns, Self::Cleaner, Self::Dns];

    pub fn title(self) -> &'static str {
        self.title_in(Language::En)
    }

    pub fn title_in(self, lang: Language) -> &'static str {
        match self {
            Self::Interrupts => t(lang, Msg::TabMsiMode),
            Self::Display => t(lang, Msg::TabDisplay),
            Self::Autoruns => t(lang, Msg::TabAutoruns),
            Self::Cleaner => t(lang, Msg::TabCleaner),
            Self::Dns => t(lang, Msg::TabDns),
        }
    }

    pub(crate) fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inspected {
    Tweak(String),
    App(String),
    Feature(String),
    Appx(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirm {
    DisableUpdates,
    RollbackSession(u64),
    EraseDisk { number: u32, model: String },
    IrreversibleTweak(String),
    QuitDuringJob,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoFlag {
    Bypass,
    LocalAccount,
    DarkMode,
    NoEncryption,
    PauseUpdates,
    InjectDrivers,
    BundleApps,
}

#[derive(Debug, Clone)]
pub struct ChangeView {
    pub record: ChangeRecord,
    pub live: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    Frame(Instant),
    WindowOpened(window::Id),
    WindowHandle(u64),
    Navigate(Page),
    Keyboard(keyboard::Event),
    Query(String),
    SearchSelected(SearchKind, String),
    CycleAppearance,
    SystemThemeChanged,
    Relaunch,
    CloseRequested,
    DismissNotice,
    GripEnter,
    GripExit,
    GripPressed,
    Pointer(Point),
    PointerUp,
    PointerLost,
    // Applications
    AppToggled(String),
    Category(String),
    Manager(PackageManager),
    ClearApps,
    ShowInstalled,
    InstalledLoaded(Vec<InstalledProgram>),
    UpgradesLoaded(Vec<(String, String)>),
    InstallApps,
    UninstallApps,
    UpgradeAll,
    // Tweaks
    Tab(TweakTab),
    TweakChecked(String),
    Preset(PresetName),
    ClearTweaks,
    DetectApplied,
    Detected(BTreeMap<String, Detection>, BTreeMap<String, String>),
    ApplyTweaks,
    Toggle(String, bool),
    Segment(String, String),
    Dns(String),
    RunAction(String),
    Undo(String),
    AppxChecked(String),
    AppxDefaults,
    RemoveAppx,
    InstallAppx,
    ImportProfile,
    ProfilePicked(Option<PathBuf>),
    ExportProfile,
    ExportReport,
    // System
    FeatureChecked(String),
    InstallFeatures,
    RunFix(String),
    FixMode(String),
    Launch(String),
    // Updates
    UpdateMode(String),
    UpdateModeLoaded(String),
    // ISO
    IsoPath(String),
    IsoBrowse,
    IsoPicked(Option<PathBuf>),
    IsoDownloadUrl(String),
    IsoDownloadHash(String),
    IsoDownload,
    IsoOpen,
    IsoImages(Result<Vec<ImageInfo>, String>),
    IsoEdition(u32),
    IsoFlag(IsoFlag, bool),
    IsoOutput(String),
    IsoBrowseOutput,
    IsoOutputPicked(Option<PathBuf>),
    IsoBuild,
    IsoRefreshDisks,
    IsoDisks(Vec<DiskFacts>),
    IsoWriteUsb(u32),
    // Extensions
    InstallExtension(Extension),
    UninstallExtension(Extension),
    StripPick,
    StripPicked(Vec<PathBuf>),
    VideoUrl(String),
    VideoFormat(Format),
    VideoFolder(String),
    VideoBrowseFolder,
    VideoFolderPicked(Option<PathBuf>),
    VideoDownload,
    ConvertTarget(convert::Target),
    ConvertPick,
    ConvertPicked(Vec<PathBuf>),
    // Time Machine
    Refreshed(Vec<Session>, Vec<Drift>),
    SelectSession(u64),
    Changes(u64, Vec<ChangeView>),
    RollbackChange(u64, u32),
    RepairDrift,
    // Inspector page
    InspectorTab(InspectorTab),
    Devices(Vec<PciDevice>, Option<TimerResolution>),
    SetMsi(usize, bool),
    SetPriority(usize, InterruptPriority),
    Autoruns(Vec<AutorunEntry>),
    SetAutorun(usize, bool),
    CleanScan,
    CleanResults(Vec<(CleanTarget, ScanResult)>, Vec<OrphanCandidate>),
    CleanToggled(&'static str),
    OrphanToggled(PathBuf),
    CleanRun,
    BenchRun,
    BenchResults(Result<Vec<BenchResult>, String>),
    // Drawer
    Inspect(Inspected),
    InspectRows(Vec<InspectRow>),
    CloseInspector,
    // Jobs
    Job(JobEvent),
    CancelJob,
    Ask(Confirm),
    Answer(bool),
    Notice(String),
    SetLanguage(Language),
    // Settings
    SetAppearance(Appearance),
    SetInterval(CheckInterval),
    SetAfterUpdates(bool),
    ScheduleApplied(Result<(), String>),
    CheckDriftNow,
    DriftChecked(Vec<Drift>, String),
}

/// A selection that a job consumes when it succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Queue {
    Apps,
    Tweaks,
    Appx,
    Features,
}

pub struct ActiveJob {
    pub label: String,
    pub fraction: f32,
    pub step: String,
    pub cancel: CancelToken,
    pub log: LogRing,
    pub queue: Option<Queue>,
    /// Keeps the job out of windowcraft.log, because its steps and summary
    /// name the user's files or the video.
    pub private: bool,
}

pub struct IsoState {
    pub path: String,
    pub url: String,
    pub hash: String,
    pub images: Vec<ImageInfo>,
    pub selected: Option<u32>,
    pub options: AnswerOptions,
    pub inject_drivers: bool,
    pub bundle_apps: bool,
    pub output: String,
    pub disks: Vec<DiskFacts>,
}

pub struct MiscState {
    pub url: String,
    pub format: Format,
    pub folder: String,
    pub target: convert::Target,
    /// Read when the page opens and after each job, not on every frame.
    pub installed: Vec<Extension>,
    /// ffmpeg is there, from the additional formats or the YouTube downloader.
    pub ffmpeg: bool,
}

impl MiscState {
    pub fn has(&self, extension: Extension) -> bool {
        self.installed.contains(&extension)
    }
}

pub fn format_index(format: Format) -> f64 {
    match format {
        Format::Mp4 => 0.0,
        Format::Mp3 => 1.0,
    }
}

pub struct App {
    pub catalog: Arc<Catalog>,
    pub engine: Arc<Engine>,
    pub info: SystemInfo,
    pub accessibility: Accessibility,
    /// Fixed at startup with the renderer, see `StartView::mica`.
    mica: bool,
    pub appearance: Appearance,
    pub language: Language,
    pub settings: Settings,
    pub window: Option<u64>,
    pub page: Page,
    pub query: String,
    pub hits: Vec<SearchHit>,
    pub notice: Option<String>,
    /// The notice text when it reports a failed job, so it gets a warning icon.
    pub notice_failed: Option<String>,
    pub modifiers: Modifiers,
    pub timeline: Timeline,
    epoch: Instant,
    // Applications
    pub categories: BTreeSet<String>,
    pub selected_apps: BTreeSet<String>,
    pub manager: PackageManager,
    /// Installed version per catalog id. Matched once when the scan arrives,
    /// because `view` runs on every animation frame.
    pub installed: BTreeMap<String, String>,
    pub upgrades: BTreeMap<String, String>,
    pub checking_upgrades: bool,
    // Tweaks
    pub tab: TweakTab,
    pub selected_tweaks: BTreeSet<String>,
    pub detections: BTreeMap<String, Detection>,
    pub options: BTreeMap<String, String>,
    pub dns_choice: String,
    pub selected_appx: BTreeSet<String>,
    // System
    pub selected_features: BTreeSet<String>,
    pub fix_modes: BTreeMap<String, String>,
    pub update_mode: String,
    pub iso: IsoState,
    pub misc: MiscState,
    // Time Machine
    pub sessions: Vec<Session>,
    pub drift: Vec<Drift>,
    pub open_session: Option<u64>,
    pub changes: Vec<ChangeView>,
    // Inspector page
    pub inspector_tab: InspectorTab,
    pub devices: Vec<PciDevice>,
    pub timer: Option<TimerResolution>,
    pub autoruns: Vec<AutorunEntry>,
    pub clean_results: Vec<(CleanTarget, ScanResult)>,
    pub clean_selected: BTreeSet<&'static str>,
    pub orphans: Vec<OrphanCandidate>,
    pub selected_orphans: BTreeSet<PathBuf>,
    pub bench: Vec<BenchResult>,
    pub bench_running: bool,
    pub display_toggles: Vec<Tweak>,
    // Drawer
    pub inspected: Option<Inspected>,
    pub inspect_rows: Vec<InspectRow>,
    grip_hover: bool,
    drag: DragGesture,
    /// Unknown until the listener reports a move. A press before that starts
    /// the drag on the next move, because a stale origin would make it jump.
    pointer: Option<Point>,
    drag_armed: bool,
    select_applied: bool,
    // Jobs
    pub job: Option<ActiveJob>,
    pub confirm: Option<Confirm>,
    /// Set once the user chose to quit during a job, which then gets to stop first.
    pub quit_pending: bool,
    pub needs_elevation: bool,
    pub last_log_label: String,
    pub last_log: Vec<String>,
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// Progress share and localized step line while a tool is fetched.
fn tool_step(lang: Language, stage: tools::Stage) -> (f32, String) {
    match stage {
        tools::Stage::Fetching(tool, received, Some(total)) if total > 0 => (
            received as f32 / total as f32,
            i18n::format_3(t(lang, Msg::StepFetchingTool), tool.name(), &(received >> 20).to_string(), &(total >> 20).to_string()),
        ),
        tools::Stage::Fetching(tool, received, _) => {
            (0.0, format_2(t(lang, Msg::StepFetchingToolNoTotal), tool.name(), &(received >> 20).to_string()))
        }
        tools::Stage::Unpacking(tool) => (1.0, format_1(t(lang, Msg::StepUnpackingTool), tool.name())),
        tools::Stage::Retrying(tool, attempt) => (0.0, format_2(t(lang, Msg::StepRetrying), tool.name(), &attempt.to_string())),
    }
}

/// Progress share and localized step line for a downloader stage.
fn stage_step(lang: Language, stage: Stage) -> (f32, String) {
    match stage {
        Stage::Tools(stage) => tool_step(lang, stage),
        Stage::Downloading(fraction, part) => (
            fraction,
            format_2(t(lang, Msg::StepDownloadingPart), &part.to_string(), &format!("{:.0}", fraction * 100.0)),
        ),
        Stage::Converting => (1.0, t(lang, Msg::StepConverting).to_owned()),
        Stage::Cleaning => (1.0, t(lang, Msg::StepCleaningMetadata).to_owned()),
    }
}

pub fn extension_name(lang: Language, extension: Extension) -> &'static str {
    match extension {
        Extension::Stripper => t(lang, Msg::MiscStripperGroup),
        Extension::Converter => t(lang, Msg::ConverterGroup),
        Extension::Ffmpeg => t(lang, Msg::FfmpegGroup),
        Extension::YouTube => t(lang, Msg::MiscDownloaderGroup),
    }
}

/// Turns a network failure into a sentence about GitHub. The reqwest text
/// names the status in brackets, for example "(504 Gateway Timeout)".
fn readable(lang: Language, error: EngineError) -> EngineError {
    match error {
        EngineError::Network(detail) => {
            let status = detail
                .split_once('(')
                .and_then(|(_, rest)| rest.split_once(')'))
                .map(|(status, _)| status.to_owned())
                .filter(|status| status.chars().next().is_some_and(|c| c.is_ascii_digit()))
                .unwrap_or(detail);
            EngineError::Message(format_1(t(lang, Msg::NoticeGithubUnavailable), &status))
        }
        other => other,
    }
}

/// Converts each file next to itself. Fails only when no file could be converted.
fn convert_files(lang: Language, files: &[PathBuf], target: convert::Target, context: &JobContext) -> EngineResult<String> {
    // WindowCraft converts what it can itself; ffmpeg, when installed, does the rest.
    let tools = tools::Tools::in_dir(&tools::tools_dir());
    let ffmpeg = tools.has(tools::Tool::Ffmpeg).then_some(&tools);
    let mut saved: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let count = files.len() as f32;
    for (index, file) in files.iter().enumerate() {
        context.cancel.check()?;
        let step = format!("{} / {}: {}", index + 1, files.len(), file_name(file));
        context.progress(index as f32 / count, step.clone());
        let result = convert::convert_file(ffmpeg, file, target, &context.cancel, |share| {
            context.progress((index as f32 + share) / count, step.clone());
        });
        match result {
            Ok(output) => saved.push(output),
            Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
            Err(error) => {
                let reason = match &error {
                    EngineError::Invalid(kind) if kind == "NoPicture" => t(lang, Msg::ConvertNoPicture).to_owned(),
                    EngineError::Invalid(kind) if kind == "NoVideo" => t(lang, Msg::ConvertNoVideo).to_owned(),
                    EngineError::Invalid(kind) if kind == "NoAudio" => t(lang, Msg::ConvertNoAudio).to_owned(),
                    EngineError::Unsupported(_) => t(lang, Msg::ConvertUnreadable).to_owned(),
                    EngineError::NotFound(what) if what == "ffmpeg" => t(lang, Msg::ConvertNeedsFfmpeg).to_owned(),
                    other => other.to_string(),
                };
                failed.push(format!("{}: {reason}", file_name(file)));
            }
        }
    }
    let Some(first) = saved.first() else {
        return Err(EngineError::Message(failed.first().cloned().unwrap_or_default()));
    };
    let mut summary = format_2(t(lang, Msg::NoticeConverted), &saved.len().to_string(), &file_name(first));
    if let Some(error) = failed.first() {
        summary.push(' ');
        summary.push_str(&format_2(t(lang, Msg::NoticeFailures), &failed.len().to_string(), error));
    }
    Ok(summary)
}

/// Writes a clean copy of each file. Fails only when no file could be cleaned.
fn strip_files(lang: Language, files: &[PathBuf], context: &JobContext) -> EngineResult<String> {
    let ffmpeg = tools::installed_ffmpeg();
    let mut saved: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for (index, file) in files.iter().enumerate() {
        context.cancel.check()?;
        context.progress(index as f32 / files.len() as f32, format!("{} / {}", index + 1, files.len()));
        match metadata::strip_file(file, ffmpeg.as_deref(), &context.cancel) {
            Ok(result) => saved.push(result.output),
            Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
            Err(EngineError::NotFound(_)) => {
                failed.push(format!("{}: {}", file_name(file), t(lang, Msg::MiscStripperFormats)));
            }
            Err(error) => failed.push(format!("{}: {error}", file_name(file))),
        }
    }
    let Some(first) = saved.first() else {
        return Err(EngineError::Message(failed.first().cloned().unwrap_or_default()));
    };
    let mut summary = format_2(t(lang, Msg::NoticeStripped), &saved.len().to_string(), &file_name(first));
    if let Some(error) = failed.first() {
        summary.push(' ');
        summary.push_str(&format_2(t(lang, Msg::NoticeFailures), &failed.len().to_string(), error));
    }
    Ok(summary)
}

/// Input box id for `Ctrl+K`.
pub fn search_id() -> widget::Id {
    widget::Id::new("global-search")
}

fn work_dir() -> PathBuf {
    windowcraft_engine::win::local_app_dir().join("iso")
}

impl App {
    pub fn new(catalog: Arc<Catalog>, engine: Arc<Engine>, info: SystemInfo, start: StartView) -> (Self, Task<Message>) {
        let StartView { page, inspect, profile, language, mica } = start;
        let accessibility = platform::accessibility();
        apply_motion_settings(accessibility);
        let mut timeline = Timeline::new([0.0, 0.0, 0.0, 0.0, 0.0]);
        // Each channel rests once it moves less than about a quarter pixel.
        for (channel, precision) in [
            (NAV, VISIBLE_PIXEL / f64::from(NAV_ROW_HEIGHT)),
            (DRAWER, VISIBLE_PIXEL),
            (DOCK, VISIBLE_PIXEL / 90.0),
            (TAB, VISIBLE_PIXEL / 100.0),
            (ACCORDION, VISIBLE_PIXEL),
        ] {
            timeline.channels[channel].set_precision(precision);
        }
        timeline.mode = if accessibility.animations { MotionMode::Spring } else { MotionMode::Reduced };
        timeline.channels[NAV].snap(page.index() as f64);
        let resumed = StudioSession::resume(&work_dir());
        let saved = settings::load();
        let mut app = Self {
            catalog,
            engine,
            info,
            accessibility,
            mica,
            appearance: saved.appearance,
            language,
            settings: saved,
            window: None,
            page: Page::Applications,
            query: String::new(),
            hits: Vec::new(),
            notice: None,
            notice_failed: None,
            modifiers: Modifiers::default(),
            timeline,
            epoch: Instant::now(),
            categories: BTreeSet::new(),
            selected_apps: BTreeSet::new(),
            manager: PackageManager::Winget,
            installed: BTreeMap::new(),
            upgrades: BTreeMap::new(),
            checking_upgrades: false,
            tab: TweakTab::Essential,
            selected_tweaks: BTreeSet::new(),
            detections: BTreeMap::new(),
            options: BTreeMap::new(),
            dns_choice: "Default".to_owned(),
            selected_appx: BTreeSet::new(),
            selected_features: BTreeSet::new(),
            fix_modes: BTreeMap::new(),
            update_mode: "default".to_owned(),
            iso: IsoState {
                path: resumed.as_ref().map_or_else(String::new, |session| session.source_iso.display().to_string()),
                url: String::new(),
                hash: String::new(),
                images: Vec::new(),
                selected: None,
                options: resumed.map_or_else(AnswerOptions::default, |session| session.options),
                inject_drivers: false,
                bundle_apps: false,
                output: String::new(),
                disks: Vec::new(),
            },
            misc: MiscState {
                url: String::new(),
                format: Format::Mp4,
                folder: youtube::downloads_dir().display().to_string(),
                target: convert::Target::Png,
                installed: Vec::new(),
                ffmpeg: false,
            },
            sessions: Vec::new(),
            drift: Vec::new(),
            open_session: None,
            changes: Vec::new(),
            inspector_tab: InspectorTab::Interrupts,
            devices: Vec::new(),
            timer: None,
            autoruns: Vec::new(),
            clean_results: Vec::new(),
            clean_selected: BTreeSet::new(),
            orphans: Vec::new(),
            selected_orphans: BTreeSet::new(),
            bench: Vec::new(),
            bench_running: false,
            display_toggles: gaming::display_toggles(),
            inspected: None,
            inspect_rows: Vec::new(),
            grip_hover: false,
            drag: DragGesture::new(Axis::Horizontal),
            pointer: None,
            drag_armed: false,
            select_applied: false,
            job: None,
            confirm: None,
            quit_pending: false,
            needs_elevation: false,
            last_log_label: String::new(),
            last_log: Vec::new(),
        };
        if let Some(profile) = profile {
            app.apply_loaded_profile(profile);
        }
        let navigation = app.navigate(page);
        let mut startup = vec![app.refresh_detection(), app.refresh_time_machine(), app.refresh_update_mode(), navigation];
        if let Some(id) = inspect {
            startup.push(app.open_inspector(Inspected::Tweak(id)));
        }
        let startup = Task::batch(startup);
        (app, startup)
    }

    pub fn palette(&self) -> Palette {
        let dark = match self.appearance {
            Appearance::System => !self.accessibility.light_theme,
            Appearance::Dark => true,
            Appearance::Light => false,
        };
        if self.accessibility.high_contrast {
            return Palette::HIGH_CONTRAST;
        }
        let palette = if dark { Palette::DARK } else { Palette::LIGHT };
        if self.translucent() { palette } else { palette.solid() }
    }

    /// Mica needs both the Windows setting and the software renderer.
    fn translucent(&self) -> bool {
        self.mica && self.accessibility.transparency
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            window::open_events().map(Message::WindowOpened),
            window::close_requests().map(|_| Message::CloseRequested),
            // Raw, because a focused text field captures Escape otherwise.
            event::listen_raw(keyboard_event),
            iced::system::theme_changes().map(|_| Message::SystemThemeChanged),
        ];
        // No frame source exists unless something is moving, so an idle app
        // sleeps in the message loop at 0% CPU. While it runs, frames follow
        // the display refresh instead of the speed of the CPU.
        if self.timeline.needs_frame() {
            subscriptions.push(crate::vsync::animation_frames().map(Message::Frame));
        }
        if self.grip_hover || self.drag.is_active() || self.drag_armed {
            subscriptions.push(event::listen_with(pointer_event));
        }
        Subscription::batch(subscriptions)
    }

    fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(Arc<Engine>) -> T + Send + 'static,
        done: impl Fn(T) -> Message + Send + 'static,
    ) -> Task<Message> {
        let engine = self.engine.clone();
        let lang = self.language;
        Task::future(async move {
            match tokio::task::spawn_blocking(move || work(engine)).await {
                Ok(value) => done(value),
                Err(error) => Message::Notice(format_1(t(lang, Msg::NoticeBackgroundTaskFailed), &error.to_string())),
            }
        })
    }

    fn refresh_detection(&self) -> Task<Message> {
        self.blocking(|engine| (detect_all(&engine), detect_options(&engine)), |(states, options)| {
            Message::Detected(states, options)
        })
    }

    fn refresh_time_machine(&self) -> Task<Message> {
        self.blocking(
            |engine| (engine.store.sessions().unwrap_or_default(), engine.detect_drift().unwrap_or_default()),
            |(sessions, drift)| Message::Refreshed(sessions, drift),
        )
    }

    fn refresh_update_mode(&self) -> Task<Message> {
        self.blocking(
            |engine| engine.current_update_mode().unwrap_or("default").to_owned(),
            Message::UpdateModeLoaded,
        )
    }

    fn refresh_all(&self) -> Task<Message> {
        Task::batch([self.refresh_detection(), self.refresh_time_machine(), self.refresh_update_mode()])
    }

    fn start_job(
        &mut self,
        label: &str,
        body: impl FnOnce(&JobContext) -> EngineResult<String> + Send + 'static,
    ) -> Task<Message> {
        self.start_queue_job(label, None, body)
    }

    /// Starts a job that writes nothing to windowcraft.log.
    fn start_private_job(
        &mut self,
        label: &str,
        body: impl FnOnce(&JobContext) -> EngineResult<String> + Send + 'static,
    ) -> Task<Message> {
        let task = self.start_queue_job(label, None, body);
        if let Some(job) = &mut self.job {
            job.private = true;
        }
        task
    }

    /// Starts a job. `queue` is cleared only if this job succeeds, so a quick
    /// toggle never wipes a selection the user is still building.
    fn start_queue_job(
        &mut self,
        label: &str,
        queue: Option<Queue>,
        body: impl FnOnce(&JobContext) -> EngineResult<String> + Send + 'static,
    ) -> Task<Message> {
        if self.job.is_some() {
            self.notice = Some(t(self.language, Msg::NoticeAnotherOperationRunning).to_owned());
            return Task::none();
        }
        let handle = jobs::spawn(label, body);
        self.job = Some(ActiveJob {
            label: label.to_owned(),
            fraction: 0.0,
            step: t(self.language, Msg::JobStarting).to_owned(),
            cancel: handle.cancel.clone(),
            log: handle.log.clone(),
            queue,
            private: false,
        });
        self.update_dock();
        if let Some(window) = self.window {
            platform::taskbar_progress(window, Some(0.0));
        }
        let events = handle.into_events();
        Task::stream(stream::unfold(events, |mut events| async move {
            events.recv().await.map(|event| (Message::Job(event), events))
        }))
    }

    fn queued_count(&self) -> usize {
        match self.page {
            Page::Applications => self.selected_apps.len(),
            Page::Tweaks if self.tab == TweakTab::Appx => self.selected_appx.len(),
            Page::Tweaks => self.selected_tweaks.len(),
            Page::System => self.selected_features.len(),
            _ => 0,
        }
    }

    pub fn dock_visible(&self) -> bool {
        self.job.is_some() || self.queued_count() > 0
    }

    fn update_dock(&mut self) {
        let target = if self.dock_visible() { 1.0 } else { 0.0 };
        let mode = self.timeline.mode;
        self.timeline.channels[DOCK].animate_to(target, mode);
    }

    fn animate(&mut self, channel: usize, target: f64) {
        let mode = self.timeline.mode;
        self.timeline.channels[channel].animate_to(target, mode);
    }

    fn navigate(&mut self, page: Page) -> Task<Message> {
        self.page = page;
        self.query.clear();
        self.hits.clear();
        if page != Page::Settings {
            self.animate(NAV, page.index() as f64);
        }
        // Every page with a segmented control shares one tab indicator.
        match page {
            Page::Applications => self.animate(TAB, manager_index(self.manager)),
            Page::Tweaks => self.animate(TAB, self.tab.index() as f64),
            Page::Inspector => self.animate(TAB, self.inspector_tab.index() as f64),
            Page::Extensions => {
                self.animate(TAB, format_index(self.misc.format));
                self.misc.installed = tools::installed();
                self.misc.ffmpeg = tools::installed_ffmpeg().is_some();
            }
            _ => {}
        }
        self.update_dock();
        match page {
            Page::TimeMachine => self.refresh_time_machine(),
            Page::Inspector => self.load_inspector_tab(),
            _ => Task::none(),
        }
    }

    fn load_inspector_tab(&self) -> Task<Message> {
        match self.inspector_tab {
            InspectorTab::Interrupts | InspectorTab::Display => self.blocking(
                |engine| (gaming::pci_devices(&engine.scope).unwrap_or_default(), gaming::timer_resolution().ok()),
                |(devices, timer)| Message::Devices(devices, timer),
            ),
            InspectorTab::Autoruns => self.blocking(|_| autoruns::scan(), Message::Autoruns),
            _ => Task::none(),
        }
    }

    fn close_inspector(&mut self) {
        self.inspected = None;
        self.animate(DRAWER, 0.0);
    }

    fn open_inspector(&mut self, inspected: Inspected) -> Task<Message> {
        self.animate(DRAWER, f64::from(INSPECTOR_WIDTH));
        self.inspected = Some(inspected.clone());
        self.inspect_rows.clear();
        self.load_inspect_rows(inspected)
    }

    fn load_inspect_rows(&self, inspected: Inspected) -> Task<Message> {
        let Inspected::Tweak(id) = inspected else {
            return Task::none();
        };
        let tweak = self.find_tweak(&id);
        let option = self.options.get(&id).cloned();
        self.blocking(
            move |engine| {
                let Some(tweak) = tweak else { return Vec::new() };
                let action = match tweak.kind {
                    TweakKind::Toggle => TweakAction::Set(true),
                    TweakKind::Segmented => TweakAction::Select(option.unwrap_or_default()),
                    _ => TweakAction::Apply,
                };
                engine.inspect(&tweak, &action).unwrap_or_default()
            },
            Message::InspectRows,
        )
    }

    /// Looks a tweak up in the catalog and the built-in display toggles.
    pub fn find_tweak(&self, id: &str) -> Option<Tweak> {
        self.catalog.tweak(id).or_else(|| self.display_toggles.iter().find(|tweak| tweak.id == id)).cloned()
    }

    fn run_items(&mut self, label: &str, items: Vec<(Tweak, TweakAction)>) -> Task<Message> {
        self.run_items_for(label, None, items)
    }

    fn run_items_for(&mut self, label: &str, queue: Option<Queue>, items: Vec<(Tweak, TweakAction)>) -> Task<Message> {
        let engine = self.engine.clone();
        let session_label = label.to_owned();
        let lang = self.language;
        self.start_queue_job(label, queue, move |context| {
            let total = items.len().max(1) as f32;
            let borrowed: Vec<(&Tweak, TweakAction)> = items.iter().map(|(tweak, action)| (tweak, action.clone())).collect();
            let report = engine.run(&session_label, &borrowed, &context.cancel, |index, name| {
                context.progress(index as f32 / total, name);
            })?;
            if report.failed.is_empty() {
                Ok(format_1(t(lang, Msg::NoticeItemsApplied), &report.applied.len().to_string()))
            } else {
                let (name, reason) = &report.failed[0];
                let first = format!("{name}: {reason}");
                Err(EngineError::Invalid(format_2(t(lang, Msg::NoticeFailures), &report.failed.len().to_string(), &first)))
            }
        })
    }

    fn selected_profile(&self) -> Profile {
        let mut profile = Profile::new();
        profile.tweaks = self.selected_tweaks.iter().cloned().collect();
        profile.apps = self.selected_apps.iter().cloned().collect();
        profile.features = self.selected_features.iter().cloned().collect();
        profile.appx_remove = self.selected_appx.iter().cloned().collect();
        profile.segmented = self.options.iter().filter(|(_, option)| option.as_str() != "Enabled").map(|(k, v)| (k.clone(), v.clone())).collect();
        if self.dns_choice != "Default" {
            profile.dns = Some(self.dns_choice.clone());
        }
        profile
    }

    fn apply_loaded_profile(&mut self, profile: Profile) {
        self.selected_tweaks = profile.tweaks.into_iter().collect();
        self.selected_apps = profile.apps.into_iter().collect();
        self.selected_features = profile.features.into_iter().collect();
        self.selected_appx = profile.appx_remove.into_iter().collect();
        if let Some(dns) = profile.dns {
            self.dns_choice = dns;
        }
        self.update_dock();
    }

    fn studio_session(&self) -> StudioSession {
        let mut session = StudioSession::new(PathBuf::from(&self.iso.path), self.iso.options.clone());
        session.inject_drivers = self.iso.inject_drivers;
        if self.iso.bundle_apps {
            let mut profile = Profile::new();
            profile.apps = self.selected_apps.iter().cloned().collect();
            session.profile = Some(profile);
        }
        session
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Frame(now) => {
                let seconds = now.saturating_duration_since(self.epoch).as_secs_f64();
                self.timeline.tick(seconds);
            }
            Message::WindowOpened(id) => return window::raw_id::<Message>(id).map(Message::WindowHandle),
            Message::WindowHandle(handle) => {
                self.window = Some(handle);
                platform::apply_backdrop(handle, self.palette().dark, self.translucent());
            }
            Message::Navigate(page) => return self.navigate(page),
            Message::Keyboard(event) => return self.on_keyboard(event),
            Message::Query(query) => {
                self.hits = windowcraft_config::search(&self.catalog, &query, 60);
                self.query = query;
            }
            Message::SearchSelected(kind, id) => {
                self.query.clear();
                self.hits.clear();
                let (page, inspected) = match kind {
                    SearchKind::App => (Page::Applications, Inspected::App(id)),
                    SearchKind::Fix => (Page::System, Inspected::Feature(id)),
                    SearchKind::Appx => (Page::Tweaks, Inspected::Appx(id)),
                    SearchKind::Tweak | SearchKind::RegistryKey | SearchKind::Service => {
                        if let Some(tab) = self.catalog.tweak(&id).map(|tweak| tab_of(tweak.group)) {
                            self.tab = tab;
                            self.animate(TAB, tab.index() as f64);
                        }
                        (Page::Tweaks, Inspected::Tweak(id))
                    }
                };
                let navigation = self.navigate(page);
                return Task::batch([navigation, self.open_inspector(inspected)]);
            }
            Message::CycleAppearance => {
                let next = match self.appearance {
                    Appearance::System => Appearance::Dark,
                    Appearance::Dark => Appearance::Light,
                    Appearance::Light => Appearance::System,
                };
                self.set_appearance(next);
            }
            Message::SetAppearance(appearance) => self.set_appearance(appearance),
            Message::SetInterval(interval) => {
                self.settings.interval = interval;
                return self.save_schedule();
            }
            Message::SetAfterUpdates(on) => {
                self.settings.after_updates = on;
                return self.save_schedule();
            }
            Message::ScheduleApplied(result) => {
                if let Err(error) = result {
                    self.notice = Some(format_1(t(self.language, Msg::NoticeScheduleFailed), &error));
                }
            }
            Message::CheckDriftNow => {
                return self.blocking(
                    |engine| {
                        let build = sysinfo::query().map(|info| info.full_build()).unwrap_or_default();
                        (engine.detect_drift().unwrap_or_default(), build)
                    },
                    |(drift, build)| Message::DriftChecked(drift, build),
                );
            }
            Message::DriftChecked(drift, build) => {
                if drift.is_empty() {
                    self.notice = Some(t(self.language, Msg::NoticeNoDrift).to_owned());
                }
                self.drift = drift;
                self.settings.last_check = windowcraft_engine::snapshot::unix_now();
                if !build.is_empty() {
                    self.settings.last_build = build;
                }
                self.save_settings();
            }
            Message::SystemThemeChanged => {
                self.accessibility = platform::accessibility();
                apply_motion_settings(self.accessibility);
                self.timeline.mode =
                    if self.accessibility.animations { MotionMode::Spring } else { MotionMode::Reduced };
                if let Some(handle) = self.window {
                    platform::apply_backdrop(handle, self.palette().dark, self.translucent());
                }
            }
            Message::Relaunch => match sysinfo::relaunch_elevated("") {
                Ok(()) => return iced::exit(),
                Err(error) => self.notice = Some(error.to_string()),
            },
            Message::DismissNotice => {
                self.notice = None;
                self.needs_elevation = false;
            }
            Message::GripEnter => {
                self.grip_hover = true;
                self.pointer = None;
            }
            Message::GripExit => self.grip_hover = false,
            Message::GripPressed => match self.pointer {
                Some(position) => self.begin_drag(position),
                None => self.drag_armed = true,
            },
            Message::Pointer(position) => {
                self.pointer = Some(position);
                if self.drag_armed {
                    self.drag_armed = false;
                    self.begin_drag(position);
                }
                let time = self.seconds();
                if let Some(offset) = self.drag.pointer_move(f64::from(position.x), f64::from(position.y), time) {
                    let width = f64::from(INSPECTOR_WIDTH);
                    let open = if offset < 0.0 { width + rubberband(-offset, width, 0.55) } else { width - offset };
                    self.timeline.channels[DRAWER].track(open.max(0.0));
                }
            }
            Message::PointerUp => {
                self.drag_armed = false;
                let time = self.seconds();
                if let Some(release) = self.drag.pointer_up(time) {
                    self.settle_drawer(release.position, release.velocity);
                }
            }
            Message::PointerLost => {
                self.drag_armed = false;
                if let Some(release) = self.drag.cancel() {
                    self.settle_drawer(release.position, 0.0);
                }
            }
            Message::AppToggled(id) => {
                toggle(&mut self.selected_apps, id);
                self.update_dock();
            }
            Message::Category(name) => {
                if self.modifiers.control() {
                    toggle(&mut self.categories, name);
                } else if self.categories.len() == 1 && self.categories.contains(&name) {
                    self.categories.clear();
                } else {
                    self.categories = BTreeSet::from([name]);
                }
            }
            Message::Manager(manager) => {
                self.manager = manager;
                self.animate(TAB, manager_index(manager));
            }
            Message::ClearApps => {
                self.selected_apps.clear();
                self.update_dock();
            }
            Message::ShowInstalled => {
                // The registry scan is instant and WinGet takes seconds, so
                // installed versions appear first and latest versions follow.
                self.checking_upgrades = true;
                return Task::batch([
                    self.blocking(|_| packages::installed_programs(), Message::InstalledLoaded),
                    self.blocking(
                        |_| packages::available_upgrades(&CancelToken::new()).unwrap_or_default(),
                        Message::UpgradesLoaded,
                    ),
                ]);
            }
            Message::InstalledLoaded(programs) => {
                self.installed = self
                    .catalog
                    .apps
                    .iter()
                    .filter_map(|app| {
                        packages::match_installed(app, &programs).map(|found| (app.id.clone(), found.version.clone()))
                    })
                    .collect();
            }
            Message::UpgradesLoaded(upgrades) => {
                self.upgrades = upgrades.into_iter().collect();
                self.checking_upgrades = false;
            }
            Message::InstallApps => return self.package_job(t(self.language, Msg::JobInstallingApps), true),
            Message::UninstallApps => return self.package_job(t(self.language, Msg::JobUninstallingApps), false),
            Message::UpgradeAll => {
                let manager = self.manager;
                let lang = self.language;
                return self.start_job(t(lang, Msg::JobUpgradingAllPackages), move |context| {
                    packages::upgrade_all(manager, &context.cancel, &mut |line| context.log(line))?;
                    Ok(t(lang, Msg::NoticeAllPackagesUpgraded).to_owned())
                });
            }
            Message::Tab(tab) => {
                self.tab = tab;
                self.animate(TAB, tab.index() as f64);
                self.update_dock();
            }
            Message::TweakChecked(id) => {
                toggle(&mut self.selected_tweaks, id);
                self.update_dock();
            }
            Message::Preset(preset) => {
                self.selected_tweaks = self.catalog.preset(preset).iter().cloned().collect();
                self.update_dock();
            }
            Message::ClearTweaks => {
                self.selected_tweaks.clear();
                self.update_dock();
            }
            Message::DetectApplied => {
                self.select_applied = true;
                return self.refresh_detection();
            }
            Message::Detected(states, options) => {
                self.detections = states;
                self.options = options;
                if std::mem::take(&mut self.select_applied) {
                    let applied = self.detections.iter().filter(|(_, state)| **state == Detection::Applied);
                    let catalog = &self.catalog;
                    self.selected_tweaks = applied
                        .filter(|(id, _)| catalog.tweak(id).is_some_and(|tweak| tweak.kind == TweakKind::Check))
                        .map(|(id, _)| id.clone())
                        .collect();
                    self.update_dock();
                }
            }
            Message::ApplyTweaks => {
                let needs_warning = self
                    .selected_tweaks
                    .iter()
                    .find(|id| self.catalog.tweak(id).is_some_and(|tweak| tweak.irreversible));
                if let Some(id) = needs_warning {
                    self.confirm = Some(Confirm::IrreversibleTweak(id.clone()));
                    return Task::none();
                }
                return self.apply_selected_tweaks();
            }
            Message::Toggle(id, enabled) => {
                if let Some(tweak) = self.find_tweak(&id) {
                    return self.run_items(&tweak.name.clone(), vec![(tweak, TweakAction::Set(enabled))]);
                }
            }
            Message::Segment(id, option) => {
                if let Some(tweak) = self.find_tweak(&id) {
                    return self.run_items(&tweak.name.clone(), vec![(tweak, TweakAction::Select(option))]);
                }
            }
            Message::Dns(selection) => {
                self.dns_choice = selection.clone();
                let catalog = self.catalog.clone();
                let lang = self.language;
                return self.start_job(t(lang, Msg::JobConfiguringDns), move |_| actions::apply_dns(&catalog, &selection, lang));
            }
            Message::RunAction(id) => {
                if let Some(tweak) = self.find_tweak(&id) {
                    return self.run_items(&tweak.name.clone(), vec![(tweak, TweakAction::Apply)]);
                }
            }
            Message::Undo(id) => {
                if let Some(tweak) = self.find_tweak(&id) {
                    let engine = self.engine.clone();
                    let lang = self.language;
                    return self.start_job(t(lang, Msg::JobUndoingChange), move |_| {
                        engine.undo_tweak(&tweak)?;
                        Ok(format_1(t(lang, Msg::NoticeRevertedTweak), &tweak.name))
                    });
                }
            }
            Message::AppxChecked(id) => {
                toggle(&mut self.selected_appx, id);
                self.update_dock();
            }
            Message::AppxDefaults => {
                self.selected_appx = self.catalog.presets.appx_default.iter().cloned().collect();
                self.update_dock();
            }
            Message::RemoveAppx => {
                let patterns: Vec<String> = self
                    .selected_appx
                    .iter()
                    .filter_map(|id| self.catalog.appx_package(id))
                    .map(|package| package.package_id.clone())
                    .collect();
                let engine = self.engine.clone();
                let lang = self.language;
                return self.start_queue_job(t(lang, Msg::JobRemovingAppx), Some(Queue::Appx), move |context| {
                    let total = patterns.len().max(1) as f32;
                    for (index, pattern) in patterns.iter().enumerate() {
                        context.progress(index as f32 / total, pattern);
                        engine.native.run(&NativeOp::RemoveAppx { pattern: pattern.clone() }, &context.cancel)?;
                    }
                    Ok(format_1(t(lang, Msg::NoticeRemovedPackages), &patterns.len().to_string()))
                });
            }
            Message::InstallAppx => {
                let ids: Vec<(String, Option<String>)> = self
                    .selected_appx
                    .iter()
                    .filter_map(|id| self.catalog.appx_package(id))
                    .map(|package| (package.name.clone(), package.store_id.clone()))
                    .collect();
                let engine = self.engine.clone();
                let lang = self.language;
                return self.start_queue_job(t(lang, Msg::JobInstallingAppx), Some(Queue::Appx), move |context| {
                    let total = ids.len().max(1) as f32;
                    for (index, (name, store)) in ids.iter().enumerate() {
                        context.progress(index as f32 / total, name);
                        if let Some(store) = store {
                            engine.native.run(&NativeOp::WingetInstall { id: store.clone() }, &context.cancel)?;
                        }
                    }
                    Ok(format_1(t(lang, Msg::NoticeInstalledPackages), &ids.len().to_string()))
                });
            }
            Message::ImportProfile => {
                return Task::perform(
                    platform::pick_open_file(
                        t(self.language, Msg::DialogImportConfigTitle),
                        t(self.language, Msg::DialogJsonProfilesLabel),
                        "*.json",
                    ),
                    Message::ProfilePicked,
                );
            }
            Message::ProfilePicked(path) => {
                if let Some(path) = path {
                    match std::fs::read_to_string(&path)
                        .map_err(|error| error.to_string())
                        .and_then(|text| windowcraft_config::import_any(&text, &self.catalog).map_err(|e| e.to_string()))
                    {
                        Ok(resolved) => {
                            self.apply_loaded_profile(resolved.profile);
                            self.notice = Some(format_1(t(self.language, Msg::NoticeImported), &path.display().to_string()));
                        }
                        Err(error) => self.notice = Some(format_1(t(self.language, Msg::NoticeImportFailed), &error)),
                    }
                }
            }
            Message::ExportProfile => {
                let path = windowcraft_engine::win::local_app_dir().join("profile.json");
                let text = self.selected_profile().to_json();
                return match std::fs::create_dir_all(windowcraft_engine::win::local_app_dir())
                    .and_then(|()| std::fs::write(&path, text))
                {
                    Ok(()) => {
                        let command = format!("windowcraft.exe --config \"{}\" --run", path.display());
                        self.notice = Some(format_1(t(self.language, Msg::NoticeProfileSavedCopied), &command));
                        iced::clipboard::write(command)
                    }
                    Err(error) => {
                        self.notice = Some(format_1(t(self.language, Msg::NoticeExportFailed), &error.to_string()));
                        Task::none()
                    }
                };
            }
            Message::ExportReport => {
                let engine = self.engine.clone();
                let lang = self.language;
                return self.start_job(t(lang, Msg::JobExportingEnvironmentReport), move |_| {
                    let report = windowcraft_engine::report::build(&engine)?;
                    let path = windowcraft_engine::win::local_app_dir().join("environment-report.json");
                    let text = windowcraft_engine::report::to_json(&report)?;
                    std::fs::write(&path, text).map_err(|e| EngineError::io(path.display().to_string(), e))?;
                    Ok(format_1(t(lang, Msg::NoticeReportWrittenTo), &path.display().to_string()))
                });
            }
            Message::FeatureChecked(id) => {
                toggle(&mut self.selected_features, id);
                self.update_dock();
            }
            Message::InstallFeatures => {
                let features: Vec<Feature> =
                    self.selected_features.iter().filter_map(|id| self.catalog.feature(id)).cloned().collect();
                return self.feature_job(t(self.language, Msg::JobEnablingFeatures), Some(Queue::Features), features);
            }
            Message::RunFix(id) => {
                if let Some(feature) = self.catalog.feature(&id).cloned() {
                    return self.feature_job(&feature.name.clone(), None, vec![feature]);
                }
            }
            Message::FixMode(mode) => {
                self.fix_modes.insert("update".to_owned(), mode);
            }
            Message::Launch(target) => {
                if let Err(error) = windowcraft_engine::shell::launch(&target, None) {
                    self.notice = Some(error.to_string());
                }
            }
            Message::UpdateMode(id) => {
                if self.catalog.update_mode(&id).is_some_and(|mode| mode.confirm) {
                    self.confirm = Some(Confirm::DisableUpdates);
                    return Task::none();
                }
                return self.update_mode_job(id);
            }
            Message::UpdateModeLoaded(mode) => self.update_mode = mode,
            Message::IsoPath(path) => self.iso.path = path,
            Message::IsoBrowse => {
                return Task::perform(
                    platform::pick_open_file(
                        t(self.language, Msg::DialogSelectIsoTitle),
                        t(self.language, Msg::DialogIsoImagesLabel),
                        "*.iso",
                    ),
                    Message::IsoPicked,
                );
            }
            Message::IsoPicked(path) => {
                if let Some(path) = path {
                    self.iso.path = path.display().to_string();
                }
            }
            Message::IsoDownloadUrl(url) => self.iso.url = url,
            Message::IsoDownloadHash(hash) => self.iso.hash = hash,
            Message::IsoDownload => {
                let (url, hash) = (self.iso.url.clone(), self.iso.hash.trim().to_owned());
                let destination = work_dir().join("Win11.iso");
                self.iso.path = destination.display().to_string();
                let lang = self.language;
                return self.start_job(t(lang, Msg::JobDownloadingWin11), move |context| {
                    let hash = (!hash.is_empty()).then_some(hash);
                    pipeline::download_iso(&url, &destination, hash.as_deref(), context)
                        .map_err(|e| EngineError::Invalid(e.to_string()))?;
                    Ok(t(lang, Msg::NoticeDownloadVerified).to_owned())
                });
            }
            Message::IsoOpen => {
                let mut session = self.studio_session();
                let work = work_dir();
                // Picks up where an interrupted build stopped for the same ISO.
                if let Some(saved) = StudioSession::resume(&work).filter(|saved| saved.source_iso == session.source_iso) {
                    session.step = saved.step;
                }
                let lang = self.language;
                return self.start_job(t(lang, Msg::JobOpeningIso), move |context| {
                    session.save(&work).map_err(|e| EngineError::Invalid(e.to_string()))?;
                    let images = pipeline::extract(&mut session, &work, context)
                        .map_err(|e| EngineError::Invalid(e.to_string()))?;
                    Ok(format_1(t(lang, Msg::NoticeFoundEditions), &images.len().to_string()))
                });
            }
            Message::IsoImages(result) => match result {
                Ok(images) => {
                    self.iso.selected = pipeline::default_edition(&images).map(|image| image.index);
                    if let Some(image) = pipeline::default_edition(&images) {
                        pipeline::select_edition(&mut self.iso.options, image);
                    }
                    self.iso.images = images;
                }
                Err(error) => self.notice = Some(error),
            },
            Message::IsoEdition(index) => {
                self.iso.selected = Some(index);
                if let Some(image) = self.iso.images.iter().find(|image| image.index == index) {
                    pipeline::select_edition(&mut self.iso.options, image);
                }
            }
            Message::IsoFlag(flag, value) => match flag {
                IsoFlag::Bypass => self.iso.options.bypass_requirements = value,
                IsoFlag::LocalAccount => self.iso.options.local_account = value,
                IsoFlag::DarkMode => self.iso.options.dark_mode = value,
                IsoFlag::NoEncryption => self.iso.options.disable_device_encryption = value,
                IsoFlag::PauseUpdates => self.iso.options.pause_updates_during_oobe = value,
                IsoFlag::InjectDrivers => self.iso.inject_drivers = value,
                IsoFlag::BundleApps => self.iso.bundle_apps = value,
            },
            Message::IsoOutput(path) => self.iso.output = path,
            Message::IsoBrowseOutput => {
                return Task::perform(
                    platform::pick_save_file(t(self.language, Msg::DialogSaveIsoTitle), "Windows11-WindowCraft.iso"),
                    Message::IsoOutputPicked,
                );
            }
            Message::IsoOutputPicked(path) => {
                if let Some(path) = path {
                    self.iso.output = path.display().to_string();
                }
            }
            Message::IsoBuild => return self.iso_job(None),
            Message::IsoRefreshDisks => return self.blocking(|_| usb::eligible_disks(), Message::IsoDisks),
            Message::IsoDisks(disks) => self.iso.disks = disks,
            Message::IsoWriteUsb(number) => {
                if let Some(disk) = self.iso.disks.iter().find(|disk| disk.number == number) {
                    self.confirm = Some(Confirm::EraseDisk { number, model: disk.model.clone() });
                }
            }
            Message::StripPick => {
                return Task::perform(
                    platform::pick_open_files(
                        t(self.language, Msg::DialogSelectMediaTitle),
                        t(self.language, Msg::DialogMediaLabel),
                        metadata::SUPPORTED_PATTERN,
                    ),
                    Message::StripPicked,
                );
            }
            Message::StripPicked(files) => {
                if files.is_empty() {
                    return Task::none();
                }
                let lang = self.language;
                return self.start_private_job(t(lang, Msg::JobStrippingMetadata), move |context| {
                    strip_files(lang, &files, context)
                });
            }
            Message::VideoUrl(url) => self.misc.url = url,
            Message::VideoFormat(format) => {
                self.misc.format = format;
                self.animate(TAB, format_index(format));
            }
            Message::VideoFolder(folder) => self.misc.folder = folder,
            Message::VideoBrowseFolder => {
                return Task::perform(
                    platform::pick_folder(t(self.language, Msg::DialogSelectFolderTitle)),
                    Message::VideoFolderPicked,
                );
            }
            Message::VideoFolderPicked(folder) => {
                if let Some(folder) = folder {
                    self.misc.folder = folder.display().to_string();
                }
            }
            Message::VideoDownload => {
                let url = self.misc.url.trim().to_owned();
                if !youtube::is_youtube_url(&url) {
                    self.notice = Some(t(self.language, Msg::NoticeInvalidYoutubeUrl).to_owned());
                    return Task::none();
                }
                let folder = if self.misc.folder.trim().is_empty() {
                    youtube::downloads_dir()
                } else {
                    PathBuf::from(self.misc.folder.trim())
                };
                let (lang, format) = (self.language, self.misc.format);
                return self.start_private_job(t(lang, Msg::JobDownloadingVideo), move |context| {
                    let report = |stage: Stage| {
                        let (fraction, step) = stage_step(lang, stage);
                        context.progress(fraction, step);
                    };
                    let tools = tools::ready(Extension::YouTube, &context.cancel, |stage| report(Stage::Tools(stage)))
                        .map_err(|error| readable(lang, error))?;
                    let saved = youtube::download_video(&tools, &url, format, &folder, &context.cancel, report)?;
                    Ok(format_1(t(lang, Msg::NoticeVideoSaved), &file_name(&saved)))
                });
            }
            Message::InstallExtension(extension) => {
                let lang = self.language;
                let name = extension_name(lang, extension);
                return self.start_private_job(&format_1(t(lang, Msg::JobInstallingExtension), name), move |context| {
                    tools::install(extension, &context.cancel, |stage| {
                        let (fraction, step) = tool_step(lang, stage);
                        context.progress(fraction, step);
                    })
                    .map_err(|error| readable(lang, error))?;
                    Ok(format_1(t(lang, Msg::NoticeExtensionInstalled), name))
                });
            }
            Message::UninstallExtension(extension) => {
                let lang = self.language;
                let name = extension_name(lang, extension);
                return self.start_private_job(&format_1(t(lang, Msg::JobRemovingExtension), name), move |_| {
                    tools::uninstall(extension)?;
                    Ok(format_1(t(lang, Msg::NoticeExtensionRemoved), name))
                });
            }
            Message::ConvertTarget(target) => self.misc.target = target,
            Message::ConvertPick => {
                return Task::perform(
                    platform::pick_open_files(
                        t(self.language, Msg::DialogConvertTitle),
                        t(self.language, Msg::DialogConvertLabel),
                        convert::INPUT_PATTERN,
                    ),
                    Message::ConvertPicked,
                );
            }
            Message::ConvertPicked(files) => {
                if files.is_empty() {
                    return Task::none();
                }
                let (lang, target) = (self.language, self.misc.target);
                return self.start_private_job(&format_1(t(lang, Msg::JobConverting), target.label()), move |context| {
                    convert_files(lang, &files, target, context)
                });
            }
            Message::Refreshed(sessions, drift) => {
                self.sessions = sessions;
                self.drift = drift;
            }
            Message::SelectSession(id) => {
                if self.open_session == Some(id) {
                    self.open_session = None;
                    self.animate(ACCORDION, 0.0);
                    return Task::none();
                }
                self.open_session = Some(id);
                self.timeline.channels[ACCORDION].snap(0.0);
                let lang = self.language;
                return self.blocking(
                    move |engine| {
                        let records = engine.store.changes(id).unwrap_or_default();
                        let views = records.into_iter().map(|record| live_view(&engine, record, lang)).collect();
                        (id, views)
                    },
                    |(id, views)| Message::Changes(id, views),
                );
            }
            Message::Changes(id, views) => {
                if self.open_session == Some(id) {
                    let height = views.len() as f64 * 52.0;
                    self.changes = views;
                    self.animate(ACCORDION, height);
                }
            }
            Message::RollbackChange(session, index) => {
                let engine = self.engine.clone();
                let lang = self.language;
                return self.start_job(t(lang, Msg::JobRollingBackOneChange), move |_| {
                    engine.rollback_change(session, index)?;
                    Ok(t(lang, Msg::NoticeChangeRolledBack).to_owned())
                });
            }
            Message::RepairDrift => {
                let engine = self.engine.clone();
                let lang = self.language;
                return self.start_job(t(lang, Msg::JobReApplyingDrift), move |_| {
                    Ok(format_1(t(lang, Msg::NoticeReAppliedTweaks), &engine.repair_drift()?.to_string()))
                });
            }
            Message::InspectorTab(tab) => {
                self.inspector_tab = tab;
                self.animate(TAB, tab.index() as f64);
                return self.load_inspector_tab();
            }
            Message::Devices(devices, timer) => {
                self.devices = devices;
                self.timer = timer;
            }
            Message::SetMsi(index, enabled) => {
                if let Some(device) = self.devices.get_mut(index) {
                    match gaming::set_msi(&self.engine, device, enabled) {
                        Ok(_) => device.msi_enabled = enabled,
                        Err(error) => self.notice = Some(error.to_string()),
                    }
                }
            }
            Message::SetPriority(index, priority) => {
                if let Some(device) = self.devices.get_mut(index) {
                    match gaming::set_priority(&self.engine, device, priority) {
                        Ok(_) => device.priority = priority,
                        Err(error) => self.notice = Some(error.to_string()),
                    }
                }
            }
            Message::Autoruns(entries) => self.autoruns = entries,
            Message::SetAutorun(index, enabled) => {
                if let Some(entry) = self.autoruns.get_mut(index) {
                    match autoruns::set_enabled_tracked(&self.engine, entry, enabled) {
                        Ok(_) => entry.enabled = enabled,
                        Err(error) => self.notice = Some(error.to_string()),
                    }
                }
            }
            Message::CleanScan => {
                return self.blocking(
                    |_| {
                        let results: Vec<(CleanTarget, ScanResult)> = cleaner::targets()
                            .into_iter()
                            .map(|target| {
                                let scan = cleaner::scan(&target);
                                (target, scan)
                            })
                            .collect();
                        let names: Vec<String> =
                            packages::installed_programs().into_iter().map(|program| program.name).collect();
                        (results, cleaner::orphan_candidates(&names))
                    },
                    |(results, orphans)| Message::CleanResults(results, orphans),
                );
            }
            Message::CleanResults(results, orphans) => {
                self.clean_results = results;
                self.orphans = orphans;
                self.selected_orphans.clear();
            }
            Message::CleanToggled(id) => toggle(&mut self.clean_selected, id),
            Message::OrphanToggled(path) => toggle(&mut self.selected_orphans, path),
            Message::CleanRun => {
                let targets: Vec<CleanTarget> = self
                    .clean_results
                    .iter()
                    .filter(|(target, _)| self.clean_selected.contains(target.id))
                    .map(|(target, _)| target.clone())
                    .collect();
                let orphans: Vec<PathBuf> = self.selected_orphans.iter().cloned().collect();
                let iso_open = !self.iso.images.is_empty();
                let lang = self.language;
                return self.start_job(t(lang, Msg::JobCleaningCaches), move |context| {
                    let total = (targets.len() + orphans.len()).max(1) as f32;
                    let mut freed = 0;
                    for (index, target) in targets.iter().enumerate() {
                        context.cancel.check()?;
                        if target.id == cleaner::ISO_TARGET && iso_open {
                            context.progress(
                                index as f32 / total,
                                t(lang, Msg::NoticeIsoWorkFilesSkipped),
                            );
                            continue;
                        }
                        context.progress(index as f32 / total, target.name);
                        freed += cleaner::clean(target).removed_bytes;
                    }
                    for (index, path) in orphans.iter().enumerate() {
                        context.cancel.check()?;
                        context.progress((targets.len() + index) as f32 / total, path.display().to_string());
                        freed += cleaner::remove_orphan(path).removed_bytes;
                    }
                    Ok(format_1(t(lang, Msg::NoticeFreedMb), &(freed >> 20).to_string()))
                });
            }
            Message::BenchRun => {
                self.bench_running = true;
                let targets: Vec<Target> = self.catalog.dns.iter().filter_map(Target::from_provider).collect();
                return Task::perform(
                    async move { dns_bench::benchmark(targets, 6).await.map_err(|e| e.to_string()) },
                    Message::BenchResults,
                );
            }
            Message::BenchResults(result) => {
                self.bench_running = false;
                match result {
                    Ok(results) => self.show_bench(results),
                    Err(error) => self.notice = Some(error),
                }
            }
            Message::Inspect(inspected) => {
                if self.inspected.as_ref() == Some(&inspected) {
                    self.close_inspector();
                    return Task::none();
                }
                return self.open_inspector(inspected);
            }
            Message::InspectRows(rows) => self.inspect_rows = rows,
            Message::CloseInspector => self.close_inspector(),
            Message::Job(event) => return self.on_job_event(event),
            Message::CancelJob => {
                if let Some(job) = &self.job {
                    job.cancel.cancel();
                }
            }
            Message::Ask(confirm) => self.confirm = Some(confirm),
            Message::Answer(accepted) => {
                let confirm = self.confirm.take();
                if accepted && let Some(confirm) = confirm {
                    return self.on_confirmed(confirm);
                }
            }
            Message::Notice(text) => self.notice = Some(text),
            Message::SetLanguage(lang) => {
                self.language = lang;
                if let Err(error) = i18n::save_settings_language(lang) {
                    self.notice = Some(format_1(t(self.language, Msg::NoticeFailedToSaveSettings), &error));
                }
            }
            Message::CloseRequested => {
                // A second close while the job stops means it should not wait.
                if self.job.is_none() || self.quit_pending {
                    return iced::exit();
                }
                self.confirm = Some(Confirm::QuitDuringJob);
            }
        }
        Task::none()
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.appearance = appearance;
        self.settings.appearance = appearance;
        if let Some(handle) = self.window {
            platform::apply_backdrop(handle, self.palette().dark, self.translucent());
        }
        self.save_settings();
    }

    fn save_settings(&mut self) {
        if let Err(error) = settings::save(&self.settings) {
            self.notice = Some(format_1(t(self.language, Msg::NoticeFailedToSaveSettings), &error));
        }
    }

    /// Saves the check settings, then creates or removes the scheduled task.
    fn save_schedule(&mut self) -> Task<Message> {
        self.save_settings();
        let chosen = self.settings.clone();
        let description = t(self.language, Msg::SettingsTaskDescription).to_owned();
        self.blocking(move |_| settings::apply_schedule(&chosen, &description), Message::ScheduleApplied)
    }

    fn seconds(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    fn apply_selected_tweaks(&mut self) -> Task<Message> {
        let items: Vec<(Tweak, TweakAction)> = self
            .selected_tweaks
            .iter()
            .filter_map(|id| self.catalog.tweak(id))
            .map(|tweak| (tweak.clone(), TweakAction::Apply))
            .collect();
        self.run_items_for(t(self.language, Msg::JobApplyingTweaks), Some(Queue::Tweaks), items)
    }

    fn begin_drag(&mut self, position: Point) {
        let origin = f64::from(INSPECTOR_WIDTH) - self.timeline.channels[DRAWER].value();
        self.drag.pointer_down(f64::from(position.x), f64::from(position.y), origin, self.seconds());
    }

    fn settle_drawer(&mut self, offset: f64, velocity: f64) {
        let width = f64::from(INSPECTOR_WIDTH);
        // Commit follows the sign of the flick. A pointer that stopped moving
        // has no opinion, so position decides.
        let close = if velocity.abs() > FLICK_THRESHOLD { velocity > 0.0 } else { offset > width / 2.0 };
        let target = if close { 0.0 } else { width };
        let position = self.timeline.channels[DRAWER].value();
        let mode = self.timeline.mode;
        self.timeline.channels[DRAWER].fling_to(position, -velocity, target, mode);
        if close {
            self.inspected = None;
        }
    }

    fn show_bench(&mut self, results: Vec<BenchResult>) {
        let slowest = results.iter().filter_map(BenchResult::score).fold(1.0, f64::max);
        self.timeline.channels.truncate(DNS_BARS);
        let mode = self.timeline.mode;
        for result in &results {
            let mut channel = windowcraft_physics::Channel::with_precision(0.0, VISIBLE_PIXEL / DNS_BAR_PIXELS);
            channel.animate_to(result.score().map_or(0.0, |score| score / slowest), mode);
            self.timeline.channels.push(channel);
        }
        self.bench = results;
    }

    fn on_keyboard(&mut self, event: keyboard::Event) -> Task<Message> {
        match event {
            keyboard::Event::ModifiersChanged(modifiers) => self.modifiers = modifiers,
            keyboard::Event::KeyPressed { key, modifiers, .. } => match key.as_ref() {
                Key::Character("k" | "f") if modifiers.control() => {
                    return Task::batch([
                        widget::operation::focus(search_id()),
                        widget::operation::select_all(search_id()),
                    ]);
                }
                Key::Named(key::Named::Escape) => {
                    if self.confirm.is_some() {
                        self.confirm = None;
                    } else if !self.query.is_empty() {
                        self.query.clear();
                        self.hits.clear();
                    } else {
                        self.close_inspector();
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Task::none()
    }

    fn package_job(&mut self, label: &str, install: bool) -> Task<Message> {
        let manager = self.manager;
        let apps: Vec<windowcraft_config::App> =
            self.selected_apps.iter().filter_map(|id| self.catalog.app(id)).cloned().collect();
        let lang = self.language;
        self.start_queue_job(label, Some(Queue::Apps), move |context| {
            let total = apps.len().max(1) as f32;
            let mut failures = Vec::new();
            for (index, app) in apps.iter().enumerate() {
                context.cancel.check()?;
                context.progress(index as f32 / total, &app.name);
                let outcome = if install {
                    packages::install(manager, app, &context.cancel, &mut |line| context.log(line))
                } else {
                    packages::uninstall(manager, app, &context.cancel, &mut |line| context.log(line))
                };
                if let Err(error) = outcome {
                    failures.push(format!("{}: {error}", app.name));
                }
            }
            if failures.is_empty() {
                Ok(format_1(t(lang, Msg::NoticeApplicationsProcessed), &apps.len().to_string()))
            } else {
                Err(EngineError::Invalid(format_2(t(lang, Msg::NoticeFailures), &failures.len().to_string(), &failures[0])))
            }
        })
    }

    fn feature_job(&mut self, label: &str, queue: Option<Queue>, features: Vec<Feature>) -> Task<Message> {
        let engine = self.engine.clone();
        let option = self.fix_modes.get("update").cloned();
        let lang = self.language;
        self.start_queue_job(label, queue, move |context| {
            let total = features.len().max(1) as f32;
            for (index, feature) in features.iter().enumerate() {
                context.progress(index as f32 / total, &feature.name);
                // The scan reports per-stage results and live progress, which
                // the generic operation path has no way to carry.
                if feature.apply.contains(&NativeOp::FixSystemScan) {
                    let results = fixes::system_scan(&context.cancel, |fraction, line| context.progress(fraction, line))?;
                    fixes::require_clean(&results)?;
                    return Ok(fixes::scan_summary(&results));
                }
                engine.apply_feature(feature, option.as_deref(), &context.cancel)?;
            }
            Ok(format_1(t(lang, Msg::NoticeItemsCompleted), &features.len().to_string()))
        })
    }

    fn update_mode_job(&mut self, id: String) -> Task<Message> {
        let engine = self.engine.clone();
        let catalog = self.catalog.clone();
        let lang = self.language;
        self.start_job(t(lang, Msg::JobChangingUpdatePolicy), move |_| {
            let mode = catalog.update_mode(&id).ok_or_else(|| EngineError::NotFound(id.clone()))?;
            engine.apply_update_mode(mode)?;
            Ok(format_1(t(lang, Msg::NoticeWindowsUpdatesSetTo), &mode.name))
        })
    }

    fn iso_job(&mut self, disk: Option<(u32, String)>) -> Task<Message> {
        let mut session = self.studio_session();
        session.step = windowcraft_iso::Step::Extracted;
        let (work, output) = (work_dir(), PathBuf::from(&self.iso.output));
        let catalog = self.catalog.clone();
        let lang = self.language;
        self.start_job(t(lang, Msg::JobBuildingMedia), move |context| {
            let wrap = |error: windowcraft_iso::IsoError| EngineError::Invalid(error.to_string());
            pipeline::configure(&mut session, &work, &catalog, context).map_err(wrap)?;
            match disk {
                Some((number, model)) => {
                    pipeline::export_usb(&mut session, &work, &catalog, number, &model, context).map_err(wrap)?;
                    Ok(t(lang, Msg::NoticeUsbDriveReady).to_owned())
                }
                None => {
                    pipeline::export_iso(&mut session, &work, &output, context).map_err(wrap)?;
                    Ok(format_1(t(lang, Msg::NoticeIsoWrittenTo), &output.display().to_string()))
                }
            }
        })
    }

    fn on_confirmed(&mut self, confirm: Confirm) -> Task<Message> {
        match confirm {
            Confirm::DisableUpdates => self.update_mode_job("disabled".to_owned()),
            Confirm::RollbackSession(id) => {
                let engine = self.engine.clone();
                let lang = self.language;
                self.start_job(t(lang, Msg::JobRollingBackSession), move |_| {
                    let kept = engine.irreversible_in_session(id)?;
                    engine.rollback_session(id)?;
                    Ok(if kept.is_empty() {
                        t(lang, Msg::NoticeSessionRolledBack).to_owned()
                    } else {
                        format_1(t(lang, Msg::NoticeSessionRolledBackKept), &kept.join(", "))
                    })
                })
            }
            Confirm::EraseDisk { number, model } => self.iso_job(Some((number, model))),
            Confirm::IrreversibleTweak(_) => self.apply_selected_tweaks(),
            Confirm::QuitDuringJob => {
                // Cancelling lets the job stop its child tools. The app exits
                // when the job reports back.
                let Some(job) = &self.job else { return iced::exit() };
                job.cancel.cancel();
                self.quit_pending = true;
                self.notice = Some(format_1(t(self.language, Msg::NoticeStoppingBeforeClosing), &job.label));
                Task::none()
            }
        }
    }

    fn on_job_event(&mut self, event: JobEvent) -> Task<Message> {
        match event {
            JobEvent::Progress { fraction, step } => {
                let mut changed = false;
                if let Some(job) = &mut self.job {
                    // The taskbar call needs its own thread, so it only runs
                    // when the visible percentage moves.
                    changed = (job.fraction * 100.0) as u32 != (fraction * 100.0) as u32;
                    job.fraction = fraction;
                    job.step = step;
                }
                if changed && let Some(window) = self.window {
                    platform::taskbar_progress(window, Some(fraction));
                }
                Task::none()
            }
            JobEvent::Finished { outcome } => {
                let job = self.job.take();
                let label = job.as_ref().map(|job| job.label.clone()).unwrap_or_default();
                let queue = job.as_ref().and_then(|job| job.queue);
                let private = job.as_ref().is_some_and(|job| job.private);
                let mut lines = job.map_or_else(Vec::new, |job| job.log.snapshot());
                lines.push(match &outcome {
                    Ok(summary) => summary.clone(),
                    Err(error) => format!("Failed: {error}"),
                });
                if private {
                    self.misc.installed = tools::installed();
                self.misc.ffmpeg = tools::installed_ffmpeg().is_some();
                } else {
                    actions::append_log(&windowcraft_engine::win::local_app_dir().join("windowcraft.log"), &label, &lines);
                }
                self.last_log_label.clone_from(&label);
                self.last_log = lines;
                if self.quit_pending {
                    return iced::exit();
                }

                match outcome {
                    Ok(summary) => {
                        match queue {
                            Some(Queue::Apps) => self.selected_apps.clear(),
                            Some(Queue::Tweaks) => self.selected_tweaks.clear(),
                            Some(Queue::Appx) => self.selected_appx.clear(),
                            Some(Queue::Features) => self.selected_features.clear(),
                            None => {}
                        }
                        self.notice = Some(summary);
                    }
                    Err(error) => {
                        self.needs_elevation = error.contains("administrator rights");
                        self.notice = Some(format!("{label}: {error}"));
                        self.notice_failed.clone_from(&self.notice);
                    }
                }
                self.update_dock();
                if let Some(window) = self.window {
                    platform::taskbar_progress(window, None);
                }
                let mut tasks = vec![self.refresh_all()];
                if label == t(self.language, Msg::JobOpeningIso) || label == "Opening ISO" {
                    let media = work_dir().join("media");
                    tasks.push(self.blocking(
                        move |_| pipeline::validate_media(&media).map_err(|error| error.to_string()),
                        Message::IsoImages,
                    ));
                }
                Task::batch(tasks)
            }
        }
    }
}

fn manager_index(manager: PackageManager) -> f64 {
    if manager == PackageManager::Winget { 0.0 } else { 1.0 }
}

/// Wheel easing follows the Windows animation switch and wheel setting.
fn apply_motion_settings(accessibility: Accessibility) {
    // These are process-wide switches. In unit tests every App would set them
    // from the machine's Windows settings while the scroll tests, which hold
    // their own lock, run in parallel; a CI runner with animations off then
    // broke them at random.
    if cfg!(test) {
        return;
    }
    crate::smooth::set_enabled(accessibility.animations);
    crate::smooth::set_lines_per_notch(accessibility.wheel_lines);
}

fn keyboard_event(event: Event, _status: event::Status, _window: window::Id) -> Option<Message> {
    match event {
        Event::Keyboard(event) => Some(Message::Keyboard(event)),
        _ => None,
    }
}

fn pointer_event(event: Event, _status: event::Status, _window: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::Pointer(position)),
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Message::PointerUp),
        Event::Mouse(mouse::Event::CursorLeft) | Event::Window(window::Event::Unfocused) => Some(Message::PointerLost),
        _ => None,
    }
}

fn toggle<T: Ord>(set: &mut BTreeSet<T>, value: T) {
    if !set.remove(&value) {
        set.insert(value);
    }
}

fn tab_of(group: windowcraft_config::TweakGroup) -> TweakTab {
    use windowcraft_config::TweakGroup;
    match group {
        TweakGroup::Essential => TweakTab::Essential,
        TweakGroup::Advanced => TweakTab::Advanced,
        TweakGroup::Preferences | TweakGroup::Performance => TweakTab::Preferences,
    }
}

fn live_view(engine: &Engine, record: ChangeRecord, lang: Language) -> ChangeView {
    use windowcraft_engine::snapshot::Change;
    let live = match &record.change {
        Change::Registry { path, name, .. } => RegPath::parse(path)
            .ok()
            .and_then(|parsed| engine.scope.read(&parsed, name).ok())
            .flatten()
            .map_or_else(|| t(lang, Msg::NotSet).to_owned(), |value| value.display()),
        Change::Service { name, .. } => engine
            .services
            .query(name)
            .ok()
            .flatten()
            .map_or_else(|| t(lang, Msg::Absent).to_owned(), |info| i18n::service_startup(lang, info.startup).to_owned()),
        Change::Task { .. } | Change::Native { .. } => String::new(),
    };
    ChangeView { record, live }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windowcraft_engine::SnapshotStore;
    use windowcraft_engine::executor::RecordingRunner;
    use windowcraft_engine::services::MemoryServices;
    use windowcraft_engine::testing::Sandbox;

    #[test]
    fn inspector_tabs_fit_a_1024_pixel_window_in_every_language() {
        // 1024 px minus the 220 px sidebar and the 44 px page padding.
        const ROOM: f32 = 1024.0 - 220.0 - 44.0;
        for &lang in Language::ALL {
            let labels: Vec<&str> = InspectorTab::ALL.iter().map(|tab| tab.title_in(lang)).collect();
            let width = crate::ui::tab_width(&labels, 100.0);
            assert!(width * labels.len() as f32 <= ROOM, "{lang:?}: {labels:?} need {width} px each");
        }
    }

    struct Rig {
        app: App,
        _sandbox: Sandbox,
        _dir: tempfile::TempDir,
    }

    fn rig(name: &str) -> Rig {
        let sandbox = Sandbox::new(name);
        let dir = tempfile::tempdir().unwrap();
        let catalog = Arc::new(Catalog::load().unwrap());
        let engine = Arc::new(Engine {
            scope: sandbox.scope.clone(),
            services: Arc::new(MemoryServices::default()),
            native: Arc::new(RecordingRunner::default()),
            store: Arc::new(SnapshotStore::open(&dir.path().join("ui.redb")).unwrap()),
            catalog: catalog.clone(),
            ram_kb: 1 << 24,
            os_build: "1.1".to_owned(),
        });
        let info = SystemInfo {
            product: "Windows 11".to_owned(),
            display_version: "25H2".to_owned(),
            build: "26300".to_owned(),
            ubr: 1,
            architecture: "x64",
            ram_kb: 1 << 24,
            logical_processors: 8,
            elevated: false,
        };
        let (mut app, _) = App::new(catalog, engine, info, StartView::default());
        app.timeline.mode = MotionMode::Spring;
        Rig { app, _sandbox: sandbox, _dir: dir }
    }

    /// Runs the animation clock until every spring rests.
    fn settle(app: &mut App) -> usize {
        let start = Instant::now();
        let mut frames = 0;
        while app.timeline.needs_frame() {
            frames += 1;
            assert!(frames < 600, "the animation loop never stopped");
            drop(app.update(Message::Frame(start + std::time::Duration::from_millis(frames as u64 * 16))));
        }
        frames
    }

    fn send(app: &mut App, message: Message) {
        drop(app.update(message));
    }

    /// The real view behind the same diff, layout, update and operate calls
    /// the runtime makes, without a window.
    mod headless {
        use super::super::*;
        use iced::advanced::widget::{Operation, Tree};
        use iced::advanced::{Layout, Shell, layout};
        use iced::mouse::Cursor;
        use iced::{Event, Point, Rectangle, Renderer, Size, Vector};

        pub const BOUNDS: Size = Size::new(1000.0, 520.0);

        pub fn renderer() -> Renderer {
            use iced::advanced::renderer::Headless as _;
            use std::task::{Context, Poll, Waker};
            let future = Renderer::new(iced::Font::DEFAULT, iced::Pixels(16.0), Some("tiny-skia"));
            let mut future = std::pin::pin!(future);
            match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                Poll::Ready(Some(renderer)) => renderer,
                _ => panic!("tiny-skia must be available without waiting"),
            }
        }

        /// Content height and offset of every scrollable, in tree order.
        struct Offsets(Vec<(f32, f32)>);

        impl Operation for Offsets {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }

            fn scrollable(
                &mut self,
                _id: Option<&iced::advanced::widget::Id>,
                _bounds: Rectangle,
                content_bounds: Rectangle,
                translation: Vector,
                _state: &mut dyn iced::advanced::widget::operation::Scrollable,
            ) {
                self.0.push((content_bounds.height, translation.y));
            }
        }

        pub fn tree(app: &App) -> Tree {
            Tree::new(crate::views::view(app).as_widget())
        }

        /// One runtime pass. Returns the offset of the tallest scrollable,
        /// which is the page body, and the messages the event produced.
        pub fn pass(app: &App, tree: &mut Tree, renderer: &Renderer, event: Option<(Event, Point)>) -> (f32, Vec<Message>) {
            let mut root = crate::views::view(app);
            tree.diff(root.as_widget());
            let node = root.as_widget_mut().layout(tree, renderer, &layout::Limits::new(Size::ZERO, BOUNDS));
            let mut messages = Vec::new();
            if let Some((event, at)) = event {
                let mut shell = Shell::new(&mut messages);
                root.as_widget_mut().update(
                    tree,
                    &event,
                    Layout::new(&node),
                    Cursor::Available(at),
                    renderer,
                    &mut iced::advanced::clipboard::Null,
                    &mut shell,
                    &Rectangle::with_size(BOUNDS),
                );
            }
            let mut offsets = Offsets(Vec::new());
            root.as_widget_mut().operate(tree, Layout::new(&node), renderer, &mut offsets);
            let page = offsets.0.iter().copied().max_by(|a, b| a.0.total_cmp(&b.0)).map_or(0.0, |(_, offset)| offset);
            (page, messages)
        }
    }

    #[test]
    fn a_notice_that_comes_and_goes_keeps_the_page_where_it_was() {
        use iced::mouse::{self, ScrollDelta};
        let mut rig = rig("noticescroll");
        send(&mut rig.app, Message::Navigate(Page::Tweaks));
        settle(&mut rig.app);
        let renderer = headless::renderer();
        let mut tree = headless::tree(&rig.app);
        assert_eq!(headless::pass(&rig.app, &mut tree, &renderer, None).0, 0.0);

        let wheel = iced::Event::Mouse(mouse::Event::WheelScrolled { delta: ScrollDelta::Pixels { x: 0.0, y: -300.0 } });
        headless::pass(&rig.app, &mut tree, &renderer, Some((wheel, iced::Point::new(700.0, 400.0))));
        let scrolled = headless::pass(&rig.app, &mut tree, &renderer, None).0;
        assert!(scrolled > 250.0, "the wheel moved the page to {scrolled}");

        rig.app.notice = Some("Done.".to_owned());
        assert_eq!(headless::pass(&rig.app, &mut tree, &renderer, None).0, scrolled, "a new notice");
        send(&mut rig.app, Message::DismissNotice);
        assert_eq!(headless::pass(&rig.app, &mut tree, &renderer, None).0, scrolled, "a dismissed notice");
    }

    #[test]
    fn idle_app_requests_no_frames() {
        let rig = rig("uiidle");
        assert!(!rig.app.timeline.needs_frame());
    }

    #[test]
    fn navigation_animates_then_stops_requesting_frames() {
        let mut rig = rig("uinav");
        send(&mut rig.app, Message::Navigate(Page::Updates));
        assert_eq!(rig.app.page, Page::Updates);
        assert!(rig.app.timeline.needs_frame());
        assert!(settle(&mut rig.app) > 3);
        assert_eq!(rig.app.timeline.channels[NAV].value(), Page::Updates.index() as f64);
        assert!(!rig.app.timeline.needs_frame());
    }

    #[test]
    fn interrupted_navigation_keeps_its_presentation_value() {
        let mut rig = rig("uiinterrupt");
        let start = Instant::now();
        send(&mut rig.app, Message::Navigate(Page::Inspector));
        send(&mut rig.app, Message::Frame(start));
        send(&mut rig.app, Message::Frame(start + std::time::Duration::from_millis(48)));
        let before = rig.app.timeline.channels[NAV].value();
        assert!(before > 0.0);
        send(&mut rig.app, Message::Navigate(Page::Applications));
        assert_eq!(rig.app.timeline.channels[NAV].value(), before, "must not jump to the logical target");
    }

    #[test]
    fn dock_follows_the_selection() {
        let mut rig = rig("uidock");
        assert!(!rig.app.dock_visible());
        send(&mut rig.app, Message::AppToggled("7zip".to_owned()));
        assert!(rig.app.dock_visible());
        settle(&mut rig.app);
        assert_eq!(rig.app.timeline.channels[DOCK].value(), 1.0);
        send(&mut rig.app, Message::ClearApps);
        settle(&mut rig.app);
        assert_eq!(rig.app.timeline.channels[DOCK].value(), 0.0);
    }

    #[test]
    fn inspector_opens_and_dismisses_along_the_same_path() {
        let mut rig = rig("uidrawer");
        let inspected = Inspected::Tweak("activity".to_owned());
        send(&mut rig.app, Message::Inspect(inspected.clone()));
        settle(&mut rig.app);
        assert_eq!(rig.app.timeline.channels[DRAWER].value(), f64::from(INSPECTOR_WIDTH));

        send(&mut rig.app, Message::Inspect(inspected));
        assert!(rig.app.inspected.is_none(), "inspecting the same row again closes it");
        settle(&mut rig.app);
        assert_eq!(rig.app.timeline.channels[DRAWER].value(), 0.0);
    }

    fn open_drawer_and_grab(app: &mut App) {
        send(app, Message::Inspect(Inspected::Tweak("activity".to_owned())));
        settle(app);
        send(app, Message::Pointer(Point::new(900.0, 300.0)));
        send(app, Message::GripPressed);
    }

    #[test]
    fn drawer_tracks_the_pointer_one_to_one_after_the_threshold() {
        let mut rig = rig("uidrag");
        open_drawer_and_grab(&mut rig.app);
        send(&mut rig.app, Message::Pointer(Point::new(905.0, 300.0)));
        assert_eq!(rig.app.timeline.channels[DRAWER].value(), f64::from(INSPECTOR_WIDTH), "inside the 10px threshold");
        send(&mut rig.app, Message::Pointer(Point::new(960.0, 300.0)));
        assert_eq!(rig.app.timeline.channels[DRAWER].value(), f64::from(INSPECTOR_WIDTH) - 60.0);
    }

    #[test]
    fn dragging_past_the_open_position_rubberbands() {
        let mut rig = rig("uirubber");
        open_drawer_and_grab(&mut rig.app);
        send(&mut rig.app, Message::Pointer(Point::new(600.0, 300.0)));
        let value = rig.app.timeline.channels[DRAWER].value();
        let width = f64::from(INSPECTOR_WIDTH);
        assert!(value > width && value < width + 200.0, "{value}");
    }

    #[test]
    fn window_blur_mid_drag_settles_the_drawer() {
        let mut rig = rig("uiblur");
        open_drawer_and_grab(&mut rig.app);
        send(&mut rig.app, Message::Pointer(Point::new(960.0, 300.0)));
        send(&mut rig.app, Message::PointerLost);
        assert!(!rig.app.drag.is_active(), "the gesture must end");
        settle(&mut rig.app);
        assert_eq!(rig.app.timeline.channels[DRAWER].value(), f64::from(INSPECTOR_WIDTH));
        send(&mut rig.app, Message::Pointer(Point::new(100.0, 300.0)));
        assert_eq!(rig.app.timeline.channels[DRAWER].value(), f64::from(INSPECTOR_WIDTH), "no stuck drag");
    }

    #[test]
    fn slow_release_past_halfway_closes_the_drawer() {
        let mut rig = rig("uirelease");
        open_drawer_and_grab(&mut rig.app);
        send(&mut rig.app, Message::Pointer(Point::new(1200.0, 300.0)));
        std::thread::sleep(std::time::Duration::from_millis(80));
        send(&mut rig.app, Message::PointerUp);
        assert!(rig.app.inspected.is_none());
        settle(&mut rig.app);
        assert_eq!(rig.app.timeline.channels[DRAWER].value(), 0.0);
    }

    #[test]
    fn ctrl_click_builds_a_multi_selection() {
        let mut rig = rig("uichips");
        send(&mut rig.app, Message::Category("Games".to_owned()));
        send(&mut rig.app, Message::Category("Document".to_owned()));
        assert_eq!(rig.app.categories.len(), 1, "a plain click replaces");
        rig.app.modifiers = Modifiers::CTRL;
        send(&mut rig.app, Message::Category("Games".to_owned()));
        assert_eq!(rig.app.categories.len(), 2);
    }

    #[test]
    fn search_selection_opens_the_owning_module_and_inspector() {
        let mut rig = rig("uisearch");
        send(&mut rig.app, Message::Query("AllowTelemetry".to_owned()));
        assert!(!rig.app.hits.is_empty());
        send(&mut rig.app, Message::SearchSelected(SearchKind::RegistryKey, "telemetry".to_owned()));
        assert_eq!(rig.app.page, Page::Tweaks);
        assert_eq!(rig.app.inspected, Some(Inspected::Tweak("telemetry".to_owned())));
        assert!(rig.app.query.is_empty());
    }

    #[test]
    fn presets_and_clear_drive_the_queue() {
        let mut rig = rig("uipreset");
        send(&mut rig.app, Message::Navigate(Page::Tweaks));
        send(&mut rig.app, Message::Preset(PresetName::Standard));
        assert_eq!(rig.app.selected_tweaks.len(), 13);
        send(&mut rig.app, Message::ClearTweaks);
        assert!(rig.app.selected_tweaks.is_empty());
    }

    #[test]
    fn dangerous_actions_wait_for_confirmation() {
        let mut rig = rig("uiconfirm");
        send(&mut rig.app, Message::UpdateMode("disabled".to_owned()));
        assert_eq!(rig.app.confirm, Some(Confirm::DisableUpdates));
        assert!(rig.app.job.is_none(), "nothing runs before the answer");
        send(&mut rig.app, Message::Answer(false));
        assert!(rig.app.confirm.is_none());
        assert!(rig.app.job.is_none());

        send(&mut rig.app, Message::TweakChecked("disable-bit-locker".to_owned()));
        send(&mut rig.app, Message::ApplyTweaks);
        assert!(matches!(rig.app.confirm, Some(Confirm::IrreversibleTweak(_))));
    }

    #[test]
    fn a_second_job_is_refused_while_one_runs() {
        let mut rig = rig("uijob");
        drop(rig.app.start_job("First", |context| {
            while !context.cancel.is_cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Ok(String::new())
        }));
        drop(rig.app.start_job("Second", |_| Ok(String::new())));
        assert_eq!(rig.app.job.as_ref().unwrap().label, "First");
        assert!(rig.app.notice.is_some());
        send(&mut rig.app, Message::CancelJob);
    }

    fn running_job() -> ActiveJob {
        ActiveJob {
            label: "Writing USB".to_owned(),
            fraction: 0.0,
            step: String::new(),
            cancel: CancelToken::new(),
            log: LogRing::new(8),
            queue: None,
            private: false,
        }
    }

    #[test]
    fn closing_without_a_job_asks_nothing() {
        let mut rig = rig("uicloseidle");
        send(&mut rig.app, Message::CloseRequested);
        assert!(rig.app.confirm.is_none());
    }

    #[test]
    fn closing_during_a_job_asks_and_cancel_keeps_it_running() {
        let mut rig = rig("uiclosekeep");
        rig.app.job = Some(running_job());
        send(&mut rig.app, Message::CloseRequested);
        assert_eq!(rig.app.confirm, Some(Confirm::QuitDuringJob));

        send(&mut rig.app, Message::Answer(false));
        assert!(rig.app.confirm.is_none());
        assert!(!rig.app.quit_pending);
        assert!(!rig.app.job.as_ref().unwrap().cancel.is_cancelled());
    }

    #[test]
    fn quitting_anyway_cancels_the_job_before_the_app_exits() {
        let mut rig = rig("uiclosequit");
        rig.app.job = Some(running_job());
        send(&mut rig.app, Message::CloseRequested);
        send(&mut rig.app, Message::Answer(true));
        assert!(rig.app.quit_pending);
        assert!(rig.app.job.as_ref().unwrap().cancel.is_cancelled());
        assert!(rig.app.job.is_some(), "the job reports back before the app exits");
    }

    #[test]
    fn job_completion_clears_state_and_hides_the_dock() {
        let mut rig = rig("uidone");
        send(&mut rig.app, Message::Navigate(Page::Applications));
        send(&mut rig.app, Message::AppToggled("7zip".to_owned()));
        rig.app.job = Some(ActiveJob {
            label: "Installing".to_owned(),
            fraction: 0.0,
            step: String::new(),
            cancel: CancelToken::new(),
            log: LogRing::new(8),
            queue: Some(Queue::Apps),
            private: false,
        });
        send(&mut rig.app, Message::Job(JobEvent::Progress { fraction: 0.5, step: "Step".to_owned() }));
        assert_eq!(rig.app.job.as_ref().unwrap().fraction, 0.5);

        send(&mut rig.app, Message::Job(JobEvent::Finished { outcome: Ok("Done".to_owned()) }));
        assert!(rig.app.job.is_none());
        assert!(rig.app.selected_apps.is_empty());
        assert_eq!(rig.app.notice.as_deref(), Some("Done"));
        settle(&mut rig.app);
        assert!(!rig.app.dock_visible());
    }

    #[test]
    fn a_job_only_consumes_its_own_queue() {
        let mut rig = rig("uiqueue");
        send(&mut rig.app, Message::Navigate(Page::Tweaks));
        send(&mut rig.app, Message::TweakChecked("activity".to_owned()));
        rig.app.job = Some(ActiveJob {
            label: "Dark Theme".to_owned(),
            fraction: 0.0,
            step: String::new(),
            cancel: CancelToken::new(),
            log: LogRing::new(8),
            queue: None,
            private: false,
        });
        send(&mut rig.app, Message::Job(JobEvent::Finished { outcome: Ok("Done".to_owned()) }));
        assert_eq!(rig.app.selected_tweaks.len(), 1, "a toggle must not wipe the queue");
    }

    #[test]
    fn a_failed_job_keeps_the_queue_and_offers_elevation() {
        let mut rig = rig("uifailed");
        send(&mut rig.app, Message::AppToggled("7zip".to_owned()));
        rig.app.job = Some(ActiveJob {
            label: "Installing".to_owned(),
            fraction: 0.0,
            step: "Step one".to_owned(),
            cancel: CancelToken::new(),
            log: LogRing::new(8),
            queue: Some(Queue::Apps),
            private: false,
        });
        let error = "administrator rights are required for RegSetValueExW".to_owned();
        send(&mut rig.app, Message::Job(JobEvent::Finished { outcome: Err(error) }));
        assert_eq!(rig.app.selected_apps.len(), 1);
        assert!(rig.app.needs_elevation);
        assert_eq!(rig.app.last_log_label, "Installing");
        assert!(rig.app.last_log.last().unwrap().starts_with("Failed:"));
        send(&mut rig.app, Message::DismissNotice);
        assert!(!rig.app.needs_elevation);
    }

    #[test]
    fn changing_page_clears_the_search() {
        let mut rig = rig("uisearchclear");
        send(&mut rig.app, Message::Query("telemetry".to_owned()));
        send(&mut rig.app, Message::Navigate(Page::Updates));
        assert!(rig.app.query.is_empty());
        assert!(rig.app.hits.is_empty());
    }

    #[test]
    fn escape_clears_the_search_before_closing_anything() {
        let mut rig = rig("uiescape");
        send(&mut rig.app, Message::Inspect(Inspected::Tweak("activity".to_owned())));
        send(&mut rig.app, Message::Query("telemetry".to_owned()));
        let escape = keyboard::Event::KeyPressed {
            key: Key::Named(key::Named::Escape),
            modified_key: Key::Named(key::Named::Escape),
            physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
            location: keyboard::Location::Standard,
            modifiers: Modifiers::default(),
            text: None,
            repeat: false,
        };
        send(&mut rig.app, Message::Keyboard(escape.clone()));
        assert!(rig.app.query.is_empty());
        assert!(rig.app.inspected.is_some());
        send(&mut rig.app, Message::Keyboard(escape));
        assert!(rig.app.inspected.is_none());
    }

    #[test]
    fn the_tab_indicator_follows_the_visible_page() {
        let mut rig = rig("uitabs");
        send(&mut rig.app, Message::Navigate(Page::Tweaks));
        send(&mut rig.app, Message::Tab(TweakTab::Appx));
        settle(&mut rig.app);
        assert_eq!(rig.app.timeline.channels[TAB].value(), 3.0);
        send(&mut rig.app, Message::Navigate(Page::Inspector));
        settle(&mut rig.app);
        assert_eq!(rig.app.timeline.channels[TAB].value(), 0.0, "this page has its own selected tab");
    }

    #[test]
    fn detect_applied_uses_fresh_results() {
        let mut rig = rig("uidetect");
        send(&mut rig.app, Message::DetectApplied);
        assert!(rig.app.selected_tweaks.is_empty(), "stale data must not drive the selection");
        let states = BTreeMap::from([("activity".to_owned(), Detection::Applied)]);
        send(&mut rig.app, Message::Detected(states.clone(), BTreeMap::new()));
        assert!(rig.app.selected_tweaks.contains("activity"));

        rig.app.selected_tweaks.clear();
        send(&mut rig.app, Message::Detected(states, BTreeMap::new()));
        assert!(rig.app.selected_tweaks.is_empty(), "a routine refresh must not select anything");
    }

    #[test]
    fn a_press_before_any_pointer_report_cannot_make_the_drawer_jump() {
        let mut rig = rig("uiarmed");
        send(&mut rig.app, Message::Inspect(Inspected::Tweak("activity".to_owned())));
        settle(&mut rig.app);
        send(&mut rig.app, Message::GripEnter);
        send(&mut rig.app, Message::GripPressed);
        send(&mut rig.app, Message::Pointer(Point::new(900.0, 300.0)));
        assert_eq!(rig.app.timeline.channels[DRAWER].value(), f64::from(INSPECTOR_WIDTH));
        send(&mut rig.app, Message::Pointer(Point::new(960.0, 300.0)));
        assert_eq!(rig.app.timeline.channels[DRAWER].value(), f64::from(INSPECTOR_WIDTH) - 60.0);
    }

    #[test]
    fn a_profile_from_the_command_line_opens_as_a_reviewable_selection() {
        let rig = rig("uiprofile");
        let mut profile = Profile::new();
        profile.tweaks.push("activity".to_owned());
        let start = StartView { page: Page::Tweaks, profile: Some(profile), ..StartView::default() };
        let (app, _) = App::new(rig.app.catalog.clone(), rig.app.engine.clone(), rig.app.info.clone(), start);
        assert_eq!(app.page, Page::Tweaks);
        assert!(app.selected_tweaks.contains("activity"));
        assert!(app.job.is_none(), "loading must not run anything");
    }

    #[test]
    fn every_page_builds_its_view() {
        let mut rig = rig("uiviews");
        for page in Page::ALL {
            send(&mut rig.app, Message::Navigate(page));
            drop(crate::views::view(&rig.app));
        }
        for tab in TweakTab::ALL {
            send(&mut rig.app, Message::Navigate(Page::Tweaks));
            send(&mut rig.app, Message::Tab(tab));
            drop(crate::views::view(&rig.app));
        }
        send(&mut rig.app, Message::Navigate(Page::Inspector));
        for tab in InspectorTab::ALL {
            send(&mut rig.app, Message::InspectorTab(tab));
            drop(crate::views::view(&rig.app));
        }
        send(&mut rig.app, Message::Query("telemetry".to_owned()));
        drop(crate::views::view(&rig.app));
    }
}
