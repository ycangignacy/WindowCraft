// Made by ycangignacy
//! A layer that reports a press anywhere it is reached and lets the press go on.
//!
//! It sits in the window stack under a floating card and over the pages. A
//! press on the card never reaches it, because the card takes the press
//! first. A press anywhere else publishes the message and is not captured,
//! so the button or row under the pointer still gets it.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::mouse::{self, Cursor};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, touch};

pub struct Outside<Message> {
    on_press: Message,
}

/// Fills its parent and publishes `on_press` for every button press over it.
pub fn outside<Message>(on_press: Message) -> Outside<Message> {
    Outside { on_press }
}

impl<Message: Clone> Widget<Message, Theme, Renderer> for Outside<Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.width(Length::Fill).height(Length::Fill).max())
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let pressed = matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(_)) | Event::Touch(touch::Event::FingerPressed { .. })
        );
        if pressed && !shell.is_event_captured() && cursor.is_over(layout.bounds()) {
            shell.publish(self.on_press.clone());
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout<'_>,
        _cursor: Cursor,
        _viewport: &Rectangle,
    ) {
    }
}

impl<'a, Message: Clone + 'a> From<Outside<Message>> for Element<'a, Message> {
    fn from(widget: Outside<Message>) -> Self {
        Element::new(widget)
    }
}
