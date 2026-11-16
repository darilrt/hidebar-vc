//! Pill that stretches (ADR-0006): lays its content out at natural size, but takes only `size` of that
//! width, with the rounded background drawn at that width and the content clipped to it.
//!
//! Used for the module pills, whose width depends on text and cannot be computed up front.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Tree, Widget};
use iced::advanced::{Renderer as _, mouse};
use iced::{Border, Color, Element, Length, Padding, Rectangle, Size};

pub(crate) struct Stretch<'a, Message> {
    content: Element<'a, Message>,
    /// Share of the natural width, from 0 (a circle of `height`) to 1.
    size: f32,
    background: Color,
    height: f32,
    padding: Padding,
}

impl<'a, Message> Stretch<'a, Message> {
    pub(crate) fn new(
        content: impl Into<Element<'a, Message>>,
        size: f32,
        background: Color,
        height: f32,
        padding: Padding,
    ) -> Self {
        Self {
            content: content.into(),
            size,
            background,
            height,
            padding,
        }
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Stretch<'_, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Fixed(self.height))
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let inner = layout::Limits::new(Size::ZERO, Size::new(f32::INFINITY, self.height));
        let content = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &inner);
        let natural = content.size().width + self.padding.left + self.padding.right;
        let width = (natural * self.size)
            .max(self.height)
            .min(limits.max().width);
        let y = (self.height - content.size().height) / 2.0;
        // Centered: once stretched it sits as a regular pill would.
        let x = (width - content.size().width) / 2.0;
        layout::Node::with_children(Size::new(width, self.height), vec![content.move_to((x, y))])
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        renderer.fill_quad(
            Quad {
                bounds,
                border: Border::default().rounded(self.height / 2.0),
                ..Quad::default()
            },
            self.background,
        );
        let Some(content) = layout.children().next() else {
            return;
        };
        renderer.with_layer(bounds, |renderer| {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                content,
                cursor,
                viewport,
            );
        });
    }
}

impl<'a, Message: 'a> From<Stretch<'a, Message>> for Element<'a, Message> {
    fn from(stretch: Stretch<'a, Message>) -> Self {
        Element::new(stretch)
    }
}
