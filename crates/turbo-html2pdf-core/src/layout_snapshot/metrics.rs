use super::Rect;
use crate::layout::value::{Display, Edges, Position};

/// Values captured from the actual resolved box style during layout. They are
/// local to the fragment, so subsequent flex/relative-position translations do
/// not leave a stale absolute content origin. They do not claim scrollbar or
/// scrollable-overflow metrics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsedBoxMetrics {
    /// Real non-replaced inline fragment, distinguished from inline images/controls.
    pub inline_fragment: bool,
    pub border: Edges,
    pub padding: Edges,
    pub margin: Edges,
    pub content_width: f32,
    pub content_height: f32,
    pub position: Position,
    pub display: Display,
}

impl UsedBoxMetrics {
    pub fn content_rect(self, border_box: Rect) -> Rect {
        Rect {
            x: border_box.x + self.border.left + self.padding.left,
            y: border_box.y + self.border.top + self.padding.top,
            width: self.content_width,
            height: self.content_height,
        }
    }

    pub fn padding_rect(self, border_box: Rect) -> Rect {
        Rect {
            x: border_box.x + self.border.left,
            y: border_box.y + self.border.top,
            width: self.content_width + self.padding.horizontal(),
            height: self.content_height + self.padding.vertical(),
        }
    }
}
