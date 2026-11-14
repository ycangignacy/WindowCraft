// Made by Nordan
//! Eased mouse-wheel scrolling for any scrollable inside it.
//!
//! Iced moves a scrollable by a fixed 60 px on every wheel notch, so a list
//! jumps in steps. This wrapper takes line-based wheel events before the
//! scrollable sees them, adds them to a pending distance and hands that
//! distance out over the next display frames as pixel deltas, carried by a
//! critically damped spring. Pixel deltas pass through untouched; on Windows
//! winit reports touchpads as fractional lines, so they are eased as well.
//! The scrollable keeps its fractional offset and rounds the drawn
//! translation, so text never lands between pixels.

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard::{self, Modifiers};
use iced::mouse::{self, Cursor, ScrollDelta};
use iced::{Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, window};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Instant;

/// Response of the critically damped spring that carries the list, in
/// seconds. About 95% of a notch arrives within 0.15 s.
const RESPONSE_SECONDS: f32 = 0.2;
/// A notch that starts from rest also gets this share of the spring's top
/// speed, so the first frame already moves instead of easing in.
const START_KICK: f32 = 0.5;
/// Below these the rest is handed out at once, so the tail never crawls.
const FINISH_PIXELS: f32 = 0.5;
const FINISH_SPEED: f32 = 6.0;
/// Frame gaps longer than this are treated as one frame, so a stalled window
/// does not swallow the whole distance in one jump.
const MAX_STEP_SECONDS: f32 = 1.0 / 24.0;
/// A notch keeps stacking only up to this many pixels ahead of the screen.
const MAX_PENDING_PIXELS: f32 = 2400.0;

static ENABLED: AtomicBool = AtomicBool::new(true);
static LINE_PIXELS: AtomicU32 = AtomicU32::new(20);
static LINES_PER_NOTCH: AtomicU32 = AtomicU32::new(3);

/// Turns easing on or off, for example when Windows animations are off.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether Windows animations are on, for widgets that animate themselves.
pub fn motion_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Lines moved per wheel notch, from the Windows mouse setting.
pub fn set_lines_per_notch(lines: u32) {
    LINES_PER_NOTCH.store(lines.clamp(1, 30), Ordering::Relaxed);
}

fn notch_pixels() -> f32 {
    (LINE_PIXELS.load(Ordering::Relaxed) * LINES_PER_NOTCH.load(Ordering::Relaxed)) as f32
}

pub struct Smooth<'a, Message> {
    content: Element<'a, Message>,
}

/// One axis of the motion: how far the list still has to go and how fast
/// it moves. A new notch moves the target and keeps the speed, so a fast
/// spin of the wheel is one continuous motion instead of a pulse per notch.
#[derive(Debug, Default, Clone, Copy)]
struct Axis {
    remaining: f32,
    velocity: f32,
}

impl Axis {
    fn is_moving(self) -> bool {
        self.remaining != 0.0 || self.velocity != 0.0
    }

    fn push(&mut self, delta: f32, omega: f32) {
        if delta == 0.0 {
            return;
        }
        // Reversing drops what is left, so the turn shows on the next frame.
        if self.remaining * delta < 0.0 || self.velocity * delta < 0.0 {
            *self = Self::default();
        }
        if !self.is_moving() {
            self.velocity = delta * omega * START_KICK;
        }
        self.remaining = (self.remaining + delta).clamp(-MAX_PENDING_PIXELS, MAX_PENDING_PIXELS);
    }

    /// Advances the critically damped spring by `dt` and returns the pixels
    /// covered in that time.
    fn step(&mut self, dt: f32, omega: f32) -> f32 {
        // The spring works on the displacement from the target, which is
        // minus what remains.
        let x0 = -self.remaining;
        let decay = (-omega * dt).exp();
        let b = self.velocity + omega * x0;
        let x = decay * (x0 + b * dt);
        let velocity = decay * (self.velocity - omega * b * dt);
        if x.abs() < FINISH_PIXELS && velocity.abs() < FINISH_SPEED {
            let rest = self.remaining;
            *self = Self::default();
            return rest;
        }
        let moved = x - x0;
        self.remaining -= moved;
        self.velocity = velocity;
        moved
    }
}

#[derive(Debug, Default)]
struct State {
    x: Axis,
    y: Axis,
    cursor: Option<Point>,
    last_frame: Option<Instant>,
    modifiers: Modifiers,
}

fn omega() -> f32 {
    core::f32::consts::TAU / RESPONSE_SECONDS
}

impl State {
    fn is_moving(&self) -> bool {
        self.x.is_moving() || self.y.is_moving()
    }

    #[cfg(test)]
    fn remaining(&self) -> Vector {
        Vector::new(self.x.remaining, self.y.remaining)
    }

    fn stop(&mut self) {
        self.x = Axis::default();
        self.y = Axis::default();
        self.last_frame = None;
    }

    /// Adds a notch to the motion.
    fn push(&mut self, delta: Vector) {
        self.x.push(delta.x, omega());
        self.y.push(delta.y, omega());
    }

    /// Takes the distance covered since the last frame.
    fn take(&mut self, now: Instant) -> Vector {
        let dt = self
            .last_frame
            .map_or(1.0 / 120.0, |last| now.saturating_duration_since(last).as_secs_f32())
            .clamp(0.0, MAX_STEP_SECONDS);
        self.last_frame = Some(now);
        let step = Vector::new(self.x.step(dt, omega()), self.y.step(dt, omega()));
        trace::frame(now, dt, step.y, self.y.remaining, self.y.velocity);
        if !self.is_moving() {
            self.last_frame = None;
        }
        step
    }
}

/// Per-frame log of the wheel motion, for measuring how smooth it is on a
/// given machine. Off unless `WINDOWCRAFT_SCROLL_TRACE` names a file.
mod trace {
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    static FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    static EPOCH: OnceLock<Instant> = OnceLock::new();

    pub fn frame(now: Instant, dt: f32, step: f32, remaining: f32, velocity: f32) {
        let file = FILE.get_or_init(|| {
            let path = std::env::var_os("WINDOWCRAFT_SCROLL_TRACE")?;
            std::fs::File::create(path).ok().map(Mutex::new)
        });
        if let Some(file) = file
            && let Ok(mut file) = file.lock()
        {
            let at = now.saturating_duration_since(*EPOCH.get_or_init(|| now)).as_secs_f64() * 1000.0;
            let _ = writeln!(file, "{at:.2} {:.2} {step:.2} {remaining:.2} {velocity:.1}", dt * 1000.0);
        }
    }
}

pub fn smooth<'a, Message>(content: impl Into<Element<'a, Message>>) -> Smooth<'a, Message> {
    Smooth { content: content.into() }
}

impl<Message> Widget<Message, Theme, Renderer> for Smooth<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => state.modifiers = *modifiers,
            Event::Mouse(mouse::Event::WheelScrolled { delta: ScrollDelta::Lines { x, y } })
                if ENABLED.load(Ordering::Relaxed) && !shell.is_event_captured() =>
            {
                if let Some(position) = cursor.position_over(layout.bounds()) {
                    // Shift turns a vertical wheel sideways, as the scrollable does for lines.
                    let (x, y) = if state.modifiers.shift() { (*y, *x) } else { (*x, *y) };
                    state.push(Vector::new(x, y) * notch_pixels());
                    state.cursor = Some(position);
                    shell.capture_event();
                    shell.request_redraw();
                    return;
                }
            }
            // A grab of the scrollbar or the content owns the position from here.
            Event::Mouse(mouse::Event::ButtonPressed(_)) | Event::Window(window::Event::Unfocused) => state.stop(),
            Event::Window(window::Event::RedrawRequested(now)) if state.is_moving() => {
                let step = state.take(*now);
                let at = state.cursor.unwrap_or_else(|| layout.bounds().center());
                let wheel = Event::Mouse(mouse::Event::WheelScrolled { delta: ScrollDelta::Pixels { x: step.x, y: step.y } });
                self.content.as_widget_mut().update(
                    &mut tree.children[0],
                    &wheel,
                    layout,
                    Cursor::Available(at),
                    renderer,
                    clipboard,
                    shell,
                    viewport,
                );
                let state = tree.state.downcast_mut::<State>();
                if state.is_moving() {
                    shell.request_redraw_at(crate::vsync::next_redraw(*now));
                }
            }
            _ => {}
        }
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a> From<Smooth<'a, Message>> for Element<'a, Message> {
    fn from(value: Smooth<'a, Message>) -> Self {
        Element::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn run(state: &mut State, frames: usize) -> f32 {
        let start = Instant::now();
        (0..frames).map(|frame| state.take(start + Duration::from_millis(16 * frame as u64)).y).sum()
    }

    #[test]
    fn a_notch_arrives_in_full_and_then_stops() {
        let mut state = State::default();
        state.push(Vector::new(0.0, 60.0));
        let moved = run(&mut state, 40);
        assert!((moved - 60.0).abs() < 1e-3, "moved {moved}");
        assert!(!state.is_moving());
    }

    #[test]
    fn most_of_a_notch_arrives_within_a_fifth_of_a_second() {
        let mut state = State::default();
        state.push(Vector::new(0.0, -60.0));
        let moved = run(&mut state, 12);
        assert!(moved < -55.0, "moved {moved}");
    }

    #[test]
    fn reversing_drops_the_rest_of_the_old_direction() {
        let mut state = State::default();
        state.push(Vector::new(0.0, 120.0));
        state.push(Vector::new(0.0, -60.0));
        assert_eq!(state.remaining().y, -60.0);
        assert!(state.y.velocity < 0.0, "the reversal starts moving the new way at once");
    }

    #[test]
    fn a_long_stall_is_one_frame_not_a_jump() {
        let mut state = State::default();
        state.push(Vector::new(0.0, 600.0));
        let start = Instant::now();
        state.take(start);
        let step = state.take(start + Duration::from_secs(2)).y;
        assert!(step < 600.0 * 0.6, "stalled step {step}");
    }

    /// tiny-skia behind iced's fallback renderer. Its `Headless::new` never
    /// waits, so one poll with a no-op waker finishes it.
    fn software_renderer() -> Renderer {
        use iced::advanced::renderer::Headless as _;
        use std::task::{Context, Poll, Waker};
        let future = Renderer::new(iced::Font::DEFAULT, iced::Pixels(16.0), Some("tiny-skia"));
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(Some(renderer)) => renderer,
            _ => panic!("tiny-skia must be available without waiting"),
        }
    }

    /// A 400x300 list of 200 rows behind the wrapper, driven through the
    /// same diff, layout, update calls the runtime makes.
    struct Pipeline {
        root: Element<'static, f32>,
        tree: Tree,
        node: layout::Node,
        renderer: Renderer,
        offset: f32,
        clock: Instant,
    }

    const BOUNDS: Size = Size::new(400.0, 300.0);

    impl Pipeline {
        fn new() -> Self {
            let mut rows = iced::widget::Column::new();
            for _ in 0..200 {
                rows = rows.push(iced::widget::space().width(Length::Fill).height(40));
            }
            let list = iced::widget::scrollable(rows)
                .width(Length::Fill)
                .height(Length::Fill)
                .on_scroll(|viewport| viewport.absolute_offset().y);
            let mut root: Element<'static, f32> = smooth(list).into();
            let renderer = software_renderer();
            let mut tree = Tree::new(root.as_widget());
            let node = root.as_widget_mut().layout(&mut tree, &renderer, &layout::Limits::new(Size::ZERO, BOUNDS));
            Self { root, tree, node, renderer, offset: 0.0, clock: Instant::now() }
        }

        fn send(&mut self, event: Event, at: Point) {
            let mut messages = Vec::new();
            let mut shell = Shell::new(&mut messages);
            self.root.as_widget_mut().update(
                &mut self.tree,
                &event,
                Layout::new(&self.node),
                Cursor::Available(at),
                &self.renderer,
                &mut iced::advanced::clipboard::Null,
                &mut shell,
                &Rectangle::with_size(BOUNDS),
            );
            drop(shell);
            if let Some(offset) = messages.last() {
                self.offset = *offset;
            }
        }

        fn wheel(&mut self, delta: ScrollDelta) {
            self.send(Event::Mouse(mouse::Event::WheelScrolled { delta }), Point::new(200.0, 150.0));
        }

        fn frame(&mut self) {
            self.clock += Duration::from_millis(16);
            self.send(Event::Window(window::Event::RedrawRequested(self.clock)), Point::new(200.0, 150.0));
        }
    }

    /// Easing is a process-wide switch, so tests that flip it run one at a time.
    static SWITCH: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn a_wheel_notch_eases_over_frames_to_its_full_distance() {
        let _guard = SWITCH.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        set_enabled(true);
        let mut ui = Pipeline::new();
        ui.wheel(ScrollDelta::Lines { x: 0.0, y: -1.0 });
        assert_eq!(ui.offset, 0.0, "nothing moves before the next frame");
        let mut previous = 0.0;
        for _ in 0..40 {
            ui.frame();
            assert!(ui.offset >= previous, "went back from {previous} to {}", ui.offset);
            previous = ui.offset;
        }
        assert!((ui.offset - notch_pixels()).abs() < 0.01, "ended at {}", ui.offset);
    }

    #[test]
    fn pixel_deltas_and_disabled_easing_move_at_once() {
        let _guard = SWITCH.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut ui = Pipeline::new();
        ui.wheel(ScrollDelta::Pixels { x: 0.0, y: -30.0 });
        assert_eq!(ui.offset, 30.0);

        set_enabled(false);
        ui.wheel(ScrollDelta::Lines { x: 0.0, y: -1.0 });
        set_enabled(true);
        assert_eq!(ui.offset, 90.0, "iced moves 60 px per line on its own");
    }

    #[test]
    fn a_click_stops_the_rest_of_a_notch() {
        let _guard = SWITCH.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        set_enabled(true);
        let mut ui = Pipeline::new();
        ui.wheel(ScrollDelta::Lines { x: 0.0, y: -3.0 });
        ui.frame();
        ui.frame();
        let held = ui.offset;
        assert!(held > 0.0);
        ui.send(Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)), Point::new(200.0, 150.0));
        for _ in 0..20 {
            ui.frame();
        }
        assert_eq!(ui.offset, held);
    }

    #[test]
    fn a_wheel_outside_the_list_is_left_alone() {
        let _guard = SWITCH.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        set_enabled(true);
        let mut ui = Pipeline::new();
        ui.send(Event::Mouse(mouse::Event::WheelScrolled { delta: ScrollDelta::Lines { x: 0.0, y: -1.0 } }), Point::new(600.0, 150.0));
        for _ in 0..20 {
            ui.frame();
        }
        assert_eq!(ui.offset, 0.0);
    }

    #[test]
    fn a_fast_spin_moves_at_an_even_speed() {
        // A notch every 32 ms, drawn at 125 frames per second.
        let mut state = State::default();
        let start = Instant::now();
        let mut steps = Vec::new();
        for frame in 0..60_u64 {
            if frame % 4 == 0 && frame < 48 {
                state.push(Vector::new(0.0, 60.0));
            }
            steps.push(state.take(start + Duration::from_millis(8 * frame)).y);
        }
        // Once the motion is up to speed, no frame moves twice as far as another.
        let cruise = &steps[12..44];
        let (low, high) = cruise.iter().fold((f32::MAX, 0.0_f32), |(low, high), step| (low.min(*step), high.max(*step)));
        assert!(high < low * 2.0, "steps pulse between {low} and {high}");
        assert!(steps.iter().all(|step| *step >= 0.0), "the list never moves back");
    }

    #[test]
    fn pending_distance_is_capped() {
        let mut state = State::default();
        for _ in 0..200 {
            state.push(Vector::new(0.0, 60.0));
        }
        assert_eq!(state.remaining().y, MAX_PENDING_PIXELS);
    }
}
