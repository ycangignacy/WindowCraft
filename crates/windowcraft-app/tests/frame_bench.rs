// Made by ycangignacy
//! Headless per-frame CPU cost of the desktop interface.
//!
//! Run in release with
//! `cargo test --release -p windowcraft-app --test frame_bench -- --ignored --nocapture`.
//!
//! `iced` does not re-export `iced_runtime`, so `Ui` below repeats what
//! `iced_runtime::UserInterface` does for the base layer: diff the cached
//! widget tree, lay it out against the window bounds, route events and draw.
//! Overlays are asked for before every event, as the runtime does, but never
//! laid out or drawn; the bench reports whether any page produced one.
//!
//! Rasterization goes through `Headless::screenshot`. It repaints the whole
//! surface on every call, so the damage tracking of the windowed compositor is
//! bypassed, and it also allocates a fresh buffer and converts it to RGBA. The
//! cost of that copy is measured on its own and printed next to the tables.
//!
//! Animation time comes from a virtual clock that advances 16 ms per frame, so
//! the frame counts are deterministic. Only the wall-clock timings vary.

use iced::advanced::clipboard;
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Headless};
use iced::advanced::widget::Tree;
use iced::advanced::{Renderer as _, Shell};
use iced::{Color, Element, Event, Point, Rectangle, Size, Theme, Vector, mouse, window};
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};
use windowcraft_app::app::{App, DRAWER, Inspected, Message, Page, StartView};
use windowcraft_app::theme::{INSPECTOR_WIDTH, SANS, SIDEBAR_WIDTH};
use windowcraft_app::views;
use windowcraft_config::Catalog;
use windowcraft_engine::executor::RecordingRunner;
use windowcraft_engine::services::MemoryServices;
use windowcraft_engine::sysinfo::SystemInfo;
use windowcraft_engine::testing::Sandbox;
use windowcraft_engine::{Engine, SnapshotStore};
use windowcraft_physics::MotionMode;

type Renderer = iced::Renderer;

const WINDOW: Size = Size::new(1280.0, 800.0);
const SCALES: [f32; 2] = [1.0, 1.5];
const FRAME: Duration = Duration::from_millis(16);
const WARMUP: usize = 5;
const ITERATIONS: usize = 60;
const SCROLL_STEP: f32 = 8.0;
/// The default text size of `iced::Settings`, which the app keeps.
const TEXT_SIZE: f32 = 16.0;
const SETTLE_LIMIT: usize = 600;

// ---------------------------------------------------------------------------
// App rig, the same construction as `rig` in `app::tests`.

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
    let (mut app, startup) = App::new(catalog, engine, info, StartView::default());
    drop(startup);
    // Windows may report animations as off; the bench measures the spring path.
    app.timeline.mode = MotionMode::Spring;
    Rig { app, _sandbox: sandbox, _dir: dir }
}

/// Frame timestamps for `Message::Frame`. It starts after `App::new`, so every
/// instant lies after the app's private epoch.
struct Clock(Instant);

impl Clock {
    fn new() -> Self {
        Self(Instant::now())
    }

    fn tick(&mut self) -> Instant {
        self.0 += FRAME;
        self.0
    }

    fn now(&self) -> Instant {
        self.0
    }
}

fn send(app: &mut App, message: Message) {
    drop(app.update(message));
}

/// Sends frames until every channel rests and returns how many it took.
fn settle(app: &mut App, clock: &mut Clock) -> usize {
    let mut frames = 0;
    while app.timeline.needs_frame() {
        frames += 1;
        assert!(frames < SETTLE_LIMIT, "the animation loop never stopped");
        send(app, Message::Frame(clock.tick()));
    }
    frames
}

// ---------------------------------------------------------------------------
// The base layer of `iced_runtime::UserInterface`.

struct Ui<'a> {
    root: Element<'a, Message>,
    base: layout::Node,
    tree: Tree,
    bounds: Size,
}

struct BuildTimes {
    diff: Duration,
    layout: Duration,
}

#[derive(Default)]
struct UpdateOutcome {
    redraw: bool,
    relayout: bool,
    outdated: bool,
    overlay: bool,
}

impl<'a> Ui<'a> {
    fn build(mut root: Element<'a, Message>, bounds: Size, mut tree: Tree, renderer: &Renderer) -> (Self, BuildTimes) {
        let start = Instant::now();
        tree.diff(root.as_widget());
        let diffed = Instant::now();
        let base = root.as_widget_mut().layout(&mut tree, renderer, &layout::Limits::new(Size::ZERO, bounds));
        let laid_out = Instant::now();
        (Self { root, base, tree, bounds }, BuildTimes { diff: diffed - start, layout: laid_out - diffed })
    }

    fn update(
        &mut self,
        event: &Event,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        messages: &mut Vec<Message>,
    ) -> UpdateOutcome {
        let viewport = Rectangle::with_size(self.bounds);
        let overlay = self
            .root
            .as_widget_mut()
            .overlay(&mut self.tree, Layout::new(&self.base), renderer, &viewport, Vector::ZERO)
            .is_some();
        let mut shell = Shell::new(messages);
        let mut clipboard = clipboard::Null;
        self.root.as_widget_mut().update(
            &mut self.tree,
            event,
            Layout::new(&self.base),
            cursor,
            renderer,
            &mut clipboard,
            &mut shell,
            &viewport,
        );
        let redraw = shell.redraw_request() != window::RedrawRequest::Wait;
        let relayout = shell.is_layout_invalid();
        let outdated = shell.are_widgets_invalid();
        drop(shell);
        if relayout {
            self.base =
                self.root.as_widget_mut().layout(&mut self.tree, renderer, &layout::Limits::new(Size::ZERO, self.bounds));
        }
        UpdateOutcome { redraw, relayout, outdated, overlay }
    }

    fn draw(&self, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, cursor: mouse::Cursor) {
        let viewport = Rectangle::with_size(self.bounds);
        renderer.reset(viewport);
        self.root.as_widget().draw(&self.tree, renderer, theme, style, Layout::new(&self.base), cursor, &viewport);
    }

    fn into_tree(self) -> Tree {
        self.tree
    }
}

fn count_nodes(tree: &Tree) -> usize {
    1 + tree.children.iter().map(count_nodes).sum::<usize>()
}

// ---------------------------------------------------------------------------
// Renderer and paint state.

struct Surface {
    renderer: Renderer,
    scale: f32,
    physical: Size<u32>,
}

impl Surface {
    fn new(scale: f32) -> Self {
        // `Headless::new` for tiny-skia only checks the backend name and then
        // calls this constructor, so the async wrapper is skipped.
        let renderer = Renderer::new(SANS, iced::Pixels(TEXT_SIZE));
        let physical =
            Size::new((WINDOW.width * scale).round() as u32, (WINDOW.height * scale).round() as u32);
        Self { renderer, scale, physical }
    }

    fn raster(&mut self, background: Color) -> Duration {
        let start = Instant::now();
        let pixels = self.renderer.screenshot(self.physical, self.scale, background);
        let elapsed = start.elapsed();
        black_box(pixels);
        elapsed
    }
}

/// What `App::view` callers in `lib.rs` pass to the renderer.
struct Paint {
    theme: Theme,
    style: renderer::Style,
    background: Color,
}

fn paint(app: &App) -> Paint {
    let palette = app.palette();
    Paint {
        theme: if palette.dark { Theme::Dark } else { Theme::Light },
        style: renderer::Style { text_color: palette.ink },
        // The opaque path; with Mica the background is transparent instead.
        background: palette.base,
    }
}

/// The part of `screenshot` that a windowed present does not do: a fresh
/// buffer plus the per-pixel conversion to RGBA bytes.
fn copy_overhead(physical: Size<u32>) -> Series {
    let mut series = Series::default();
    let pixels = physical.width as usize * physical.height as usize;
    for i in 0..WARMUP + 20 {
        let start = Instant::now();
        let buffer: Vec<u32> = black_box(vec![0; pixels]);
        let bytes = buffer.iter().fold(Vec::with_capacity(pixels * 4), |mut acc: Vec<u8>, pixel| {
            acc.extend([(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8, (pixel >> 24) as u8]);
            acc
        });
        black_box(bytes);
        if i >= WARMUP {
            series.push(start.elapsed());
        }
    }
    series
}

// ---------------------------------------------------------------------------
// Statistics.

#[derive(Default)]
struct Series(Vec<f64>);

impl Series {
    fn push(&mut self, duration: Duration) {
        self.0.push(duration.as_secs_f64() * 1000.0);
    }

    fn sorted(&self) -> Vec<f64> {
        let mut values = self.0.clone();
        values.sort_by(f64::total_cmp);
        values
    }

    /// Nearest-rank percentile.
    fn percentile(&self, q: f64) -> f64 {
        let values = self.sorted();
        if values.is_empty() {
            return f64::NAN;
        }
        let rank = (q * values.len() as f64).ceil() as usize;
        values[rank.clamp(1, values.len()) - 1]
    }

    fn median(&self) -> f64 {
        self.percentile(0.5)
    }

    fn p95(&self) -> f64 {
        self.percentile(0.95)
    }

    fn cell(&self) -> String {
        format!("{:>6.2} {:>6.2}", self.median(), self.p95())
    }
}

// ---------------------------------------------------------------------------
// Static page frames and wheel scrolling.

#[derive(Default)]
struct PageRow {
    view: Series,
    diff: Series,
    layout: Series,
    record: Series,
    raster: Series,
    full: Series,
    scroll_events: Series,
    scroll_record: Series,
    scroll_raster: Series,
    scroll_total: Series,
    nodes: usize,
    settle_frames: usize,
    moved: usize,
    reversals: usize,
    scroll_messages: usize,
    scroll_relayouts: usize,
    overlay: bool,
}

/// Center of the content column, right of the sidebar.
fn content_center() -> Point {
    Point::new(SIDEBAR_WIDTH + (WINDOW.width - SIDEBAR_WIDTH) / 2.0, WINDOW.height / 2.0)
}

/// The redraw event carries the time of the frame it paints.
fn redraw_event(clock: &Clock) -> Event {
    Event::Window(window::Event::RedrawRequested(clock.now()))
}

fn bench_page(app: &mut App, page: Page, surface: &mut Surface, clock: &mut Clock) -> PageRow {
    let mut row = PageRow::default();
    send(app, Message::Navigate(page));
    row.settle_frames = settle(app, clock);
    let paint = paint(app);
    let cursor = mouse::Cursor::Available(content_center());

    // (a) to (c): a message arrived, so the runtime rebuilds and redraws.
    let mut tree = Tree::empty();
    for i in 0..WARMUP + ITERATIONS {
        let start = Instant::now();
        let root = views::view(app);
        let view = start.elapsed();
        let (ui, build) = Ui::build(root, WINDOW, tree, &surface.renderer);
        let start = Instant::now();
        ui.draw(&mut surface.renderer, &paint.theme, &paint.style, cursor);
        let record = start.elapsed();
        let raster = surface.raster(paint.background);
        tree = ui.into_tree();
        if i >= WARMUP {
            row.view.push(view);
            row.diff.push(build.diff);
            row.layout.push(build.layout);
            row.record.push(record);
            row.raster.push(raster);
            row.full.push(view + build.diff + build.layout + record + raster);
        }
    }
    row.nodes = count_nodes(&tree);

    // (d): one wheel notch of 8 px, then the redraw it asks for. The runtime
    // routes `RedrawRequested` through `update` before every draw.
    let mut messages = Vec::new();
    let (mut ui, _) = Ui::build(views::view(app), WINDOW, tree, &surface.renderer);
    let mut dy = -SCROLL_STEP;
    for i in 0..WARMUP + ITERATIONS {
        let start = Instant::now();
        let wheel = Event::Mouse(mouse::Event::WheelScrolled { delta: mouse::ScrollDelta::Pixels { x: 0.0, y: dy } });
        let wheel_outcome = ui.update(&wheel, cursor, &surface.renderer, &mut messages);
        clock.tick();
        let redraw_outcome = ui.update(&redraw_event(clock), cursor, &surface.renderer, &mut messages);
        let mut events = start.elapsed();
        let mut relayouts = usize::from(wheel_outcome.relayout) + usize::from(redraw_outcome.relayout);
        row.overlay |= wheel_outcome.overlay || redraw_outcome.overlay;

        // Messages or invalid widgets make the runtime call update and view
        // again, then rebuild against the cached tree.
        if !messages.is_empty() || wheel_outcome.outdated || redraw_outcome.outdated {
            let start = Instant::now();
            let tree = ui.into_tree();
            let produced = messages.len();
            for message in messages.drain(..) {
                send(app, message);
            }
            let (rebuilt, _) = Ui::build(views::view(app), WINDOW, tree, &surface.renderer);
            ui = rebuilt;
            events += start.elapsed();
            relayouts += 1;
            if i >= WARMUP {
                row.scroll_messages += produced;
            }
        }

        let start = Instant::now();
        ui.draw(&mut surface.renderer, &paint.theme, &paint.style, cursor);
        let record = start.elapsed();
        let raster = surface.raster(paint.background);

        // A wheel event that changes nothing requests no redraw: the content
        // hit an end or does not scroll at all. Turn around to keep moving.
        if !wheel_outcome.redraw {
            dy = -dy;
            if i >= WARMUP {
                row.reversals += 1;
            }
        }
        if i >= WARMUP {
            row.moved += usize::from(wheel_outcome.redraw);
            row.scroll_relayouts += relayouts;
            row.scroll_events.push(events);
            row.scroll_record.push(record);
            row.scroll_raster.push(raster);
            row.scroll_total.push(events + record + raster);
        }
    }
    drop(ui);
    row
}

// ---------------------------------------------------------------------------
// The inspector drawer opening on Applications.

#[derive(Default)]
struct DrawerRun {
    message_frame: Series,
    update: Series,
    view: Series,
    diff: Series,
    layout: Series,
    events: Series,
    record: Series,
    raster: Series,
    total: Series,
    widths: Vec<f64>,
    relayouts: usize,
}

/// One full frame: update with `message`, view, build, redraw event, draw.
/// Hands the widget tree back for the next frame.
fn drawer_frame(
    app: &mut App,
    message: Message,
    tree: Tree,
    surface: &mut Surface,
    clock: &mut Clock,
    run: &mut DrawerRun,
    record_series: bool,
) -> Tree {
    let cursor = mouse::Cursor::Available(content_center());
    let start = Instant::now();
    send(app, message);
    let update = start.elapsed();
    let paint = paint(app);
    let start = Instant::now();
    let root = views::view(app);
    let view = start.elapsed();
    let (mut ui, build) = Ui::build(root, WINDOW, tree, &surface.renderer);
    let mut messages = Vec::new();
    let start = Instant::now();
    let outcome = ui.update(&redraw_event(clock), cursor, &surface.renderer, &mut messages);
    let events = start.elapsed();
    let start = Instant::now();
    ui.draw(&mut surface.renderer, &paint.theme, &paint.style, cursor);
    let record = start.elapsed();
    let raster = surface.raster(paint.background);
    let total = update + view + build.diff + build.layout + events + record + raster;
    run.relayouts += usize::from(outcome.relayout);
    if record_series {
        run.update.push(update);
        run.view.push(view);
        run.diff.push(build.diff);
        run.layout.push(build.layout);
        run.events.push(events);
        run.record.push(record);
        run.raster.push(raster);
        run.total.push(total);
    } else {
        run.message_frame.push(total);
    }
    // Dropped unhandled: a redraw event on a static frame produces none.
    drop(messages);
    ui.into_tree()
}

fn bench_drawer(app: &mut App, surface: &mut Surface, clock: &mut Clock) -> DrawerRun {
    let mut run = DrawerRun::default();
    send(app, Message::Navigate(Page::Applications));
    settle(app, clock);

    // Warm the tree and the glyph cache with the closed state.
    let paint = paint(app);
    let mut tree = Tree::empty();
    for _ in 0..WARMUP {
        let (ui, _) = Ui::build(views::view(app), WINDOW, tree, &surface.renderer);
        ui.draw(&mut surface.renderer, &paint.theme, &paint.style, mouse::Cursor::Unavailable);
        surface.raster(paint.background);
        tree = ui.into_tree();
    }

    let id = app.catalog.apps.first().expect("the catalog lists applications").id.clone();
    tree = drawer_frame(app, Message::Inspect(Inspected::App(id)), tree, surface, clock, &mut run, false);
    while app.timeline.needs_frame() {
        assert!(run.widths.len() < SETTLE_LIMIT, "the drawer never settled");
        tree = drawer_frame(app, Message::Frame(clock.tick()), tree, surface, clock, &mut run, true);
        run.widths.push(app.timeline.channels[DRAWER].value());
    }
    drop(tree);
    run
}

/// First frame (1-based) from which the drawer stays within `tolerance` of
/// its target.
fn frames_until_close(widths: &[f64], target: f64, tolerance: f64) -> Option<usize> {
    let last_far = widths.iter().rposition(|width| (width - target).abs() > tolerance);
    match last_far {
        None => (!widths.is_empty()).then_some(1),
        Some(index) if index + 1 < widths.len() => Some(index + 2),
        Some(_) => None,
    }
}

// ---------------------------------------------------------------------------
// Report.

fn page_name(page: Page) -> &'static str {
    match page {
        Page::Applications => "Applications",
        Page::Tweaks => "Tweaks",
        Page::System => "System",
        Page::Updates => "Updates",
        Page::Iso => "Iso",
        Page::TimeMachine => "TimeMachine",
        Page::Inspector => "Inspector",
    }
}

fn print_pages(rows: &[(Page, PageRow)]) {
    println!("Rebuild frame: view + build (diff, layout) + draw (record, raster), ms as median p95");
    println!(
        "{:<13} {:>5} {:>13} {:>13} {:>13} {:>13} {:>13} {:>13}",
        "page", "nodes", "view", "diff", "layout", "record", "raster", "sum"
    );
    for (page, row) in rows {
        println!(
            "{:<13} {:>5} {:>13} {:>13} {:>13} {:>13} {:>13} {:>13}",
            page_name(*page),
            row.nodes,
            row.view.cell(),
            row.diff.cell(),
            row.layout.cell(),
            row.record.cell(),
            row.raster.cell(),
            row.full.cell(),
        );
    }
    println!();
    println!("Wheel scroll, Pixels {{ y: -8 }} at the content center, then RedrawRequested and draw, ms as median p95");
    println!(
        "{:<13} {:>13} {:>13} {:>13} {:>13} {:>6} {:>5} {:>5} {:>8} {:>7}",
        "page", "events", "record", "raster", "sum", "moved", "turns", "msgs", "relayout", "overlay"
    );
    for (page, row) in rows {
        println!(
            "{:<13} {:>13} {:>13} {:>13} {:>13} {:>6} {:>5} {:>5} {:>8} {:>7}",
            page_name(*page),
            row.scroll_events.cell(),
            row.scroll_record.cell(),
            row.scroll_raster.cell(),
            row.scroll_total.cell(),
            format!("{}/{}", row.moved, ITERATIONS),
            row.reversals,
            row.scroll_messages,
            row.scroll_relayouts,
            row.overlay,
        );
    }
    println!();
    let settles: Vec<String> =
        rows.iter().map(|(page, row)| format!("{} {}", page_name(*page), row.settle_frames)).collect();
    println!("Frames to settle after Navigate: {}", settles.join(", "));
}

fn print_drawer(run: &DrawerRun) {
    let target = f64::from(INSPECTOR_WIDTH);
    let frames = run.widths.len();
    println!("Inspector drawer opening on Applications, per Frame message, ms as median p95");
    println!(
        "{:>13} {:>13} {:>13} {:>13} {:>13} {:>13} {:>13} {:>13}",
        "update", "view", "diff", "layout", "redraw ev", "record", "raster", "frame"
    );
    println!(
        "{:>13} {:>13} {:>13} {:>13} {:>13} {:>13} {:>13} {:>13}",
        run.update.cell(),
        run.view.cell(),
        run.diff.cell(),
        run.layout.cell(),
        run.events.cell(),
        run.record.cell(),
        run.raster.cell(),
        run.total.cell(),
    );
    println!("Frame for the Inspect message itself: {:.2} ms", run.message_frame.median());
    println!("Frames until the timeline settles: {frames}");
    let close = frames_until_close(&run.widths, target, 0.5);
    match close {
        Some(frame) => println!(
            "Drawer within 0.5 px of {target} from frame {frame}; {} of {frames} frames move it less than that",
            frames - frame
        ),
        None => println!("Drawer never came within 0.5 px of {target}"),
    }
    let first_within =
        run.widths.iter().position(|width| (width - target).abs() <= 0.5).map_or(0, |index| index + 1);
    println!("First frame within 0.5 px: {first_within}");
    let stuck = run.widths.iter().take_while(|width| **width == 0.0).count();
    println!("Leading frames with the drawer still at 0: {stuck}");
    let widths: Vec<String> = run.widths.iter().map(|width| format!("{width:.1}")).collect();
    println!("Drawer width per frame: {}", widths.join(" "));
    println!("Relayouts requested by the redraw event: {}", run.relayouts);
}

#[test]
#[ignore = "benchmark, run in release with --ignored --nocapture"]
fn frame_bench() {
    println!();
    println!(
        "frame_bench: window {}x{}, {WARMUP} warm-up + {ITERATIONS} measured iterations, debug_assertions={}",
        WINDOW.width,
        WINDOW.height,
        cfg!(debug_assertions)
    );
    for scale in SCALES {
        let mut page_rig = rig(&format!("framebench{}", (scale * 10.0) as u32));
        let mut clock = Clock::new();
        let mut surface = Surface::new(scale);
        println!();
        println!(
            "=== scale {scale}, physical {}x{}, dark={} ===",
            surface.physical.width,
            surface.physical.height,
            page_rig.app.palette().dark
        );
        let copy = copy_overhead(surface.physical);
        println!(
            "screenshot buffer + RGBA copy alone: {:.2} ms median, {:.2} ms p95 (included in raster)",
            copy.median(),
            copy.p95()
        );
        println!();
        let rows: Vec<(Page, PageRow)> =
            Page::ALL.iter().map(|page| (*page, bench_page(&mut page_rig.app, *page, &mut surface, &mut clock))).collect();
        print_pages(&rows);
        println!();

        let mut drawer_rig = rig(&format!("framebenchdrawer{}", (scale * 10.0) as u32));
        // A fresh clock, so no frame lies before the new app's epoch.
        let mut drawer_clock = Clock::new();
        let mut drawer_surface = Surface::new(scale);
        let run = bench_drawer(&mut drawer_rig.app, &mut drawer_surface, &mut drawer_clock);
        print_drawer(&run);
    }
}
