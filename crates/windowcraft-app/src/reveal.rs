// Made by WindowCraft Dev Team
//! Entrance transition for a page body.
//!
//! When the key changes, the content rises a few pixels into place. The
//! widget keeps its own clock and asks for redraws only, so the transition
//! never rebuilds the view or recomputes layout. There is no fade: tiny-skia
//! blends a translucent veil over the whole page at about ten times the cost
//! of an opaque fill, which made every transition frame slower than a scroll
//! frame. Under reduced motion the content appears at once.

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Renderer as _, Shell};
use iced::mouse::{self, Cursor};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Transformation, Vector, window};
use std::time::{Duration, Instant};

const DURATION: Duration = Duration::from_millis(220);
const RISE_PIXELS: f32 = 8.0;

pub struct Reveal<'a, Message> {
    content: Element<'a, Message>,
    key: u64,
}

#[derive(Debug)]
struct State {
    key: u64,
    start: Option<Instant>,
}

impl State {
    fn new(key: u64) -> Self {
        Self { key, start: crate::smooth::motion_enabled().then(Instant::now) }
    }

    /// Eased progress from 0 to 1. `None` once the transition is over.
    fn progress(&self, now: Instant) -> Option<f32> {
        let start = self.start?;
        let linear = now.saturating_duration_since(start).as_secs_f32() / DURATION.as_secs_f32();
        (linear < 1.0).then(|| ease_out_cubic(linear))
    }
}

fn ease_out_cubic(t: f32) -> f32 {
    let inverse = 1.0 - t.clamp(0.0, 1.0);
    1.0 - inverse * inverse * inverse
}

pub fn reveal<'a, Message>(content: impl Into<Element<'a, Message>>, key: u64) -> Reveal<'a, Message> {
    Reveal { content: content.into(), key }
}

impl<Message> Widget<Message, Theme, Renderer> for Reveal<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new(self.key))
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        if state.key != self.key {
            *state = State::new(self.key);
        }
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
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            let state = tree.state.downcast_mut::<State>();
            if state.progress(*now).is_some() {
                shell.request_redraw_at(crate::vsync::next_redraw(*now));
            } else {
                state.start = None;
            }
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
        let state = tree.state.downcast_ref::<State>();
        let content = &tree.children[0];
        let Some(progress) = state.progress(Instant::now()) else {
            self.content.as_widget().draw(content, renderer, theme, style, layout, cursor, viewport);
            return;
        };
        // Whole pixels, so text never lands between them on the way.
        let rise = (RISE_PIXELS * (1.0 - progress)).round();
        renderer.with_transformation(Transformation::translate(0.0, rise), |renderer| {
            self.content.as_widget().draw(content, renderer, theme, style, layout, cursor, viewport);
        });
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

impl<'a, Message: 'a> From<Reveal<'a, Message>> for Element<'a, Message> {
    fn from(value: Reveal<'a, Message>) -> Self {
        Element::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_starts_at_zero_and_ends_at_one() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert!(ease_out_cubic(0.5) > 0.5, "ease-out covers most of the way early");
    }

    #[test]
    fn a_transition_ends_after_its_duration() {
        let start = Instant::now();
        let state = State { key: 1, start: Some(start) };
        assert!(state.progress(start).is_some());
        assert!(state.progress(start + DURATION).is_none());
    }

    #[test]
    fn without_a_start_there_is_no_transition() {
        let state = State { key: 1, start: None };
        assert!(state.progress(Instant::now()).is_none());
    }

    #[test]
    fn a_new_key_restarts_and_the_same_key_keeps_the_clock() {
        let first: Element<'_, ()> = reveal(iced::widget::space(), 1).into();
        let mut tree = Tree::new(&first);
        tree.state.downcast_mut::<State>().start = None;

        let same: Element<'_, ()> = reveal(iced::widget::space(), 1).into();
        tree.diff(&same);
        assert!(tree.state.downcast_ref::<State>().start.is_none());

        let next: Element<'_, ()> = reveal(iced::widget::space(), 2).into();
        tree.diff(&next);
        assert_eq!(tree.state.downcast_ref::<State>().key, 2);
    }
}
