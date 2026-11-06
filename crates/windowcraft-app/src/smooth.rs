// Made by ycangignacy
//! Eased mouse-wheel scrolling for any scrollable inside it.
//!
//! Iced moves a scrollable by a fixed 60 px on every wheel notch, so a list
//! jumps in steps. This wrapper takes line-based wheel events before the
//! scrollable sees them, adds them to a pending distance and hands that
//! distance out over the next display frames as pixel deltas. Pixel deltas
//! from precision touchpads are already smooth and pass through untouched.
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

/// Time constant of the exponential ease. About 95% of a notch arrives
/// within three of these.
const EASE_SECONDS: f32 = 0.055;
/// Below this the rest is handed out at once, so the tail never crawls.
const FINISH_PIXELS: f32 = 0.6;
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

#[derive(Debug, Default)]
struct State {
    pending: Vector,
    cursor: Option<Point>,
    last_frame: Option<Instant>,
    modifiers: Modifiers,
}

impl State {
    fn is_moving(&self) -> bool {
        self.pending.x != 0.0 || self.pending.y != 0.0
    }

    fn stop(&mut self) {
        self.pending = Vector::ZERO;
        self.last_frame = None;
    }

    /// Adds a notch. A notch against the current direction drops what is
    /// left, so reversing responds on the next frame.
    fn push(&mut self, delta: Vector) {
        let reverse = |pending: f32, next: f32| pending * next < 0.0;
        if reverse(self.pending.x, delta.x) || reverse(self.pending.y, delta.y) {
            self.pending = Vector::ZERO;
        }
        self.pending = Vector::new(
            (self.pending.x + delta.x).clamp(-MAX_PENDING_PIXELS, MAX_PENDING_PIXELS),
            (self.pending.y + delta.y).clamp(-MAX_PENDING_PIXELS, MAX_PENDING_PIXELS),
        );
    }

    /// Takes this frame's share of the pending distance.
    fn take(&mut self, now: Instant) -> Vector {
        let dt = self
            .last_frame
            .map_or(1.0 / 60.0, |last| now.saturating_duration_since(last).as_secs_f32())
            .clamp(0.0, MAX_STEP_SECONDS);
        self.last_frame = Some(now);
        let share = 1.0 - (-dt / EASE_SECONDS).exp();
        let mut step = self.pending * share;
        if (self.pending.x - step.x).abs() < FINISH_PIXELS {
            step.x = self.pending.x;
        }
        if (self.pending.y - step.y).abs() < FINISH_PIXELS {
            step.y = self.pending.y;
        }
        self.pending -= step;
        if !self.is_moving() {
            self.last_frame = None;
        }
        step
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
                    shell.request_redraw_at(crate::vsync::next_frame(*now));
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
        assert_eq!(state.pending.y, -60.0);
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
            let renderer = Renderer::new(iced::Font::DEFAULT, iced::Pixels(16.0));
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
    fn pending_distance_is_capped() {
        let mut state = State::default();
        for _ in 0..200 {
            state.push(Vector::new(0.0, 60.0));
        }
        assert_eq!(state.pending.y, MAX_PENDING_PIXELS);
    }
}
