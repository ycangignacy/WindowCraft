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
use windowcraft_config::{
    Answers, Area, Catalog, CleanScope, Depth, Feature, Goal, NativeOp, PresetName, Profile, SearchHit, SearchKind, StartPlan, Tweak,
    TweakKind,
};
use windowcraft_engine::autoruns::{self, AutorunEntry};
use windowcraft_engine::cleaner::{self, CleanTarget, OrphanCandidate, ScanResult};
use windowcraft_engine::dns_bench::{self, BenchResult, Target};
use windowcraft_engine::error::Result as EngineResult;
use windowcraft_engine::fixes;
use windowcraft_engine::gaming::{self, InterruptPriority, PciDevice, TimerResolution};
use windowcraft_engine::jobs::{self, CancelToken, JobContext, JobEvent, JobFailure, LogRing};
use windowcraft_engine::metadata;
use windowcraft_engine::convert;
use windowcraft_engine::tools::{self, Extension};
use windowcraft_engine::youtube::{self, Format, Stage};
use windowcraft_engine::packages::{self, InstalledProgram, PackageManager};
use windowcraft_engine::snapshot::{ChangeRecord, Session};
use windowcraft_engine::sysinfo::{self, SystemInfo};
use windowcraft_engine::{Detection, Drift, Engine, EngineError, InspectRow, RegPath, TweakAction};
use windowcraft_iso::usb::{DiskFacts, DiskIdentity};
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

    /// The tip this page shows on its first visit. Settings has none.
    pub fn area(self) -> Option<Area> {
        Some(match self {
            Self::Applications => Area::Applications,
            Self::Tweaks => Area::Tweaks,
            Self::System => Area::System,
            Self::Updates => Area::Updates,
            Self::Iso => Area::Iso,
            Self::TimeMachine => Area::TimeMachine,
            Self::Inspector => Area::Inspector,
            Self::Extensions => Area::Extensions,
            Self::Settings => return None,
        })
    }

    pub fn of_area(area: Area) -> Self {
        match area {
            Area::Applications => Self::Applications,
            Area::Tweaks => Self::Tweaks,
            Area::System => Self::System,
            Area::Updates => Self::Updates,
            Area::Iso => Self::Iso,
            Area::TimeMachine => Self::TimeMachine,
            Area::Inspector => Self::Inspector,
            Area::Extensions => Self::Extensions,
        }
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
    /// Started without `--open`, `--inspect`, `--config` or `--preset`, so
    /// the first-start questions and page tips may show. A deep link drives
    /// screenshots and tests, and nothing should cover what it opens.
    pub guided: bool,
}

impl Default for StartView {
    fn default() -> Self {
        Self { page: Page::Applications, inspect: None, profile: None, language: Language::En, mica: false, guided: false }
    }
}

/// Screens of the first-start questions: four questions and the summary.
pub const WIZARD_STEPS: usize = 5;
pub const SUMMARY_STEP: usize = WIZARD_STEPS - 1;

/// The open question screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wizard {
    pub step: usize,
    pub answers: Answers,
}

/// What a press on a question screen means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Onboard {
    Goal(Goal),
    KeepFiles(bool),
    BuildIso(bool),
    Depth(Depth),
    Back,
    Skip,
    Start,
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
    EraseDisk { number: u32, disk: DiskIdentity },
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
    /// A second launch handed over its command line, see `remote`.
    Remote(crate::remote::Request),
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
    /// None keeps what is shown, because the read failed.
    Refreshed(Option<Vec<Session>>, Option<Vec<Drift>>),
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
    /// Rows for the item they were read for; a late answer for another one is dropped.
    InspectRows(Inspected, Vec<InspectRow>),
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
    /// The outcome and whether the task was made to exist.
    ScheduleApplied(Result<(), String>, bool),
    CheckDriftNow,
    DriftChecked(Result<Vec<Drift>, String>, String),
    // Onboarding and page tips
    Onboarding(Onboard),
    /// "Next" on a tip: this page's tip is seen, and the next page with an
    /// unseen tip opens. On the last one the button reads "Done" and only closes.
    TipNext,
    /// "Don't show again" on a tip: no more tips.
    TipNever,
    /// A press outside the tip, Escape or a page change: closed, nothing saved.
    TipDismiss,
    ShowOnboardingAgain,
    ShowTipsAgain,
}

/// A selection that a job consumes when it succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Queue {
    Apps,
    Tweaks,
    Appx,
    Features,
}

/// What a job does, for the follow-up work when it ends. The label is only
/// for display and changes with the language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JobKind {
    #[default]
    Other,
    OpenIso,
}

pub struct ActiveJob {
    pub label: String,
    pub fraction: f32,
    pub step: String,
    pub cancel: CancelToken,
    pub log: LogRing,
    pub queue: Option<Queue>,
    /// Ids of the queue the job was started with. Items ticked while it runs
    /// are not in it and stay ticked.
    pub sent: Vec<String>,
    /// Ids the job finished, when it reports them one by one. Without it a
    /// successful job counts as having finished everything it was sent.
    pub done: Option<Arc<std::sync::Mutex<Vec<String>>>>,
    pub kind: JobKind,
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

pub struct ExtensionsState {
    pub url: String,
    pub format: Format,
    pub folder: String,
    pub target: convert::Target,
    /// Read when the page opens and after each job, not on every frame.
    pub installed: Vec<Extension>,
    /// ffmpeg is there, from the additional formats or the YouTube downloader.
    pub ffmpeg: bool,
}

impl ExtensionsState {
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
    /// A schedule change is running. The next one waits for it, see `save_schedule`.
    pub schedule_busy: bool,
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
    /// Parts of an imported profile that have no tick box: toggles and
    /// segmented options wait in the Tweaks queue, the update mode and the
    /// service rules are kept for export.
    pub imported: Profile,
    pub selected_appx: BTreeSet<String>,
    // System
    pub selected_features: BTreeSet<String>,
    pub fix_modes: BTreeMap<String, String>,
    pub update_mode: String,
    pub iso: IsoState,
    pub extensions: ExtensionsState,
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
    // Onboarding
    guided: bool,
    pub wizard: Option<Wizard>,
    pub tip: Option<Area>,
    /// What the cleaner ticks when its next scan arrives, set by "Start".
    clean_plan: Option<CleanScope>,
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
        let StartView { page, inspect, profile, language, mica, guided } = start;
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
            schedule_busy: false,
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
            imported: Profile::new(),
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
            extensions: ExtensionsState {
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
            guided,
            wizard: None,
            tip: None,
            clean_plan: None,
        };
        if guided && app.settings.onboarding.wants_questions() {
            app.wizard = Some(Wizard { step: 0, answers: app.settings.onboarding.answers });
        }
        if let Some(profile) = profile {
            let _ = app.apply_loaded_profile(profile);
        }
        let navigation = app.navigate(page);
        let mut startup = vec![app.refresh_detection(), app.refresh_time_machine(), app.refresh_update_mode(), navigation];
        if let Some(id) = inspect {
            if app.find_tweak(&id).is_some() {
                startup.push(app.open_inspector(Inspected::Tweak(id)));
            } else {
                app.notice = Some(format_1(t(app.language, Msg::NoticeUnknownInspect), &id));
            }
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
            Subscription::run(crate::remote::requests).map(Message::Remote),
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
            |engine| (engine.store.sessions().ok(), engine.detect_drift().ok()),
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

    fn selection(&self, queue: Queue) -> &BTreeSet<String> {
        match queue {
            Queue::Apps => &self.selected_apps,
            Queue::Tweaks => &self.selected_tweaks,
            Queue::Appx => &self.selected_appx,
            Queue::Features => &self.selected_features,
        }
    }

    fn selection_mut(&mut self, queue: Queue) -> &mut BTreeSet<String> {
        match queue {
            Queue::Apps => &mut self.selected_apps,
            Queue::Tweaks => &mut self.selected_tweaks,
            Queue::Appx => &mut self.selected_appx,
            Queue::Features => &mut self.selected_features,
        }
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

    /// Starts a job. When it ends, only the items of `queue` it finished are
    /// unticked, so a quick toggle never wipes a selection the user is still
    /// building.
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
        let sent = queue.map(|queue| self.selection(queue).iter().cloned().collect()).unwrap_or_default();
        self.job = Some(ActiveJob {
            label: label.to_owned(),
            fraction: 0.0,
            step: t(self.language, Msg::JobStarting).to_owned(),
            cancel: handle.cancel.clone(),
            log: handle.log.clone(),
            queue,
            sent,
            done: None,
            kind: JobKind::Other,
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
            Page::Tweaks => self.selected_tweaks.len() + self.imported.toggles.len() + self.imported.segmented.len(),
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
                self.animate(TAB, format_index(self.extensions.format));
                self.extensions.installed = tools::installed();
                self.extensions.ffmpeg = tools::installed_ffmpeg().is_some();
            }
            _ => {}
        }
        self.update_dock();
        // Opening a page closes the tip of the last one without saving it.
        self.tip = page.area().filter(|area| self.tip_allowed() && self.settings.onboarding.tip_due(*area));
        match page {
            Page::TimeMachine => self.refresh_time_machine(),
            Page::Inspector => self.load_inspector_tab(),
            _ => Task::none(),
        }
    }

    /// The next page after `from`, in sidebar order and around to the start,
    /// whose tip has not been seen.
    pub fn next_tip(&self, from: Area) -> Option<Area> {
        let count = Area::ALL.len();
        let start = Area::ALL.iter().position(|area| *area == from)?;
        (1..count)
            .map(|offset| Area::ALL[(start + offset) % count])
            .find(|area| *area != from && self.settings.onboarding.tip_due(*area))
    }

    fn tip_allowed(&self) -> bool {
        self.guided && self.wizard.is_none()
    }

    fn on_onboarding(&mut self, step: Onboard) -> Task<Message> {
        let Some(wizard) = &mut self.wizard else { return Task::none() };
        match step {
            Onboard::Goal(goal) => {
                wizard.answers.goal = Some(goal);
                wizard.step = 1;
            }
            Onboard::KeepFiles(keep) => {
                wizard.answers.keep_files = Some(keep);
                wizard.step = 2;
            }
            Onboard::BuildIso(build) => {
                wizard.answers.build_iso = Some(build);
                wizard.step = 3;
            }
            Onboard::Depth(depth) => {
                wizard.answers.depth = Some(depth);
                wizard.step = SUMMARY_STEP;
            }
            Onboard::Back => wizard.step = wizard.step.saturating_sub(1),
            Onboard::Skip => {
                let answers = wizard.answers;
                self.wizard = None;
                self.settings.onboarding.skip(answers);
                self.save_settings();
            }
            Onboard::Start => {
                let answers = wizard.answers;
                self.wizard = None;
                self.settings.onboarding.finish(answers);
                self.save_settings();
                return self.start_plan(answers.plan());
            }
        }
        Task::none()
    }

    /// A second launch, read with the same rules as a fresh start: `--config`
    /// or `--preset` loads a selection to review, `--open` picks the page and
    /// `--inspect` opens a known tweak. Open questions close, as they would
    /// never have opened over a deep link.
    fn on_remote(&mut self, request: &crate::remote::Request) -> Task<Message> {
        let arguments = std::iter::once("windowcraft".to_owned()).chain(request.args.iter().cloned());
        let Ok(mut cli) = <crate::cli::Cli as clap::Parser>::try_parse_from(arguments) else { return Task::none() };
        if cli.is_headless() {
            return Task::none();
        }
        if let Some(config) = &mut cli.config
            && config.is_relative()
        {
            *config = request.cwd.join(&*config);
        }
        let wants_selection = cli.config.is_some() || cli.preset.is_some();
        let mut page = cli.open.map(|page| page.0);
        if wants_selection {
            if self.job.is_some() {
                self.notice = Some(t(self.language, Msg::NoticeAnotherOperationRunning).to_owned());
            } else {
                match crate::cli::load_selection(&cli, &self.catalog) {
                    Ok(Some(resolved)) => {
                        let _ = self.apply_loaded_profile(resolved.profile);
                        page = page.or(Some(Page::Tweaks));
                    }
                    Ok(None) => {}
                    Err(error) => self.notice = Some(format_1(t(self.language, Msg::NoticeImportFailed), &error)),
                }
            }
        }
        if page.is_some() || cli.inspect.is_some() {
            self.wizard = None;
            self.tip = None;
        }
        let mut tasks = Vec::new();
        if let Some(page) = page {
            tasks.push(self.navigate(page));
        }
        if let Some(id) = cli.inspect {
            if self.find_tweak(&id).is_some() {
                tasks.push(self.open_inspector(Inspected::Tweak(id)));
            } else {
                self.notice = Some(format_1(t(self.language, Msg::NoticeUnknownInspect), &id));
            }
        }
        Task::batch(tasks)
    }

    /// Ticks a tweak and unticks the others in its `exclusive` group.
    fn select_tweak(&mut self, id: String) {
        for peer in self.catalog.exclusive_peers(&id) {
            self.selected_tweaks.remove(peer);
        }
        self.selected_tweaks.insert(id);
    }

    /// Ticks what the plan names and opens its first page. It adds to what
    /// is already ticked and runs nothing.
    fn start_plan(&mut self, plan: StartPlan) -> Task<Message> {
        for id in self.catalog.clone().preset(plan.preset) {
            self.select_tweak(id.clone());
        }
        if plan.appx_defaults {
            self.selected_appx.extend(self.catalog.presets.appx_default.iter().cloned());
        }
        self.clean_plan = Some(plan.clean);
        self.apply_clean_plan();
        if plan.first == Area::Inspector {
            self.inspector_tab = InspectorTab::Cleaner;
        } else {
            self.tab = TweakTab::Essential;
        }
        self.navigate(Page::of_area(plan.first))
    }

    /// Ticks the planned cleaner targets once a scan has found something.
    fn apply_clean_plan(&mut self) {
        if self.clean_results.is_empty() {
            return;
        }
        let Some(scope) = self.clean_plan.take() else { return };
        for (target, result) in &self.clean_results {
            if result.files > 0 && clean_scope_has(scope, target) {
                self.clean_selected.insert(target.id);
            }
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
        let Inspected::Tweak(id) = &inspected else {
            return Task::none();
        };
        let tweak = self.find_tweak(id);
        let option = self.options.get(id).cloned();
        self.blocking(
            move |engine| {
                let Some(tweak) = tweak else { return (inspected, Vec::new()) };
                let action = match tweak.kind {
                    TweakKind::Toggle => TweakAction::Set(true),
                    TweakKind::Segmented => TweakAction::Select(option.unwrap_or_default()),
                    _ => TweakAction::Apply,
                };
                (inspected, engine.inspect(&tweak, &action).unwrap_or_default())
            },
            |(inspected, rows)| Message::InspectRows(inspected, rows),
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
        let done = Arc::new(std::sync::Mutex::new(Vec::new()));
        let finished = done.clone();
        // A refused start leaves the running job in place, which must keep its own tracker.
        let busy = self.job.is_some();
        let task = self.start_queue_job(label, queue, move |context| {
            let total = items.len().max(1) as f32;
            let borrowed: Vec<(&Tweak, TweakAction)> = items.iter().map(|(tweak, action)| (tweak, action.clone())).collect();
            let report = engine.run(&session_label, &borrowed, &context.cancel, |index, name| {
                context.progress(index as f32 / total, name);
            })?;
            if let Ok(mut finished) = finished.lock() {
                finished.clone_from(&report.applied);
            }
            if report.cancelled {
                Ok(format_2(t(lang, Msg::NoticeCancelledAfter), &report.applied.len().to_string(), &items.len().to_string()))
            } else if report.failed.is_empty() {
                Ok(format_1(t(lang, Msg::NoticeItemsApplied), &report.applied.len().to_string()))
            } else {
                let (name, reason) = &report.failed[0];
                let first = format!("{name}: {reason}");
                Err(EngineError::Invalid(format_2(t(lang, Msg::NoticeFailures), &report.failed.len().to_string(), &first)))
            }
        });
        if !busy && let Some(job) = &mut self.job {
            job.done = Some(done);
        }
        task
    }

    fn selected_profile(&self) -> Profile {
        let mut profile = Profile::new();
        profile.tweaks = self.selected_tweaks.iter().cloned().collect();
        profile.apps = self.selected_apps.iter().cloned().collect();
        profile.features = self.selected_features.iter().cloned().collect();
        profile.appx_remove = self.selected_appx.iter().cloned().collect();
        profile.segmented = self.options.iter().filter(|(_, option)| option.as_str() != "Enabled").map(|(k, v)| (k.clone(), v.clone())).collect();
        // What an import queued but nothing applied yet goes out unchanged.
        profile.segmented.extend(self.imported.segmented.clone());
        profile.toggles = self.imported.toggles.clone();
        profile.update_mode.clone_from(&self.imported.update_mode);
        profile.services.clone_from(&self.imported.services);
        if self.dns_choice != "Default" {
            profile.dns = Some(self.dns_choice.clone());
        }
        profile
    }

    /// Takes over every section of a profile. Returns true when it holds an
    /// update mode or service rules, which the GUI keeps without applying.
    fn apply_loaded_profile(&mut self, profile: Profile) -> bool {
        self.selected_tweaks = profile.tweaks.into_iter().collect();
        self.selected_apps = profile.apps.into_iter().collect();
        self.selected_features = profile.features.into_iter().collect();
        self.selected_appx = profile.appx_remove.into_iter().collect();
        if let Some(dns) = profile.dns {
            self.dns_choice = dns;
        }
        self.imported = Profile {
            toggles: profile.toggles,
            segmented: profile.segmented,
            update_mode: profile.update_mode,
            services: profile.services,
            ..Profile::new()
        };
        self.update_dock();
        self.imported.update_mode.is_some() || !self.imported.services.is_empty()
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
                crate::remote::listen(handle);
            }
            Message::Remote(request) => return self.on_remote(&request),
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
            Message::ScheduleApplied(result, wanted) => {
                self.schedule_busy = false;
                if let Err(error) = result {
                    self.notice = Some(format_1(t(self.language, Msg::NoticeScheduleFailed), &error));
                } else if wanted != self.settings.wants_schedule() {
                    // The choice changed while the task was being written.
                    return self.save_schedule();
                }
            }
            Message::CheckDriftNow => {
                return self.blocking(
                    |engine| {
                        let build = sysinfo::query().map(|info| info.full_build()).unwrap_or_default();
                        (engine.detect_drift().map_err(|error| error.to_string()), build)
                    },
                    |(drift, build)| Message::DriftChecked(drift, build),
                );
            }
            Message::DriftChecked(drift, build) => {
                // A failed check is not a clean one: the list and the date stay.
                let drift = match drift {
                    Ok(drift) => drift,
                    Err(error) => {
                        self.notice = Some(format_1(t(self.language, Msg::NoticeDriftCheckFailed), &error));
                        self.notice_failed.clone_from(&self.notice);
                        return Task::none();
                    }
                };
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
                if self.selected_tweaks.contains(&id) {
                    self.selected_tweaks.remove(&id);
                } else {
                    self.select_tweak(id);
                }
                self.update_dock();
            }
            Message::Preset(preset) => {
                self.selected_tweaks.clear();
                for id in self.catalog.clone().preset(preset) {
                    self.select_tweak(id.clone());
                }
                self.update_dock();
            }
            Message::ClearTweaks => {
                self.selected_tweaks.clear();
                self.imported.toggles.clear();
                self.imported.segmented.clear();
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
                            let kept = self.apply_loaded_profile(resolved.profile);
                            let mut notice = format_1(t(self.language, Msg::NoticeImported), &path.display().to_string());
                            if kept {
                                notice.push_str(". ");
                                notice.push_str(t(self.language, Msg::NoticeImportedKept));
                            }
                            self.notice = Some(notice);
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
                        .map_err(iso_error)?;
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
                let busy = self.job.is_some();
                let task = self.start_job(t(lang, Msg::JobOpeningIso), move |context| {
                    session.save(&work).map_err(iso_error)?;
                    let images = pipeline::extract(&mut session, &work, context)
                        .map_err(iso_error)?;
                    Ok(format_1(t(lang, Msg::NoticeFoundEditions), &images.len().to_string()))
                });
                if !busy && let Some(job) = &mut self.job {
                    job.kind = JobKind::OpenIso;
                }
                return task;
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
                    self.confirm = Some(Confirm::EraseDisk { number, disk: disk.identity.clone() });
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
            Message::VideoUrl(url) => self.extensions.url = url,
            Message::VideoFormat(format) => {
                self.extensions.format = format;
                self.animate(TAB, format_index(format));
            }
            Message::VideoFolder(folder) => self.extensions.folder = folder,
            Message::VideoBrowseFolder => {
                return Task::perform(
                    platform::pick_folder(t(self.language, Msg::DialogSelectFolderTitle)),
                    Message::VideoFolderPicked,
                );
            }
            Message::VideoFolderPicked(folder) => {
                if let Some(folder) = folder {
                    self.extensions.folder = folder.display().to_string();
                }
            }
            Message::VideoDownload => {
                let url = self.extensions.url.trim().to_owned();
                if !youtube::is_youtube_url(&url) {
                    self.notice = Some(t(self.language, Msg::NoticeInvalidYoutubeUrl).to_owned());
                    return Task::none();
                }
                let folder = if self.extensions.folder.trim().is_empty() {
                    youtube::downloads_dir()
                } else {
                    PathBuf::from(self.extensions.folder.trim())
                };
                let (lang, format) = (self.language, self.extensions.format);
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
            Message::ConvertTarget(target) => self.extensions.target = target,
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
                let (lang, target) = (self.language, self.extensions.target);
                return self.start_private_job(&format_1(t(lang, Msg::JobConverting), target.label()), move |context| {
                    convert_files(lang, &files, target, context)
                });
            }
            Message::Refreshed(sessions, drift) => {
                if let Some(sessions) = sessions {
                    self.sessions = sessions;
                }
                if let Some(drift) = drift {
                    self.drift = drift;
                }
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
                self.apply_clean_plan();
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
                        context.progress(index as f32 / total, i18n::clean_target(lang, target).0);
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
            Message::InspectRows(inspected, rows) => {
                if self.inspected.as_ref() == Some(&inspected) {
                    self.inspect_rows = rows;
                }
            }
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
            Message::Onboarding(step) => return self.on_onboarding(step),
            Message::TipNext => {
                if let Some(area) = self.tip.take() {
                    self.settings.onboarding.tip_seen(area);
                    self.save_settings();
                    if let Some(next) = self.next_tip(area) {
                        return self.navigate(Page::of_area(next));
                    }
                }
            }
            Message::TipNever => {
                if self.tip.take().is_some() {
                    self.settings.onboarding.tips_never();
                    self.save_settings();
                }
            }
            Message::TipDismiss => self.tip = None,
            // Both buttons are an explicit request, so they also turn guidance
            // on in a window that a deep link opened.
            Message::ShowOnboardingAgain => {
                self.settings.onboarding.ask_again();
                self.save_settings();
                self.tip = None;
                self.guided = true;
                self.wizard = Some(Wizard { step: 0, answers: self.settings.onboarding.answers });
            }
            Message::ShowTipsAgain => {
                self.guided = true;
                self.settings.onboarding.reset_tips();
                // Tips wait for the questions. Questions that are not on screen
                // count as skipped here, or the tips would never come back.
                if self.settings.onboarding.wants_questions() && self.wizard.is_none() {
                    let answers = self.settings.onboarding.answers;
                    self.settings.onboarding.skip(answers);
                }
                if self.save_settings() {
                    self.notice = Some(t(self.language, Msg::NoticeTipsReset).to_owned());
                }
            }
            Message::SetLanguage(lang) => {
                self.language = lang;
                i18n::set_current(lang);
                match Catalog::load_in(lang.tag()) {
                    Ok(catalog) => {
                        self.catalog = Arc::new(catalog);
                        // Category filters hold the shown names.
                        self.categories.clear();
                        if !self.query.trim().is_empty() {
                            self.hits = windowcraft_config::search(&self.catalog, &self.query, 60);
                        }
                    }
                    Err(error) => self.notice = Some(error),
                }
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

    /// Saves the settings file. A failure shows a notice, and the choice
    /// still holds until the app closes, so nothing waits on a file that
    /// cannot be written.
    fn save_settings(&mut self) -> bool {
        match settings::save(&self.settings) {
            Ok(()) => true,
            Err(error) => {
                self.notice = Some(format_1(t(self.language, Msg::NoticeFailedToSaveSettings), &error));
                false
            }
        }
    }

    /// Saves the check settings, then creates or removes the scheduled task.
    /// Saves the choice and brings the scheduled task in line with it. Only
    /// one change runs at a time, so an older delete can never land after a
    /// newer create; the one that ends checks the choice again.
    fn save_schedule(&mut self) -> Task<Message> {
        self.save_settings();
        if self.schedule_busy {
            return Task::none();
        }
        self.schedule_busy = true;
        let chosen = self.settings.clone();
        let wanted = chosen.wants_schedule();
        let description = t(self.language, Msg::SettingsTaskDescription).to_owned();
        self.blocking(move |_| settings::apply_schedule(&chosen, &description), move |result| Message::ScheduleApplied(result, wanted))
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
            .chain(self.imported_items())
            .collect();
        self.run_items_for(t(self.language, Msg::JobApplyingTweaks), Some(Queue::Tweaks), items)
    }

    /// Toggles and segmented options of an imported profile, as tweak actions.
    fn imported_items(&self) -> Vec<(Tweak, TweakAction)> {
        let toggles = self.imported.toggles.iter().map(|(id, on)| (id, TweakAction::Set(*on)));
        let segmented = self.imported.segmented.iter().map(|(id, option)| (id, TweakAction::Select(option.clone())));
        toggles
            .chain(segmented)
            .filter_map(|(id, action)| self.catalog.tweak(id).map(|tweak| (tweak.clone(), action)))
            .collect()
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
                Key::Character("k" | "f") if modifiers.control() && self.wizard.is_none() => {
                    return Task::batch([
                        widget::operation::focus(search_id()),
                        widget::operation::select_all(search_id()),
                    ]);
                }
                // On a question screen 1 to 4 pick the answer with that number, and
                // Enter on the summary starts. Nothing else listens while it is open.
                Key::Character(digit) if self.wizard.is_some() && self.confirm.is_none() => {
                    let step = self.wizard.map_or(0, |wizard| wizard.step);
                    if let Some(answer) = digit.parse::<usize>().ok().and_then(|number| wizard_answer(step, number)) {
                        return self.on_onboarding(answer);
                    }
                }
                Key::Named(key::Named::Enter)
                    if self.confirm.is_none() && self.wizard.is_some_and(|wizard| wizard.step == SUMMARY_STEP) =>
                {
                    return self.on_onboarding(Onboard::Start);
                }
                Key::Named(key::Named::Escape) => {
                    if self.confirm.is_some() {
                        self.confirm = None;
                    } else if self.wizard.is_some() {
                        return self.on_onboarding(Onboard::Skip);
                    } else if self.tip.is_some() {
                        self.tip = None;
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

    fn iso_job(&mut self, disk: Option<(u32, DiskIdentity)>) -> Task<Message> {
        let mut session = self.studio_session();
        session.step = windowcraft_iso::Step::Extracted;
        let (work, output) = (work_dir(), PathBuf::from(&self.iso.output));
        let catalog = self.catalog.clone();
        let lang = self.language;
        self.start_job(t(lang, Msg::JobBuildingMedia), move |context| {
            let wrap = iso_error;
            pipeline::configure(&mut session, &work, &catalog, context).map_err(wrap)?;
            match disk {
                Some((number, identity)) => {
                    pipeline::export_usb(&mut session, &work, &catalog, number, &identity, context).map_err(wrap)?;
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
            Confirm::EraseDisk { number, disk } => self.iso_job(Some((number, disk))),
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
                let mut job = self.job.take();
                let label = job.as_ref().map(|job| job.label.clone()).unwrap_or_default();
                let queue = job.as_ref().and_then(|job| job.queue);
                let kind = job.as_ref().map_or(JobKind::Other, |job| job.kind);
                let private = job.as_ref().is_some_and(|job| job.private);
                // Reported items leave the queue whatever the outcome; without
                // a report, a success means everything that was sent.
                let finished: Vec<String> = match job.as_mut() {
                    Some(job) => match &job.done {
                        Some(done) => done.lock().map(|done| done.clone()).unwrap_or_default(),
                        None if outcome.is_ok() => std::mem::take(&mut job.sent),
                        None => Vec::new(),
                    },
                    None => Vec::new(),
                };
                let mut lines = job.map_or_else(Vec::new, |job| job.log.snapshot());
                lines.push(match &outcome {
                    Ok(summary) => summary.clone(),
                    Err(error) => format_1(t(self.language, Msg::NoticeFailedPrefix), &failure_text(self.language, error)),
                });
                if private {
                    self.extensions.installed = tools::installed();
                self.extensions.ffmpeg = tools::installed_ffmpeg().is_some();
                } else {
                    actions::append_log(&windowcraft_engine::win::local_app_dir().join("windowcraft.log"), &label, &lines);
                }
                self.last_log_label.clone_from(&label);
                self.last_log = lines;
                if self.quit_pending {
                    return iced::exit();
                }

                if let Some(queue) = queue {
                    let selection = self.selection_mut(queue);
                    for id in &finished {
                        selection.remove(id);
                    }
                    if queue == Queue::Tweaks {
                        for id in &finished {
                            self.imported.toggles.remove(id);
                            self.imported.segmented.remove(id);
                        }
                    }
                }
                match outcome {
                    Ok(summary) => self.notice = Some(summary),
                    Err(error) => {
                        self.needs_elevation = error.needs_elevation;
                        self.notice = Some(format!("{label}: {}", failure_text(self.language, &error)));
                        self.notice_failed.clone_from(&self.notice);
                    }
                }
                self.update_dock();
                if let Some(window) = self.window {
                    platform::taskbar_progress(window, None);
                }
                let mut tasks = vec![self.refresh_all()];
                if kind == JobKind::OpenIso {
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

/// Keeps the engine error inside an ISO Studio error, so its kind still
/// decides the wording and the elevation offer.
fn iso_error(error: windowcraft_iso::IsoError) -> EngineError {
    use windowcraft_iso::IsoError;
    match error {
        IsoError::Engine(inner) => inner,
        IsoError::Io { target, source } => EngineError::Io { target, source },
        other => EngineError::Message(other.to_string()),
    }
}

/// What a failed job says. A panic has no message worth showing.
fn failure_text(lang: Language, failure: &JobFailure) -> String {
    if failure.panicked { t(lang, Msg::ErrJobPanicked).to_owned() } else { failure.message.clone() }
}

/// TempOnly is the System group: temporary files, CBS and DISM logs and
/// minidumps. Caches adds every other group except ISO Studio's own work
/// files, which hold a build that can be resumed.
fn clean_scope_has(scope: CleanScope, target: &CleanTarget) -> bool {
    match scope {
        CleanScope::TempOnly => target.group == "System",
        CleanScope::Caches => target.id != cleaner::ISO_TARGET,
    }
}

/// The answer with `number` (from 1) on question screen `step`, in the order
/// the screen lists them.
fn wizard_answer(step: usize, number: usize) -> Option<Onboard> {
    let index = number.checked_sub(1)?;
    match step {
        0 => Goal::ALL.get(index).copied().map(Onboard::Goal),
        1 => [true, false].get(index).copied().map(Onboard::KeepFiles),
        2 => [true, false].get(index).copied().map(Onboard::BuildIso),
        3 => Depth::ALL.get(index).copied().map(Onboard::Depth),
        _ => None,
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
        rig_with(name, StartView::default())
    }

    /// A plain start with no deep link, as from the Start menu.
    fn guided() -> StartView {
        StartView { guided: true, ..StartView::default() }
    }

    fn rig_with(name: &str, start: StartView) -> Rig {
        let sandbox = Sandbox::new(name);
        let dir = tempfile::tempdir().unwrap();
        // Each test thread reads and writes a settings file of its own.
        settings::use_test_file(Some(dir.path().join("settings.json")));
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
        let (mut app, _) = App::new(catalog, engine, info, start);
        app.timeline.mode = MotionMode::Spring;
        Rig { app, _sandbox: sandbox, _dir: dir }
    }

    /// A second start of the app on the same settings file.
    fn restart(app: &App, start: StartView) -> App {
        App::new(app.catalog.clone(), app.engine.clone(), app.info.clone(), start).0
    }

    /// A guided start whose questions were already answered.
    fn toured(name: &str) -> Rig {
        let mut rig = rig_with(name, guided());
        send(&mut rig.app, Message::Onboarding(Onboard::Skip));
        assert!(rig.app.wizard.is_none());
        rig
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
    fn ticking_a_tweak_unticks_the_rest_of_its_exclusive_group() {
        let mut rig = rig("uiexclusive");
        for id in ["ipv46", "activity", "teredo"] {
            send(&mut rig.app, Message::TweakChecked(id.to_owned()));
        }
        assert_eq!(rig.app.selected_tweaks.iter().map(String::as_str).collect::<Vec<_>>(), ["activity", "teredo"]);
        send(&mut rig.app, Message::TweakChecked("disable-ipv6".to_owned()));
        assert_eq!(rig.app.selected_tweaks.iter().map(String::as_str).collect::<Vec<_>>(), ["activity", "disable-ipv6"]);
        send(&mut rig.app, Message::TweakChecked("disable-ipv6".to_owned()));
        assert_eq!(rig.app.selected_tweaks.iter().map(String::as_str).collect::<Vec<_>>(), ["activity"]);
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
            sent: Vec::new(),
            done: None,
            kind: JobKind::Other,
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
            sent: vec!["7zip".to_owned()],
            done: None,
            kind: JobKind::Other,
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
            sent: Vec::new(),
            done: None,
            kind: JobKind::Other,
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
            sent: vec!["7zip".to_owned()],
            done: None,
            kind: JobKind::Other,
            private: false,
        });
        let error = JobFailure::from_error(&EngineError::NeedsElevation("RegSetValueExW".to_owned()));
        send(&mut rig.app, Message::Job(JobEvent::Finished { outcome: Err(error) }));
        assert_eq!(rig.app.selected_apps.len(), 1);
        assert!(rig.app.needs_elevation);
        assert_eq!(rig.app.last_log_label, "Installing");
        assert!(rig.app.last_log.last().unwrap().starts_with("Failed:"));
        send(&mut rig.app, Message::DismissNotice);
        assert!(!rig.app.needs_elevation);

        // Elevation comes from the error variant, not from its English text.
        rig.app.job = Some(ActiveJob { queue: None, sent: Vec::new(), ..job("Installing") });
        let other = JobFailure::from_error(&EngineError::Invalid("administrator rights".to_owned()));
        send(&mut rig.app, Message::Job(JobEvent::Finished { outcome: Err(other) }));
        assert!(!rig.app.needs_elevation);
    }

    fn job(label: &str) -> ActiveJob {
        ActiveJob {
            label: label.to_owned(),
            fraction: 0.0,
            step: String::new(),
            cancel: CancelToken::new(),
            log: LogRing::new(8),
            queue: Some(Queue::Tweaks),
            sent: vec!["activity".to_owned(), "teredo".to_owned()],
            done: None,
            kind: JobKind::Other,
            private: false,
        }
    }

    fn remote(cwd: &std::path::Path, args: &[&str]) -> Message {
        Message::Remote(crate::remote::Request { cwd: cwd.to_path_buf(), args: args.iter().map(|arg| (*arg).to_owned()).collect() })
    }

    #[test]
    fn a_second_launch_opens_its_page_and_closes_the_questions() {
        let mut rig = rig_with("uiremotepage", guided());
        assert!(rig.app.wizard.is_some());
        let cwd = rig._dir.path().to_path_buf();
        send(&mut rig.app, remote(&cwd, &["--open", "iso"]));
        assert_eq!(rig.app.page, Page::Iso);
        assert_eq!(rig.app.wizard, None);
        assert!(rig.app.settings.onboarding.wants_questions(), "closing them is not an answer");

        send(&mut rig.app, remote(&cwd, &["--inspect", "no-such-tweak"]));
        assert_eq!(rig.app.inspected, None);
        assert!(rig.app.notice.as_deref().is_some_and(|notice| notice.contains("no-such-tweak")));

        // Headless flags and garbage change nothing.
        send(&mut rig.app, remote(&cwd, &["--check-drift"]));
        send(&mut rig.app, remote(&cwd, &["--no-such-flag"]));
        assert_eq!(rig.app.page, Page::Iso);
    }

    #[test]
    fn a_second_launch_with_a_relative_config_loads_it_unless_a_job_runs() {
        let mut rig = rig("uiremoteconfig");
        let cwd = rig._dir.path().to_path_buf();
        std::fs::write(cwd.join("mine.json"), "\u{FEFF}{\"version\":1,\"tweaks\":[\"activity\"]}").unwrap();

        rig.app.job = Some(job("Busy"));
        send(&mut rig.app, remote(&cwd, &["--config", "mine.json"]));
        assert!(rig.app.selected_tweaks.is_empty(), "a running job keeps its queue");
        rig.app.job = None;

        send(&mut rig.app, remote(&cwd, &["--config", "mine.json"]));
        assert_eq!(rig.app.selected_tweaks.iter().map(String::as_str).collect::<Vec<_>>(), ["activity"]);
        assert_eq!(rig.app.page, Page::Tweaks);
    }

    #[test]
    fn late_inspector_rows_for_another_tweak_are_dropped() {
        let mut rig = rig("uiinspectrace");
        drop(rig.app.update(Message::Inspect(Inspected::Tweak("activity".to_owned()))));
        drop(rig.app.update(Message::Inspect(Inspected::Tweak("telemetry".to_owned()))));
        let row = InspectRow {
            target: "A".to_owned(),
            kind: "DWord".to_owned(),
            current: String::new(),
            desired: String::new(),
            undo: String::new(),
            matches: false,
        };
        send(&mut rig.app, Message::InspectRows(Inspected::Tweak("activity".to_owned()), vec![row.clone()]));
        assert!(rig.app.inspect_rows.is_empty(), "rows of the first tweak must not show under the second");
        send(&mut rig.app, Message::InspectRows(Inspected::Tweak("telemetry".to_owned()), vec![row]));
        assert_eq!(rig.app.inspect_rows.len(), 1);
    }

    #[test]
    fn a_schedule_change_waits_for_the_one_running_and_then_catches_up() {
        let mut rig = rig("uischedule");
        rig.app.schedule_busy = true;
        send(&mut rig.app, Message::SetInterval(CheckInterval::Daily));
        assert!(rig.app.schedule_busy, "the second change waits");
        // The running change removed the task; the choice now wants one.
        send(&mut rig.app, Message::ScheduleApplied(Ok(()), false));
        assert!(rig.app.schedule_busy, "a new change starts to match the choice");
        send(&mut rig.app, Message::ScheduleApplied(Ok(()), true));
        assert!(!rig.app.schedule_busy);
    }

    #[test]
    fn tips_come_back_after_a_deep_link_start_that_never_showed_the_questions() {
        let mut rig = rig("uitipsdeep");
        assert!(rig.app.settings.onboarding.wants_questions());
        send(&mut rig.app, Message::ShowTipsAgain);
        assert!(rig.app.settings.onboarding.tip_due(Area::Tweaks));
    }

    #[test]
    fn inspect_with_an_unknown_id_says_so_and_opens_nothing() {
        let rig = rig_with("uiinspectbogus", StartView { inspect: Some("no-such-tweak".to_owned()), ..StartView::default() });
        assert_eq!(rig.app.inspected, None);
        assert!(rig.app.notice.as_deref().is_some_and(|notice| notice.contains("no-such-tweak")));
    }

    #[test]
    fn an_imported_profile_comes_back_whole_on_export() {
        let mut rig = rig("uiprofile");
        let text = r#"{
            "version": 1,
            "tweaks": ["activity"],
            "toggles": {"taskbar-alignment": false},
            "segmented": {"multiplane-overlay": "Fully Disabled"},
            "features": ["dotnet"],
            "apps": ["7zip"],
            "appx_remove": ["microsoft-get-help"],
            "dns": "Cloudflare",
            "update_mode": "security",
            "services": [{"name": "DiagTrack", "startup": "Disabled", "original": "Automatic"}]
        }"#;
        let profile = windowcraft_config::import_any(text, &rig.app.catalog).unwrap().profile;
        assert!(rig.app.apply_loaded_profile(profile.clone()), "the update mode and services are only kept");
        assert_eq!(rig.app.selected_profile(), profile);

        // Toggles and options wait in the Tweaks queue next to the ticked tweak.
        send(&mut rig.app, Message::Navigate(Page::Tweaks));
        assert_eq!(rig.app.queued_count(), 3);
        let actions: Vec<(String, TweakAction)> =
            rig.app.imported_items().into_iter().map(|(tweak, action)| (tweak.id, action)).collect();
        assert_eq!(
            actions,
            [
                ("taskbar-alignment".to_owned(), TweakAction::Set(false)),
                ("multiplane-overlay".to_owned(), TweakAction::Select("Fully Disabled".to_owned())),
            ]
        );
    }

    #[test]
    fn a_failed_drift_check_keeps_the_list_and_the_date() {
        let mut rig = rig("uidriftfail");
        let drift = Drift { tweak_id: "activity".to_owned(), tweak_name: "Activity".to_owned(), entries: Vec::new(), after_update: false };
        rig.app.drift = vec![drift.clone()];
        rig.app.settings.last_check = 42;
        send(&mut rig.app, Message::DriftChecked(Err("store busy".to_owned()), "26300.1".to_owned()));
        assert_eq!(rig.app.drift, vec![drift.clone()]);
        assert_eq!(rig.app.settings.last_check, 42);
        assert!(rig.app.notice.as_deref().is_some_and(|notice| notice.contains("store busy")));

        send(&mut rig.app, Message::Refreshed(None, None));
        assert_eq!(rig.app.drift, vec![drift]);

        send(&mut rig.app, Message::DriftChecked(Ok(Vec::new()), String::new()));
        assert!(rig.app.drift.is_empty());
        assert_ne!(rig.app.settings.last_check, 42);
    }

    #[test]
    fn a_cancelled_batch_unticks_only_what_it_applied() {
        let mut rig = rig("uicancelled");
        for id in ["activity", "teredo"] {
            send(&mut rig.app, Message::TweakChecked(id.to_owned()));
        }
        let done = Arc::new(std::sync::Mutex::new(vec!["activity".to_owned()]));
        rig.app.job = Some(ActiveJob { done: Some(done), ..job("Applying") });
        let summary = format_2(t(Language::En, Msg::NoticeCancelledAfter), "1", "2");
        send(&mut rig.app, Message::Job(JobEvent::Finished { outcome: Ok(summary.clone()) }));
        assert_eq!(rig.app.selected_tweaks.iter().map(String::as_str).collect::<Vec<_>>(), ["teredo"]);
        assert_eq!(rig.app.notice.as_deref(), Some(summary.as_str()));
    }

    #[test]
    fn a_tweak_ticked_while_a_job_runs_stays_ticked() {
        let mut rig = rig("uiticked");
        for id in ["activity", "teredo"] {
            send(&mut rig.app, Message::TweakChecked(id.to_owned()));
        }
        rig.app.job = Some(job("Applying"));
        send(&mut rig.app, Message::TweakChecked("show-ext".to_owned()));
        send(&mut rig.app, Message::Job(JobEvent::Finished { outcome: Ok("Done".to_owned()) }));
        assert_eq!(rig.app.selected_tweaks.iter().map(String::as_str).collect::<Vec<_>>(), ["show-ext"]);
    }

    #[test]
    fn opening_an_iso_is_known_by_its_kind_in_any_language() {
        let mut rig = rig("uiisokind");
        send(&mut rig.app, Message::IsoOpen);
        assert_eq!(rig.app.job.as_ref().map(|job| job.kind), Some(JobKind::OpenIso));
        rig.app.job.as_mut().unwrap().cancel.cancel();
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

    #[test]
    fn the_questions_open_on_a_plain_first_start_and_never_over_a_deep_link() {
        let rig = rig_with("onbfirst", guided());
        assert_eq!(rig.app.wizard, Some(Wizard { step: 0, answers: Answers::default() }));
        assert_eq!(rig.app.tip, None, "no tip under the questions");

        let linked = rig_with("onblinked", StartView::default());
        assert_eq!(linked.app.wizard, None);
        assert_eq!(linked.app.tip, None);
    }

    #[test]
    fn every_screen_can_be_skipped_and_a_skip_is_kept_after_a_restart() {
        let steps = [
            Message::Onboarding(Onboard::Goal(Goal::Privacy)),
            Message::Onboarding(Onboard::KeepFiles(true)),
            Message::Onboarding(Onboard::BuildIso(false)),
            Message::Onboarding(Onboard::Depth(Depth::Careful)),
        ];
        for screen in 0..WIZARD_STEPS {
            let mut rig = rig_with("onbskip", guided());
            for message in steps.iter().take(screen).cloned() {
                send(&mut rig.app, message);
            }
            assert_eq!(rig.app.wizard.map(|wizard| wizard.step), Some(screen));
            send(&mut rig.app, Message::Onboarding(Onboard::Skip));
            assert_eq!(rig.app.wizard, None, "screen {screen}");
            assert!(rig.app.selected_tweaks.is_empty(), "a skip ticks nothing");
            assert_eq!(settings::load().onboarding.status, windowcraft_config::onboarding::Status::Skipped);
            assert_eq!(restart(&rig.app, guided()).wizard, None, "screen {screen}: not shown a second time");
        }
    }

    #[test]
    fn escape_skips_the_questions_and_closes_a_tip_without_saving() {
        let mut rig = rig_with("onbescape", guided());
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
        assert_eq!(rig.app.wizard, None);
        assert!(!settings::load().onboarding.wants_questions());

        send(&mut rig.app, Message::Navigate(Page::System));
        assert_eq!(rig.app.tip, Some(Area::System));
        send(&mut rig.app, Message::Keyboard(escape));
        assert_eq!(rig.app.tip, None);
        assert!(settings::load().onboarding.tip_due(Area::System), "Escape saves nothing");
    }

    #[test]
    fn answers_move_forward_back_keeps_them_and_start_only_ticks_the_plan() {
        let mut rig = rig_with("onbstart", guided());
        send(&mut rig.app, Message::Onboarding(Onboard::Goal(Goal::Gaming)));
        send(&mut rig.app, Message::Onboarding(Onboard::KeepFiles(false)));
        send(&mut rig.app, Message::Onboarding(Onboard::BuildIso(true)));
        send(&mut rig.app, Message::Onboarding(Onboard::Back));
        send(&mut rig.app, Message::Onboarding(Onboard::Back));
        let wizard = rig.app.wizard.unwrap();
        assert_eq!(wizard.step, 1);
        assert_eq!(wizard.answers.build_iso, Some(true), "Back keeps later answers");
        send(&mut rig.app, Message::Onboarding(Onboard::KeepFiles(false)));
        send(&mut rig.app, Message::Onboarding(Onboard::BuildIso(true)));
        send(&mut rig.app, Message::Onboarding(Onboard::Depth(Depth::Everything)));
        assert_eq!(rig.app.wizard.unwrap().step, SUMMARY_STEP);
        drop(crate::views::view(&rig.app));

        send(&mut rig.app, Message::Onboarding(Onboard::Start));
        assert_eq!(rig.app.wizard, None);
        assert_eq!(rig.app.selected_tweaks.len(), rig.app.catalog.preset(PresetName::Advanced).len());
        assert_eq!(rig.app.selected_appx.len(), rig.app.catalog.presets.appx_default.len());
        assert_eq!(rig.app.page, Page::Tweaks);
        assert!(rig.app.job.is_none() && rig.app.confirm.is_none(), "Start runs nothing");
        assert_eq!(rig.app.tip, Some(Area::Tweaks), "the first page shows its tip");

        let saved = settings::load().onboarding;
        assert_eq!(saved.status, windowcraft_config::onboarding::Status::Finished);
        assert_eq!(saved.answers.goal, Some(Goal::Gaming));
        assert_eq!(restart(&rig.app, guided()).wizard, None);
    }

    fn scan() -> Vec<(CleanTarget, ScanResult)> {

        cleaner::targets().into_iter().map(|target| (target, ScanResult { bytes: 4096, files: 2 })).collect()
    }

    #[test]
    fn the_cleaner_ticks_only_what_the_answer_about_files_allows() {
        let mut rig = rig_with("onbtidy", guided());
        send(&mut rig.app, Message::Onboarding(Onboard::Goal(Goal::Tidy)));
        send(&mut rig.app, Message::Onboarding(Onboard::KeepFiles(true)));
        send(&mut rig.app, Message::Onboarding(Onboard::BuildIso(false)));
        send(&mut rig.app, Message::Onboarding(Onboard::Depth(Depth::Careful)));
        send(&mut rig.app, Message::Onboarding(Onboard::Start));
        assert_eq!(rig.app.page, Page::Inspector);
        assert_eq!(rig.app.inspector_tab, InspectorTab::Cleaner);
        assert!(rig.app.clean_selected.is_empty(), "nothing before a scan");
        let results = scan();
        send(&mut rig.app, Message::CleanResults(results, Vec::new()));
        let expected: BTreeSet<&str> = ["temp-user", "temp-system", "cbs-logs", "minidump"].into_iter().collect();
        assert_eq!(rig.app.clean_selected, expected);
        assert!(rig.app.selected_orphans.is_empty());

        // The plan applies to the first scan only; later scans keep the user's ticks.
        send(&mut rig.app, Message::CleanToggled("temp-user"));
        let results = scan();
        send(&mut rig.app, Message::CleanResults(results, Vec::new()));
        assert!(!rig.app.clean_selected.contains("temp-user"));

        let mut new = rig_with("onbnew", guided());
        for message in [Onboard::Goal(Goal::Tidy), Onboard::KeepFiles(false), Onboard::BuildIso(false), Onboard::Depth(Depth::Careful), Onboard::Start] {
            send(&mut new.app, Message::Onboarding(message));
        }
        let results = scan();
        send(&mut new.app, Message::CleanResults(results, Vec::new()));
        assert!(new.app.clean_selected.contains("npm") && new.app.clean_selected.contains("shader-d3d"));
        assert!(!new.app.clean_selected.contains(cleaner::ISO_TARGET), "ISO Studio work files stay");
    }

    #[test]
    fn each_tip_shows_once_a_press_outside_saves_nothing_and_never_turns_all_off() {
        let mut rig = toured("tiponce");
        send(&mut rig.app, Message::Navigate(Page::Updates));
        assert_eq!(rig.app.tip, Some(Area::Updates));
        send(&mut rig.app, Message::TipNext);
        assert_eq!(rig.app.page, Page::Iso, "Next opens the next page");
        assert_eq!(rig.app.tip, Some(Area::Iso));
        send(&mut rig.app, Message::TipDismiss);
        send(&mut rig.app, Message::Navigate(Page::Updates));
        assert_eq!(rig.app.tip, None, "seen with Next");
        send(&mut rig.app, Message::Navigate(Page::Iso));
        assert_eq!(rig.app.tip, Some(Area::Iso), "a dismissed tip comes back");
        send(&mut rig.app, Message::Navigate(Page::Settings));
        assert_eq!(rig.app.tip, None, "a page change closes it, Settings has none");

        let saved = settings::load().onboarding;
        assert!(saved.tips_seen.contains(&Area::Updates) && !saved.tips_seen.contains(&Area::Iso));

        send(&mut rig.app, Message::Navigate(Page::System));
        send(&mut rig.app, Message::TipNever);
        for page in Page::ALL {
            send(&mut rig.app, Message::Navigate(page));
            assert_eq!(rig.app.tip, None, "{page:?}");
        }
        assert!(settings::load().onboarding.tips_off);
        let again = restart(&rig.app, guided());
        assert_eq!((again.wizard, again.tip), (None, None));
    }

    #[test]
    fn next_walks_through_every_page_in_sidebar_order_and_skips_seen_tips() {
        let mut rig = toured("tiptour");
        send(&mut rig.app, Message::Navigate(Page::Tweaks));
        send(&mut rig.app, Message::Navigate(Page::Iso));
        assert_eq!(rig.app.tip, Some(Area::Iso));
        send(&mut rig.app, Message::TipNext);
        // Tweaks was only opened, not closed with Next, so its tip is still due.
        let order = [Page::TimeMachine, Page::Inspector, Page::Extensions, Page::Applications, Page::Tweaks, Page::System, Page::Updates];
        for (index, page) in order.into_iter().enumerate() {
            assert_eq!(rig.app.page, page, "step {index}");
            assert_eq!(rig.app.tip, page.area());
            let last = index + 1 == order.len();
            assert_eq!(rig.app.next_tip(page.area().unwrap()).is_none(), last);
            let label = texts(&rig.app, headless::BOUNDS);
            let expected = if last { Msg::TipDone } else { Msg::TipNext };
            assert!(label.iter().any(|(text, _)| text == t(Language::En, expected)), "step {index}: {expected:?}");
            send(&mut rig.app, Message::TipNext);
        }
        assert_eq!(rig.app.page, Page::Updates, "Done stays on the page");
        assert_eq!(rig.app.tip, None);
        assert!(Area::ALL.iter().all(|area| !settings::load().onboarding.tip_due(*area)));

        // A tip seen earlier is skipped.
        let mut rig = toured("tipskip");
        send(&mut rig.app, Message::Navigate(Page::System));
        send(&mut rig.app, Message::TipNext);
        assert_eq!(rig.app.page, Page::Updates);
        send(&mut rig.app, Message::TipNext);
        assert_eq!(rig.app.page, Page::Iso);
        send(&mut rig.app, Message::Navigate(Page::Tweaks));
        send(&mut rig.app, Message::TipNext);
        assert_eq!(rig.app.page, Page::Iso, "System and Updates were seen, ISO was only opened");
    }

    #[test]
    fn settings_bring_the_questions_and_the_tips_back() {
        let mut rig = toured("tipreset");
        send(&mut rig.app, Message::Navigate(Page::Tweaks));
        send(&mut rig.app, Message::TipNever);
        send(&mut rig.app, Message::Navigate(Page::Settings));
        send(&mut rig.app, Message::ShowTipsAgain);
        assert!(rig.app.notice.is_some());
        assert!(settings::load().onboarding.tip_due(Area::Tweaks));
        send(&mut rig.app, Message::Navigate(Page::Tweaks));
        assert_eq!(rig.app.tip, Some(Area::Tweaks));

        send(&mut rig.app, Message::Navigate(Page::Settings));
        send(&mut rig.app, Message::ShowOnboardingAgain);
        assert_eq!(rig.app.wizard.map(|wizard| wizard.step), Some(0));
        send(&mut rig.app, Message::Navigate(Page::Applications));
        assert_eq!(rig.app.tip, None, "no tip while the questions are open");
        assert!(settings::load().onboarding.wants_questions(), "reopened until answered");
    }

    #[test]
    fn a_settings_file_that_cannot_be_written_still_lets_the_questions_close() {
        let mut rig = rig_with("onbreadonly", guided());
        let blocked = rig._dir.path().join("blocked");
        std::fs::write(&blocked, "").unwrap();
        settings::use_test_file(Some(blocked.join("settings.json")));
        send(&mut rig.app, Message::Onboarding(Onboard::Goal(Goal::Privacy)));
        send(&mut rig.app, Message::Onboarding(Onboard::Skip));
        assert_eq!(rig.app.wizard, None);
        assert!(rig.app.notice.as_deref().is_some_and(|notice| notice.starts_with("Failed to save settings")));
        send(&mut rig.app, Message::DismissNotice);

        send(&mut rig.app, Message::Navigate(Page::Updates));
        send(&mut rig.app, Message::TipNext);
        assert!(rig.app.notice.is_some(), "the failed save is reported");
        send(&mut rig.app, Message::Navigate(Page::Updates));
        assert_eq!(rig.app.tip, None, "the choice holds until the app closes");
    }

    /// Every text in the view and where it was laid out, at `size`.
    fn texts(app: &App, size: iced::Size) -> Vec<(String, iced::Rectangle)> {
        use iced::advanced::widget::{Operation, Tree};
        use iced::advanced::{Layout, layout};
        struct Texts(Vec<(String, iced::Rectangle)>);
        impl Operation for Texts {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }
            fn text(&mut self, _id: Option<&iced::advanced::widget::Id>, bounds: iced::Rectangle, text: &str) {
                self.0.push((text.to_owned(), bounds));
            }
        }
        let renderer = headless::renderer();
        let mut root = crate::views::view(app);
        let mut tree = Tree::new(root.as_widget());
        let node = root.as_widget_mut().layout(&mut tree, &renderer, &layout::Limits::new(iced::Size::ZERO, size));
        let mut found = Texts(Vec::new());
        root.as_widget_mut().operate(&mut tree, Layout::new(&node), &renderer, &mut found);
        found.0
    }

    /// The last text equal to `label`, which is the one on the top layer.
    fn find(texts: &[(String, iced::Rectangle)], label: &str) -> iced::Rectangle {
        texts.iter().rev().find(|(text, _)| text == label).map(|(_, bounds)| *bounds).unwrap_or_else(|| panic!("no '{label}'"))
    }

/// Laid out whole inside the window. A squeezed row gets a height of 0,
    /// so the height has to be a full line.
    fn inside(bounds: iced::Rectangle, size: iced::Size) -> bool {
        bounds.height >= 14.0
            && bounds.x >= 0.0
            && bounds.y >= 0.0
            && bounds.x + bounds.width <= size.width + 0.5
            && bounds.y + bounds.height <= size.height + 0.5
    }

    #[test]
    fn cleaner_rows_fill_the_file_count_in_every_language() {
        let mut rig = rig("cleanrows");
        send(&mut rig.app, Message::Navigate(Page::Inspector));
        send(&mut rig.app, Message::InspectorTab(InspectorTab::Cleaner));
        let target = cleaner::targets().into_iter().next().unwrap();
        for (files, noun) in [(1, Msg::FilesCountOne), (2, Msg::FilesCountFew), (5, Msg::FilesCountMany)] {
            rig.app.clean_results = vec![(target.clone(), ScanResult { bytes: 4096, files })];
            for &language in Language::ALL {
                rig.app.language = language;
                let group = i18n::clean_target(language, &target).1;
                let expected = format_2(t(language, noun), group, &files.to_string());
                let found = texts(&rig.app, headless::BOUNDS);
                assert!(found.iter().any(|(text, _)| *text == expected), "{language:?} {files}: no '{expected}'");
                let row = found.iter().find(|(text, _)| text.starts_with(group) && text.contains(&files.to_string()));
                let row = row.unwrap_or_else(|| panic!("{language:?}: no cleaner row"));
                assert!(!row.0.contains('{'), "{language:?}: {}", row.0);
            }
        }
    }

    #[test]
    fn applications_toolbar_and_categories_fit_1024_with_the_inspector_open() {
        let size = iced::Size::new(1024.0, 700.0);
        for &language in Language::ALL {
            let mut rig = rig("apptoolbar");
            rig.app.language = language;
            rig.app.catalog = Arc::new(Catalog::load_in(language.tag()).unwrap());
            send(&mut rig.app, Message::Navigate(Page::Applications));
            drop(rig.app.update(Message::Inspect(Inspected::App("7zip".to_owned()))));
            settle(&mut rig.app);
            let found = texts(&rig.app, size);
            for msg in [Msg::ShowInstalled, Msg::UpgradeAll, Msg::ClearSelection] {
                let label = t(language, msg);
                let bounds = find(&found, label);
                assert!(inside(bounds, size) && bounds.height < 28.0, "{language:?} '{label}': {bounds:?}");
            }
            // Category names come from the translated catalog. The first match
            // is the chip; the list repeats the name as a group title.
            for category in rig.app.catalog.app_categories() {
                let chip = found.iter().find(|(text, _)| *text == category).map(|(_, bounds)| *bounds).unwrap();
                assert!(inside(chip, size), "{language:?}: category '{category}' is cut off: {chip:?}");
            }
        }
    }

    #[test]
    fn switching_the_language_translates_the_catalog_and_resets_category_filters() {
        let mut rig = rig("langcatalog");
        let english = rig.app.catalog.tweak("telemetry").unwrap().name.clone();
        let category = rig.app.catalog.app_categories()[0].to_owned();
        send(&mut rig.app, Message::Category(category));
        assert_eq!(rig.app.categories.len(), 1);
        send(&mut rig.app, Message::SetLanguage(Language::Pl));
        assert_eq!(rig.app.catalog.tweak("telemetry").unwrap().name, "Wyłącz telemetrię Windows");
        assert!(rig.app.categories.is_empty(), "an English category name would filter out everything");
        send(&mut rig.app, Message::SetLanguage(Language::En));
        assert_eq!(rig.app.catalog.tweak("telemetry").unwrap().name, english);
    }

    /// 878x425 is what a 1366x768 screen at 150 % leaves, see `window_layout`.
    const SIZES: [iced::Size; 3] = [iced::Size::new(878.0, 425.0), iced::Size::new(980.0, 600.0), iced::Size::new(1280.0, 800.0)];

    #[test]
    fn every_question_screen_keeps_its_buttons_in_a_small_window_in_every_language() {
        for &lang in Language::ALL {
            let mut rig = rig_with("onbsmall", guided());
            rig.app.language = lang;
            let answers = [
                Onboard::Goal(Goal::CleanInstall),
                Onboard::KeepFiles(false),
                Onboard::BuildIso(true),
                Onboard::Depth(Depth::Everything),
            ];
            for step in 0..WIZARD_STEPS {
                for size in SIZES {
                    let texts = texts(&rig.app, size);
                    let mut labels = vec![t(lang, Msg::OnbSkip)];
                    if step > 0 {
                        labels.push(t(lang, Msg::OnbBack));
                    }
                    if step == SUMMARY_STEP {
                        labels.push(t(lang, Msg::OnbStart));
                    }
                    for label in labels {
                        let bounds = find(&texts, label);
                        assert!(inside(bounds, size), "{lang:?} step {step} at {size:?}: '{label}' at {bounds:?}");
                    }
                }
                if let Some(answer) = answers.get(step) {
                    send(&mut rig.app, Message::Onboarding(*answer));
                }
            }
        }
    }

    #[test]
    fn every_tip_fits_beside_an_open_drawer_in_a_small_window_in_every_language() {
        for &lang in Language::ALL {
            let mut rig = toured("tipsmall");
            rig.app.language = lang;
            for page in Page::ALL {
                send(&mut rig.app, Message::Navigate(page));
                send(&mut rig.app, Message::Inspect(Inspected::Tweak("telemetry".to_owned())));
                settle(&mut rig.app);
                assert_eq!(rig.app.tip, page.area());
                let drawer = crate::theme::INSPECTOR_WIDTH;
                for size in SIZES {
                    let texts = texts(&rig.app, size);
                    for label in [t(lang, Msg::TipNext), t(lang, Msg::TipNever)] {
                        let bounds = find(&texts, label);
                        assert!(inside(bounds, size), "{lang:?} {page:?} at {size:?}: '{label}' at {bounds:?}");
                        assert!(bounds.x + bounds.width <= size.width - drawer, "{lang:?} {page:?}: '{label}' under the drawer");
                        assert!(bounds.x >= crate::theme::SIDEBAR_WIDTH, "{lang:?} {page:?}: '{label}' over the sidebar");
                    }
                }
                send(&mut rig.app, Message::TipDismiss);
                send(&mut rig.app, Message::CloseInspector);
                settle(&mut rig.app);
            }
        }
    }

    #[test]
    fn a_press_outside_the_tip_closes_it_and_still_reaches_the_page() {
        use iced::mouse;
        let mut rig = toured("tipoutside");
        send(&mut rig.app, Message::Navigate(Page::Applications));
        settle(&mut rig.app);
        assert_eq!(rig.app.tip, Some(Area::Applications));
        let renderer = headless::renderer();
        let mut tree = headless::tree(&rig.app);
        let found = texts(&rig.app, headless::BOUNDS);
        let press = iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let release = iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));

        // On the tip's own text: nothing happens.
        let body = find(&found, crate::i18n::t(Language::En, Msg::TipApplications)).center();
        let (_, messages) = headless::pass(&rig.app, &mut tree, &renderer, Some((press.clone(), body)));
        assert!(messages.is_empty(), "{messages:?}");

        // On the sidebar row of another page: the tip closes and the row still works.
        let row = texts(&rig.app, headless::BOUNDS)
            .into_iter()
            .find(|(text, _)| text == Page::Updates.title_in(Language::En))
            .map(|(_, bounds)| bounds.center())
            .unwrap();
        let (_, messages) = headless::pass(&rig.app, &mut tree, &renderer, Some((press, row)));
        assert!(matches!(messages.as_slice(), [Message::TipDismiss]), "{messages:?}");
        for message in messages {
            send(&mut rig.app, message);
        }
        assert_eq!(rig.app.tip, None);
        let (_, messages) = headless::pass(&rig.app, &mut tree, &renderer, Some((release, row)));
        assert!(matches!(messages.as_slice(), [Message::Navigate(Page::Updates)]), "{messages:?}");
        assert!(settings::load().onboarding.tip_due(Area::Applications), "closing from outside saves nothing");
    }

    #[test]
    fn the_tip_and_the_questions_never_show_with_a_dialog() {
        let mut rig = toured("tipdialog");
        send(&mut rig.app, Message::Navigate(Page::Updates));
        assert!(rig.app.tip.is_some());
        rig.app.confirm = Some(Confirm::DisableUpdates);
        let found = texts(&rig.app, headless::BOUNDS);
        assert!(!found.iter().any(|(text, _)| text == t(Language::En, Msg::TipNext)), "the dialog hides the tip");
        rig.app.confirm = None;

        send(&mut rig.app, Message::ShowOnboardingAgain);
        let found = texts(&rig.app, headless::BOUNDS);
        assert!(!found.iter().any(|(text, _)| text == t(Language::En, Msg::TipNext)));
        assert!(found.iter().any(|(text, _)| text == t(Language::En, Msg::OnbSkip)));
    }

    #[test]
    fn the_settings_buttons_work_in_a_window_a_deep_link_opened() {
        let mut rig = rig_with("onblinkedsettings", StartView::default());
        send(&mut rig.app, Message::Navigate(Page::Settings));
        send(&mut rig.app, Message::ShowOnboardingAgain);
        assert_eq!(rig.app.wizard.map(|wizard| wizard.step), Some(0));
        send(&mut rig.app, Message::Onboarding(Onboard::Skip));
        send(&mut rig.app, Message::Navigate(Page::System));
        assert_eq!(rig.app.tip, Some(Area::System), "tips follow once the user asked for the introduction");
    }

    fn key(app: &mut App, key: Key) {
        let event = keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
            location: keyboard::Location::Standard,
            modifiers: Modifiers::default(),
            text: None,
            repeat: false,
        };
        send(app, Message::Keyboard(event));
    }

    #[test]
    fn number_keys_pick_answers_and_enter_starts_on_the_summary() {
        let mut rig = rig_with("onbkeys", guided());
        key(&mut rig.app, Key::Named(key::Named::Enter));
        assert_eq!(rig.app.wizard.unwrap().step, 0, "Enter does nothing before the summary");
        key(&mut rig.app, Key::Character("9".into()));
        key(&mut rig.app, Key::Character("0".into()));
        key(&mut rig.app, Key::Character("x".into()));
        assert_eq!(rig.app.wizard.unwrap().step, 0, "keys without an answer do nothing");
        for digit in ["3", "2", "1", "2"] {
            key(&mut rig.app, Key::Character(digit.into()));
        }
        let wizard = rig.app.wizard.unwrap();
        assert_eq!(wizard.step, SUMMARY_STEP);
        assert_eq!(
            wizard.answers,
            Answers { goal: Some(Goal::Gaming), keep_files: Some(false), build_iso: Some(true), depth: Some(Depth::Standard) }
        );
        key(&mut rig.app, Key::Character("1".into()));
        assert_eq!(rig.app.wizard.unwrap().step, SUMMARY_STEP, "numbers do nothing on the summary");
        key(&mut rig.app, Key::Named(key::Named::Enter));
        assert_eq!(rig.app.wizard, None);
        assert_eq!(rig.app.page, Page::Tweaks);
        assert_eq!(settings::load().onboarding.status, windowcraft_config::onboarding::Status::Finished);

        // Without the questions the same keys change nothing.
        let selected = rig.app.selected_tweaks.len();
        key(&mut rig.app, Key::Character("2".into()));
        key(&mut rig.app, Key::Named(key::Named::Enter));
        assert_eq!((rig.app.page, rig.app.selected_tweaks.len()), (Page::Tweaks, selected));
    }

    /// Draws the view the way the window would with tiny-skia and saves it as a PNG.
    fn render(app: &App, size: iced::Size, path: &std::path::Path) {
        use iced::advanced::renderer::{Headless as _, Style};
        use iced::advanced::widget::Tree;
        use iced::advanced::{Layout, layout};
        let mut renderer = headless::renderer();
        let mut root = crate::views::view(app);
        let mut tree = Tree::new(root.as_widget());
        let node = root.as_widget_mut().layout(&mut tree, &renderer, &layout::Limits::new(iced::Size::ZERO, size));
        // Past the page entrance, so the body is drawn where it rests.
        std::thread::sleep(std::time::Duration::from_millis(260));
        let palette = app.palette();
        let theme = if palette.dark { iced::Theme::Dark } else { iced::Theme::Light };
        root.as_widget().draw(
            &tree,
            &mut renderer,
            &theme,
            &Style { text_color: palette.ink },
            Layout::new(&node),
            iced::mouse::Cursor::Unavailable,
            &iced::Rectangle::with_size(size),
        );
        let (width, height) = (size.width as u32, size.height as u32);
        let pixels = renderer.screenshot(iced::Size::new(width, height), 1.0, palette.base);
        image::save_buffer(path, &pixels, width, height, image::ColorType::Rgba8).unwrap();
    }

    /// Writes the review images in docs/screenshots/onboarding. Not part of
    /// the normal run: `cargo test -p windowcraft-app render_guide_screens -- --ignored`.
    #[test]
    #[ignore = "writes PNG files into docs/screenshots/onboarding"]
    fn render_guide_screens() {
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots/onboarding");
        std::fs::create_dir_all(&out).unwrap();
        let large = iced::Size::new(1280.0, 800.0);
        let small = iced::Size::new(878.0, 425.0);
        let answers = [Onboard::Goal(Goal::Gaming), Onboard::KeepFiles(false), Onboard::BuildIso(true), Onboard::Depth(Depth::Standard)];
        let names = ["1-goal", "2-files", "3-iso", "4-depth", "5-summary"];
        for (lang, suffix) in [(Language::En, "en"), (Language::Pl, "pl")] {
            let mut rig = rig_with("render", guided());
            rig.app.language = lang;
            rig.app.appearance = Appearance::Dark;
            for (step, name) in names.iter().enumerate() {
                render(&rig.app, large, &out.join(format!("wizard-{name}-{suffix}.png")));
                if let Some(answer) = answers.get(step) {
                    send(&mut rig.app, Message::Onboarding(*answer));
                }
            }
            render(&rig.app, small, &out.join(format!("wizard-5-summary-878x425-{suffix}.png")));
            send(&mut rig.app, Message::Onboarding(Onboard::Start));
            settle(&mut rig.app);
            render(&rig.app, large, &out.join(format!("after-start-tweaks-tip-{suffix}.png")));
            send(&mut rig.app, Message::TipDismiss);
            for (page, name) in Page::ALL.iter().zip(["applications", "tweaks", "system", "updates", "iso", "time-machine", "inspector", "extensions"]) {
                send(&mut rig.app, Message::Navigate(*page));
                settle(&mut rig.app);
                assert!(rig.app.tip.is_some());
                render(&rig.app, large, &out.join(format!("tip-{name}-{suffix}.png")));
                send(&mut rig.app, Message::TipDismiss);
            }
            send(&mut rig.app, Message::Navigate(Page::Settings));
            settle(&mut rig.app);
            render(&rig.app, large, &out.join(format!("settings-guide-{suffix}.png")));
        }
    }
}
