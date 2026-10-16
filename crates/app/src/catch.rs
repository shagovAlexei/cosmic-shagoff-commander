//! Takes the right button's release over its child while `on`, so layers under it never see it:
//! libcosmic's `context_menu` opens on that release, and it cannot be given no menu (its `diff`
//! unwraps it). Everything else, drop zones too, passes through.

use cosmic::iced::advanced::widget::{Operation, Tree};
use cosmic::iced::advanced::{Clipboard, Shell, Widget, layout, mouse, overlay, renderer};
use cosmic::iced::{Event, Length, Rectangle, Size, Vector};
use cosmic::{Element, Renderer, Theme};

pub struct CatchRightUp<'a, Message> {
    child: Element<'a, Message>,
    on: bool,
}

pub fn catch_right_up<'a, Message>(
    child: impl Into<Element<'a, Message>>,
    on: bool,
) -> CatchRightUp<'a, Message> {
    CatchRightUp {
        child: child.into(),
        on,
    }
}

impl<Message> Widget<Message, Theme, Renderer> for CatchRightUp<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.child)]
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.child));
    }

    fn size(&self) -> Size<Length> {
        self.child.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        (self.child.as_widget_mut()).layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: layout::Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        (self.child.as_widget_mut()).operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.child.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        if self.on
            && matches!(
                event,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right))
            )
            && cursor.is_over(layout.bounds())
        {
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        (self.child.as_widget()).mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        (self.child.as_widget()).draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: layout::Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        (self.child.as_widget_mut()).overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }

    fn drag_destinations(
        &self,
        tree: &Tree,
        layout: layout::Layout<'_>,
        renderer: &Renderer,
        rects: &mut cosmic::iced::advanced::clipboard::DndDestinationRectangles,
    ) {
        (self.child.as_widget()).drag_destinations(&tree.children[0], layout, renderer, rects);
    }
}

impl<'a, Message: 'a> From<CatchRightUp<'a, Message>> for Element<'a, Message> {
    fn from(c: CatchRightUp<'a, Message>) -> Self {
        Element::new(c)
    }
}
