// Made by Nordan
//! Interactive surface that responds on pointer down, not on click.
//!
//! The highlight and `scale(0.98)` appear on the exact frame the button goes
//! down. Dragging past the hit padding cancels, dragging back re-engages, and
//! losing window focus releases the press so it can never stick.
//!
//! The hover tint fades in and out over a short time. A row inside a grouped
//! list is wrapped in `corners`, which tells the row which of its corners
//! meet the rounded edge of the group, so the tint follows that edge.

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Renderer as _, Shell};
use iced::border::Radius;
use iced::mouse::{self, Cursor};
use iced::{Border, Color, Element, Event, Length, Rectangle, Renderer, Size, Theme, Transformation, Vector, window};
use std::cell::Cell;
use std::time::{Duration, Instant};
use windowcraft_physics::PressTracker;

const PRESSED_SCALE: f32 = 0.98;
const HOVER_FADE: Duration = Duration::from_millis(110);

thread_local! {
    /// Bounds and corner radii of the `corners` slot being drawn right now.
    static SLOT: Cell<Option<(Rectangle, Radius)>> = const { Cell::new(None) };
}

pub struct Press<'a, Message> {
    content: Element<'a, Message>,
    on_press: Option<Message>,
    hover: Option<Color>,
    pressed: Option<Color>,
    radius: f32,
}

#[derive(Default)]
struct State {
    press: PressTracker,
    hovered: bool,
    /// Tint level when the hover last changed, and when that was.
    fade_from: f32,
    fade_start: Option<Instant>,
}

impl State {
    /// Hover tint from 0 to 1 at `now`.
    fn hover_level(&self, now: Instant) -> f32 {
        let target = if self.hovered { 1.0 } else { 0.0 };
        let Some(start) = self.fade_start else { return target };
        let t = now.saturating_duration_since(start).as_secs_f32() / HOVER_FADE.as_secs_f32();
        if t >= 1.0 {
            return target;
        }
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        self.fade_from + (target - self.fade_from) * eased
    }

    fn fading(&self, now: Instant) -> bool {
        self.fade_start.is_some_and(|start| now.saturating_duration_since(start) < HOVER_FADE)
    }

    fn set_hovered(&mut self, hovered: bool, now: Instant, motion: bool) {
        self.fade_from = self.hover_level(now);
        self.hovered = hovered;
        self.fade_start = motion.then_some(now);
    }
}

/// Radii for a fill drawn over `bounds`: the slot's own when the fill covers
/// the whole slot, otherwise `fallback` on every corner.
fn radius_for(bounds: Rectangle, fallback: f32) -> Radius {
    match SLOT.with(Cell::get) {
        Some((slot, radius))
            if (slot.x - bounds.x).abs() < 0.5
                && (slot.y - bounds.y).abs() < 0.5
                && (slot.width - bounds.width).abs() < 0.5
                && (slot.height - bounds.height).abs() < 0.5 =>
        {
            radius
        }
        _ => fallback.into(),
    }
}

pub fn press<'a, Message>(content: impl Into<Element<'a, Message>>) -> Press<'a, Message> {
    Press { content: content.into(), on_press: None, hover: None, pressed: None, radius: 6.0 }
}

impl<'a, Message> Press<'a, Message> {
    pub fn on_press(mut self, message: Message) -> Self {
        self.on_press = Some(message);
        self
    }

    pub fn on_press_maybe(mut self, message: Option<Message>) -> Self {
        self.on_press = message;
        self
    }

    pub fn colors(mut self, hover: Color, pressed: Color) -> Self {
        self.hover = Some(hover);
        self.pressed = Some(pressed);
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
}

impl<Message: Clone> Widget<Message, Theme, Renderer> for Press<'_, Message> {
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

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
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
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );

        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<State>();

        if let Event::Window(window::Event::Unfocused) = event {
            if state.press.is_down() {
                state.press.cancel();
                shell.request_redraw();
            }
            return;
        }

        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            if self.hover.is_some() && state.fading(*now) {
                shell.request_redraw_at(crate::vsync::next_redraw(*now));
            } else {
                state.fade_start = None;
            }
        }

        let over = cursor.is_over(bounds);
        if over != state.hovered {
            state.set_hovered(over, Instant::now(), crate::smooth::motion_enabled());
            if self.hover.is_some() {
                shell.request_redraw();
            }
        }

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if over && self.on_press.is_some() && !shell.is_event_captured() {
                    state.press.pointer_down();
                    shell.capture_event();
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if state.press.is_down() {
                    let before = state.press.is_pressed();
                    state.press.pointer_move(
                        f64::from(position.x - bounds.x),
                        f64::from(position.y - bounds.y),
                        f64::from(bounds.width),
                        f64::from(bounds.height),
                    );
                    if before != state.press.is_pressed() {
                        shell.request_redraw();
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.press.is_down() => {
                let activated = state.press.pointer_up();
                shell.request_redraw();
                if activated && let Some(message) = &self.on_press {
                    shell.publish(message.clone());
                    shell.capture_event();
                }
            }
            _ => {}
        }
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
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let pressed = state.press.is_pressed();
        let fill = if pressed {
            self.pressed
        } else if self.on_press.is_some() {
            let level = state.hover_level(Instant::now());
            self.hover.filter(|_| level > 0.0).map(|color| Color { a: color.a * level, ..color })
        } else {
            None
        };
        let radius = radius_for(bounds, self.radius);

        let paint = |renderer: &mut Renderer| {
            if let Some(color) = fill {
                renderer.fill_quad(
                    Quad { bounds, border: Border { radius, ..Border::default() }, ..Quad::default() },
                    color,
                );
            }
            self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
        };

        if pressed {
            let center = bounds.center();
            let transform = Transformation::translate(center.x, center.y)
                * Transformation::scale(PRESSED_SCALE)
                * Transformation::translate(-center.x, -center.y);
            renderer.with_transformation(transform, |renderer| paint(renderer));
        } else {
            paint(renderer);
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let inner =
            self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer);
        if inner == mouse::Interaction::None && self.on_press.is_some() && cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            inner
        }
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

impl<'a, Message: Clone + 'a> From<Press<'a, Message>> for Element<'a, Message> {
    fn from(value: Press<'a, Message>) -> Self {
        Element::new(value)
    }
}

/// Gives a row the corner radii of the group edge it touches.
pub struct Corners<'a, Message> {
    content: Element<'a, Message>,
    radius: Radius,
}

pub fn corners<'a, Message>(content: impl Into<Element<'a, Message>>, radius: Radius) -> Corners<'a, Message> {
    Corners { content: content.into(), radius }
}

impl<Message> Widget<Message, Theme, Renderer> for Corners<'_, Message> {
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
        let outer = SLOT.with(|slot| slot.replace(Some((layout.bounds(), self.radius))));
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
        SLOT.with(|slot| slot.set(outer));
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

impl<'a, Message: 'a> From<Corners<'a, Message>> for Element<'a, Message> {
    fn from(value: Corners<'a, Message>) -> Self {
        Element::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::space;

    #[test]
    fn builder_configures_handlers_colors_radius_and_delegates_size_to_content() {
        let configured = press(space().width(120).height(36))
            .on_press("activate")
            .colors(Color::WHITE, Color::BLACK)
            .radius(10.0);
        assert_eq!(configured.on_press, Some("activate"));
        assert_eq!(configured.hover, Some(Color::WHITE));
        assert_eq!(configured.pressed, Some(Color::BLACK));
        assert_eq!(configured.radius, 10.0);

        let disabled = press::<&str>(space().width(80).height(24)).on_press_maybe(None);
        assert_eq!(disabled.on_press, None);
        let element: Element<'_, &str> = configured.into();
        assert_eq!(element.as_widget().size(), Size::new(Length::Fixed(120.0), Length::Fixed(36.0)));
        assert_eq!(element.as_widget().size_hint(), Size::new(Length::Fixed(120.0), Length::Fixed(36.0)));
    }

    #[test]
    fn a_fill_takes_the_slot_corners_only_when_it_covers_the_slot() {
        let slot = Rectangle { x: 10.0, y: 20.0, width: 300.0, height: 42.0 };
        let top = Radius { top_left: 8.0, top_right: 8.0, bottom_right: 0.0, bottom_left: 0.0 };
        assert_eq!(radius_for(slot, 0.0), Radius::from(0.0), "no slot, so the fallback applies");
        SLOT.with(|cell| cell.set(Some((slot, top))));
        assert_eq!(radius_for(slot, 0.0), top);
        let button = Rectangle { x: 250.0, y: 28.0, width: 26.0, height: 26.0 };
        assert_eq!(radius_for(button, 6.0), Radius::from(6.0), "a control inside the row keeps its own radius");
        SLOT.with(|cell| cell.set(None));
    }

    #[test]
    fn hover_fades_in_and_out_and_reverses_from_where_it_is() {
        let start = Instant::now();
        let mut state = State::default();
        assert_eq!(state.hover_level(start), 0.0);
        state.set_hovered(true, start, true);
        let mid = state.hover_level(start + HOVER_FADE / 2);
        assert!(mid > 0.0 && mid < 1.0);
        assert!(state.fading(start + HOVER_FADE / 2));
        assert_eq!(state.hover_level(start + HOVER_FADE), 1.0);
        assert!(!state.fading(start + HOVER_FADE));

        state.set_hovered(false, start + HOVER_FADE / 2, true);
        let reversed = state.hover_level(start + HOVER_FADE / 2);
        assert!((reversed - mid).abs() < 1e-4, "leaving mid-fade starts from the current tint, not from full");
        assert_eq!(state.hover_level(start + HOVER_FADE * 2), 0.0);
    }

    #[test]
    fn widget_tree_initializes_and_preserves_press_state_across_view_diffs() {
        let first: Element<'_, &'static str> = press(space().width(100).height(40)).on_press("hit").into();
        let mut tree = Tree::new(&first);
        assert_eq!(tree.children.len(), 1);
        {
            let state = tree.state.downcast_mut::<State>();
            assert!(!state.hovered);
            assert!(!state.press.is_down());
            assert!(!state.press.is_pressed());
            state.hovered = true;
            state.press.pointer_down();
            state.press.pointer_move(50.0, 20.0, 100.0, 40.0);
            assert!(state.press.is_pressed());
        }

        let rebuilt: Element<'_, &'static str> = press(space().width(100).height(40)).on_press("hit").into();
        tree.diff(&rebuilt);
        let state = tree.state.downcast_mut::<State>();
        assert!(state.hovered, "diffing a rebuilt view must not reset hover state");
        assert!(state.press.is_pressed(), "diffing after pointer_down must keep the active press");
        assert!(state.press.pointer_up());
        assert!(!state.press.is_down());
    }
}

