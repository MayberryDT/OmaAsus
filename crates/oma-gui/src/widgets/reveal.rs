//! `Reveal`: draws its child slid up and slightly shrunk by `1 - progress`,
//! clipped to its own bounds. It is how the overlay panel drops down from the
//! bar and folds back up (the compositor adds its fade on top).

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{overlay, Clipboard, Shell};
use iced::{Element, Event, Length, Rectangle, Size, Transformation, Vector, mouse};

/// Fraction of the panel height the content travels while revealing.
const TRAVEL: f32 = 0.32;
/// Scale at the start of the reveal (grows to 1).
const SCALE_FROM: f32 = 0.97;

pub struct Reveal<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    progress: f32,
}

impl<'a, Message, Theme, Renderer> Reveal<'a, Message, Theme, Renderer> {
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>, progress: f32) -> Self {
        Self { content: content.into(), progress: progress.clamp(0.0, 1.0) }
    }

    /// The transform applied at this progress: slide up by `TRAVEL` of the
    /// height and shrink about the top centre, so the panel appears to hang
    /// from the bar.
    fn transformation(&self, bounds: Rectangle) -> Transformation {
        let t = self.progress;
        let dy = -(1.0 - t) * bounds.height * TRAVEL;
        let s = SCALE_FROM + (1.0 - SCALE_FROM) * t;
        let cx = bounds.x + bounds.width / 2.0;
        let cy = bounds.y;
        Transformation::translate(cx, cy + dy) * Transformation::scale(s) * Transformation::translate(-cx, -cy)
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Reveal<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        if self.progress >= 1.0 {
            self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
            return;
        }
        let bounds = layout.bounds();
        let tf = self.transformation(bounds);
        renderer.with_layer(bounds, |r| {
            r.with_transformation(tf, |r| {
                self.content.as_widget().draw(&tree.children[0], r, theme, style, layout, cursor, viewport);
            });
        });
    }

    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a, Theme: 'a, Renderer: renderer::Renderer + 'a> From<Reveal<'a, Message, Theme, Renderer>> for Element<'a, Message, Theme, Renderer> {
    fn from(r: Reveal<'a, Message, Theme, Renderer>) -> Self {
        Element::new(r)
    }
}

pub fn reveal<'a, Message: 'a, Theme: 'a, Renderer: renderer::Renderer + 'a>(content: impl Into<Element<'a, Message, Theme, Renderer>>, progress: f32) -> Reveal<'a, Message, Theme, Renderer> {
    Reveal::new(content, progress)
}

/// Easing for the drop-down: fast out of the bar, settling gently.
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Easing for folding back up: starts slow, accelerates into the bar.
pub fn ease_in_cubic(t: f32) -> f32 {
    t.clamp(0.0, 1.0).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_endpoints() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert_eq!(ease_in_cubic(0.0), 0.0);
        assert_eq!(ease_in_cubic(1.0), 1.0);
        assert!(ease_out_cubic(0.5) > 0.5, "ease-out is ahead of linear");
        assert!(ease_in_cubic(0.5) < 0.5, "ease-in lags linear");
        assert_eq!(ease_out_cubic(2.0), 1.0, "clamped");
    }

    #[test]
    fn transform_is_identity_when_done() {
        let r: Reveal<'_, (), iced::Theme, iced::Renderer> = Reveal::new(iced::widget::Space::new(), 1.0);
        let b = Rectangle { x: 10.0, y: 20.0, width: 560.0, height: 1040.0 };
        let tf = r.transformation(b);
        assert!((tf.scale_factor() - 1.0).abs() < 1e-5);
        let v = tf.translation();
        assert!(v.x.abs() < 1e-3 && v.y.abs() < 1e-3, "{v:?}");
    }

    #[test]
    fn transform_slides_up_at_start() {
        let r: Reveal<'_, (), iced::Theme, iced::Renderer> = Reveal::new(iced::widget::Space::new(), 0.0);
        let b = Rectangle { x: 0.0, y: 0.0, width: 560.0, height: 1000.0 };
        let tf = r.transformation(b);
        assert!((tf.scale_factor() - SCALE_FROM).abs() < 1e-5);
        // Top centre stays horizontally put; content starts TRAVEL of its height above.
        let v = tf.translation();
        assert!((v.y - (-TRAVEL * 1000.0)).abs() < 1e-3, "{v:?}");
        assert!((v.x - 280.0 * (1.0 - SCALE_FROM)).abs() < 1e-3, "{v:?}");
    }
}
