// Adapted from libcosmic `src/widget/dnd_source.rs` (MPL-2.0, System76):
// https://github.com/pop-os/libcosmic — a drag can start with the right button too.

//! Drag source for panel rows. As libcosmic's `dnd_source`, but the right button starts a drag as
//! well (TC: drop → menu Copy / Move / Link) and `on_start` is told which button it was. The right
//! press is not captured: the row under it still gets the cursor and its context menu.

use cosmic::iced::advanced::widget::{Operation, Tree, tree};
use cosmic::iced::advanced::{
    Clipboard, Shell, Widget, clipboard, layout, mouse, overlay, renderer,
};
use cosmic::iced::clipboard::dnd::{DndAction, DndEvent, SourceEvent};
use cosmic::iced::clipboard::mime::AsMimeTypes;
use cosmic::iced::{Event, Length, Point, Rectangle, Size, Vector, window};
use cosmic::widget::{Id, container};
use cosmic::{Element, Renderer, Theme};

pub struct DragSource<'a, Message, D> {
    id: Id,
    action: DndAction,
    child: Element<'a, Message>,
    window: window::Id,
    content: Box<dyn Fn() -> D + 'a>,
    /// `true`: started with the right button.
    on_start: Box<dyn Fn(bool) -> Message + 'a>,
    on_end: Message,
}

pub fn drag_source<'a, Message: 'static, D>(
    child: impl Into<Element<'a, Message>>,
    window: window::Id,
    action: DndAction,
    content: impl Fn() -> D + 'a,
    on_start: impl Fn(bool) -> Message + 'a,
    on_end: Message,
) -> DragSource<'a, Message, D> {
    DragSource {
        id: Id::unique(),
        action,
        child: container(child).into(),
        window,
        content: Box::new(content),
        on_start: Box::new(on_start),
        on_end,
    }
}

#[derive(Debug, Default)]
struct State {
    /// Where a button went down, and whether it was the right one.
    pressed: Option<(Point, bool)>,
    dragging: bool,
}

const THRESHOLD: f32 = 8.0;

impl<Message: Clone + 'static, D: AsMimeTypes + Send + 'static> Widget<Message, Theme, Renderer>
    for DragSource<'_, Message, D>
{
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.child)]
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.child));
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
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
        self.child
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: layout::Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.child
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
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
        let state = tree.state.downcast_mut::<State>();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(
                b @ (mouse::Button::Left | mouse::Button::Right),
            )) => {
                if let Some(p) = cursor.position_over(layout.bounds()) {
                    let right = *b == mouse::Button::Right;
                    state.pressed = Some((p, right));
                    if !right {
                        shell.capture_event();
                    }
                }
            }
            // As libcosmic's: the left release and motion over the row stay with the row.
            Event::Mouse(mouse::Event::ButtonReleased(_)) => {
                if let Some((_, false)) = state.pressed.take() {
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let (Some((from, right)), Some(p)) = (state.pressed, cursor.position())
                    && p.distance(from) > THRESHOLD
                {
                    state.pressed = None;
                    state.dragging = true;
                    shell.publish((self.on_start)(right));
                    clipboard.start_dnd(
                        false,
                        Some(clipboard::DndSource::Surface(self.window)),
                        None,
                        Box::new((self.content)()),
                        self.action,
                    );
                }
                if cursor.is_over(layout.bounds()) {
                    shell.capture_event();
                }
            }
            Event::Dnd(DndEvent::Source(SourceEvent::Cancelled | SourceEvent::Finished))
                if state.dragging =>
            {
                state.dragging = false;
                shell.publish(self.on_end.clone());
            }
            _ => {}
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
        if tree.state.downcast_ref::<State>().dragging {
            return mouse::Interaction::Grabbing;
        }
        self.child.as_widget().mouse_interaction(
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
        self.child.as_widget().draw(
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
        self.child.as_widget_mut().overlay(
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
        self.child
            .as_widget()
            .drag_destinations(&tree.children[0], layout, renderer, rects);
    }

    fn id(&self) -> Option<Id> {
        Some(self.id.clone())
    }

    fn set_id(&mut self, id: Id) {
        self.id = id;
    }
}

impl<'a, Message: Clone + 'static, D: AsMimeTypes + Send + 'static> From<DragSource<'a, Message, D>>
    for Element<'a, Message>
{
    fn from(d: DragSource<'a, Message, D>) -> Self {
        Element::new(d)
    }
}
