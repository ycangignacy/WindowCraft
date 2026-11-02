// Made by ycangignacy
//! Interactive surface that responds on pointer down, not on click.
//!
//! The highlight and `scale(0.98)` appear on the exact frame the button goes
//! down. Dragging past the hit padding cancels, dragging back re-engages, and
//! losing window focus releases the press so it can never stick.

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Renderer as _, Shell};
use iced::mouse::{self, Cursor};
use iced::{Border, Color, Element, Event, Length, Rectangle, Renderer, Size, Theme, Transformation, Vector, window};
use windowcraft_physics::PressTracker;

const PRESSED_SCALE: f32 = 0.98;

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

        let over = cursor.is_over(bounds);
        if over != state.hovered {
            state.hovered = over;
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
        } else if state.hovered && self.on_press.is_some() {
            self.hover
        } else {
            None
        };

        let paint = |renderer: &mut Renderer| {
            if let Some(color) = fill {
                renderer.fill_quad(
                    Quad {
                        bounds,
                        border: Border { radius: self.radius.into(), ..Border::default() },
                        ..Quad::default()
                    },
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

