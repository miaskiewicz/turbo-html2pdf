//! What the fragmenter knows about a box beyond its fragment's border box
//! (miaskiewicz/turbo-html2pdf#32). A [`Fragment`] carries only its border box, so a
//! margin cannot be told apart from the padding (or remaining height) of an ancestor
//! around it, nor an out-of-flow box from a flow one. The render orchestrator records
//! these used values from the styled document for the body flow ([`FlowBoxes`],
//! `crate::flow_boxes`); a box without an entry is an in-flow box whose margins are
//! unknown, and the fragmenter keeps the galley gaps around it, as before.

use std::collections::HashMap;

use crate::layout::fragment::{Fragment, FragmentContent, NodeId, RepeatKind};
use crate::layout::value::{LengthPct, Position};

/// The used values of one box that the fragmenter reads (px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxFacts {
    pub position: Position,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub padding_top: f32,
    pub padding_bottom: f32,
    /// `top` / `bottom` (`Auto` when unset).
    pub inset_top: LengthPct,
    pub inset_bottom: LengthPct,
    /// The vertical paint shift of a `relative`/`sticky` box: it is painted that far
    /// below its flow position, with its subtree.
    pub shift: f32,
}

impl BoxFacts {
    /// Whether an out-of-flow box is placed by a vertical inset (`top` or `bottom` set)
    /// rather than at its static position.
    pub(super) fn anchored(&self) -> bool {
        self.inset_top != LengthPct::Auto || self.inset_bottom != LengthPct::Auto
    }
}

/// [`BoxFacts`] by box id, for one laid-out flow.
#[derive(Debug, Clone, Default)]
pub struct FlowBoxes {
    boxes: HashMap<NodeId, BoxFacts>,
}

impl FlowBoxes {
    /// Record the facts of box `id`.
    pub fn insert(&mut self, id: NodeId, facts: BoxFacts) {
        self.boxes.insert(id, facts);
    }

    /// The facts recorded for box `id`.
    pub fn get(&self, id: NodeId) -> Option<&BoxFacts> {
        self.boxes.get(&id)
    }
}

/// The collapsed top margin above `f`'s border box (CSS 2.2 §8.3.1): its own top
/// margin, collapsed with its first child's when nothing (padding, border) separates
/// them and the child starts at its top. `None` when `f`'s facts are unknown.
pub(super) fn leading_margin(boxes: &FlowBoxes, f: &Fragment) -> Option<f32> {
    let facts = boxes.get(f.node_id)?;
    let inner = if facts.padding_top + top_border(f) > 0.0 {
        None
    } else {
        f.children
            .first()
            .filter(|c| (c.y - f.y).abs() < 0.5)
            .and_then(|c| leading_margin(boxes, c))
    };
    Some(collapse(facts.margin_top, inner.unwrap_or(0.0)))
}

/// The index of the last in-flow child of `children` (out-of-flow boxes and repeated
/// table headers are not content of the flow).
pub(super) fn last_content(boxes: &FlowBoxes, children: &[Fragment]) -> Option<usize> {
    children.iter().rposition(|c| {
        let out = boxes
            .get(c.node_id)
            .is_some_and(|f| f.position.is_out_of_flow());
        !out && c.break_meta.repeatable != Some(RepeatKind::Header)
    })
}

/// What must follow `last`, the last content of `parent`, on the same page: `parent`'s
/// bottom border and padding with `last`'s bottom margin inside them, plus `after`, what
/// must follow `parent` itself. No break can separate a box's last content from the
/// bottom padding/border that ends the box (Chrome 156 moves the content on with them,
/// miaskiewicz/turbo-html2pdf#32 item 6), while a trailing margin alone is truncated at
/// a page end (so a parent without bottom padding/border adds nothing of its own and
/// passes `after` through). A parent taller than its content (a definite height) ends
/// in breakable remaining height: nothing has to follow `last`.
pub(super) fn trailing(boxes: &FlowBoxes, parent: &Fragment, last: &Fragment, after: f32) -> f32 {
    if remaining_height(boxes, parent, last).is_some() {
        return 0.0;
    }
    if bottom_frame(boxes, parent) <= 0.0 {
        return after;
    }
    parent.bottom() - last.bottom() + after
}

/// The part of `parent` below its last content `last` (and `last`'s bottom margin) when
/// `parent` is taller than its content — its remaining height plus its bottom padding
/// and border, which break like content — or `None` when its content ends it.
pub(super) fn remaining_height(
    boxes: &FlowBoxes,
    parent: &Fragment,
    last: &Fragment,
) -> Option<f32> {
    let margin = boxes
        .get(last.node_id)
        .map_or(0.0, |f| f.margin_bottom.max(0.0));
    let end = last.bottom() + margin;
    (parent.bottom() - bottom_frame(boxes, parent) > end + 0.5).then(|| parent.bottom() - end)
}

/// `parent`'s bottom padding + border.
fn bottom_frame(boxes: &FlowBoxes, parent: &Fragment) -> f32 {
    boxes.get(parent.node_id).map_or(0.0, |f| f.padding_bottom) + bottom_border(parent)
}

/// What must follow the galley root's last content `last`: the [`trailing`] extents of
/// the dissolved `<html>`/`<body>` boxes around it (the galley root itself has none).
pub(super) fn wrapper_tail(galley: super::Galley, last: &Fragment) -> f32 {
    let chain: Vec<&Fragment> = galley.wrappers.iter().chain([last]).collect();
    chain.windows(2).fold(0.0, |after, pair| {
        trailing(galley.boxes, pair[0], pair[1], after)
    })
}

/// Two adjoining margins collapsed: the largest positive plus the most negative.
fn collapse(a: f32, b: f32) -> f32 {
    a.max(b).max(0.0) + a.min(b).min(0.0)
}

/// The used top border width of a box fragment (0 for other content).
fn top_border(f: &Fragment) -> f32 {
    match &f.content {
        FragmentContent::Box { border, .. } => f32::from(border.top.width),
        _ => 0.0,
    }
}

/// The used bottom border width of a box fragment (0 for other content).
fn bottom_border(f: &Fragment) -> f32 {
    match &f.content {
        FragmentContent::Box { border, .. } => f32::from(border.bottom.width),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::fragment::ImagePlacement;
    use crate::layout::value::BorderEdges;

    fn boxed(id: u32, y: f32, children: Vec<Fragment>) -> Fragment {
        let content = FragmentContent::Box {
            background: None,
            border: BorderEdges::default(),
            border_radius: 0.0,
            shadow: None,
            gradient: None,
            transform: None,
        };
        let mut f = Fragment::new(NodeId(id), 0.0, y, 100.0, 50.0, content);
        f.children = children;
        f
    }

    fn margins(pairs: &[(u32, f32, f32)]) -> FlowBoxes {
        let mut boxes = FlowBoxes::default();
        for &(id, margin_top, padding_top) in pairs {
            let facts = BoxFacts {
                position: Position::Static,
                margin_top,
                margin_bottom: 0.0,
                padding_top,
                padding_bottom: 0.0,
                inset_top: LengthPct::Auto,
                inset_bottom: LengthPct::Auto,
                shift: 0.0,
            };
            boxes.insert(NodeId(id), facts);
        }
        boxes
    }

    #[test]
    fn leading_margin_collapses_through_a_borderless_top() {
        let f = boxed(1, 10.0, vec![boxed(2, 10.0, vec![])]);
        assert_eq!(
            leading_margin(&margins(&[(1, 10.0, 0.0), (2, 25.0, 0.0)]), &f),
            Some(25.0)
        );
        // Positive and negative margins: the largest positive plus the most negative.
        assert_eq!(
            leading_margin(&margins(&[(1, 10.0, 0.0), (2, -4.0, 0.0)]), &f),
            Some(6.0)
        );
        assert_eq!(
            leading_margin(&margins(&[(1, -3.0, 0.0), (2, -4.0, 0.0)]), &f),
            Some(-4.0)
        );
        // A child below the top (it does not adjoin) or with unknown facts adds nothing.
        let lower = boxed(1, 10.0, vec![boxed(2, 30.0, vec![])]);
        assert_eq!(
            leading_margin(&margins(&[(1, 10.0, 0.0), (2, 25.0, 0.0)]), &lower),
            Some(10.0)
        );
        assert_eq!(leading_margin(&margins(&[(1, 10.0, 0.0)]), &f), Some(10.0));
        assert_eq!(leading_margin(&margins(&[]), &f), None);
    }

    #[test]
    fn padding_or_a_border_keeps_the_child_margin_inside() {
        let f = boxed(1, 10.0, vec![boxed(2, 10.0, vec![])]);
        assert_eq!(
            leading_margin(&margins(&[(1, 10.0, 5.0), (2, 25.0, 0.0)]), &f),
            Some(10.0)
        );
        let mut bordered = f.clone();
        if let FragmentContent::Box { border, .. } = &mut bordered.content {
            border.top.width = 2;
        }
        assert_eq!(
            leading_margin(&margins(&[(1, 10.0, 0.0), (2, 25.0, 0.0)]), &bordered),
            Some(10.0)
        );
    }

    #[test]
    fn trailing_extent_of_the_last_content() {
        let mut parent = boxed(1, 0.0, vec![boxed(2, 0.0, vec![])]);
        parent.height = 70.0; // 50 child + 20 padding
        let child = parent.children[0].clone();
        let mut boxes = margins(&[(1, 0.0, 0.0)]);
        boxes.insert(
            NodeId(1),
            BoxFacts {
                padding_bottom: 20.0,
                ..*boxes.get(NodeId(1)).unwrap()
            },
        );
        assert_eq!(trailing(&boxes, &parent, &child, 5.0), 25.0);
        // No padding/border of its own: what follows the parent passes through.
        parent.height = 50.0;
        assert_eq!(trailing(&margins(&[]), &parent, &child, 5.0), 5.0);
        // A parent taller than its content ends in breakable remaining height.
        parent.height = 90.0;
        assert_eq!(trailing(&boxes, &parent, &child, 5.0), 0.0);
        // A non-box parent has no border.
        let mut image = image(3, 0.0);
        image.height = 50.0;
        image.children = vec![child.clone()];
        assert_eq!(trailing(&margins(&[]), &image, &child, 0.0), 0.0);
    }

    fn image(id: u32, y: f32) -> Fragment {
        let image = ImagePlacement {
            name: "a.png".to_string(),
            intrinsic_w: 1,
            intrinsic_h: 1,
            has_alpha: false,
            tint: None,
            orientation: Default::default(),
        };
        Fragment::new(
            NodeId(id),
            0.0,
            y,
            10.0,
            10.0,
            FragmentContent::Image(image),
        )
    }

    #[test]
    fn a_replaced_box_has_no_top_border_of_its_own() {
        let mut f = image(1, 10.0);
        f.children = vec![boxed(2, 10.0, vec![])];
        assert_eq!(
            leading_margin(&margins(&[(1, 10.0, 0.0), (2, 25.0, 0.0)]), &f),
            Some(25.0)
        );
    }
}
