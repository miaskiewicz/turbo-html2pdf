//! What the fragmenter knows about a box beyond its fragment's border box
//! (miaskiewicz/turbo-html2pdf#32). A [`Fragment`] carries only its border box, so a
//! margin cannot be told apart from the padding (or remaining height) of an ancestor
//! around it, nor an out-of-flow box from a flow one. The render orchestrator records
//! these used values from the styled document for the body flow ([`FlowBoxes`],
//! `crate::flow_boxes`); a box without an entry is an in-flow box whose margins are
//! unknown, and the fragmenter keeps the galley gaps around it, as before.

use std::collections::HashMap;

use crate::layout::fragment::{Fragment, FragmentContent, NodeId};
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
    fn a_replaced_box_has_no_top_border_of_its_own() {
        let image = ImagePlacement {
            name: "a.png".to_string(),
            intrinsic_w: 1,
            intrinsic_h: 1,
            has_alpha: false,
            tint: None,
            orientation: Default::default(),
        };
        let mut f = Fragment::new(
            NodeId(1),
            0.0,
            10.0,
            10.0,
            10.0,
            FragmentContent::Image(image),
        );
        f.children = vec![boxed(2, 10.0, vec![])];
        assert_eq!(
            leading_margin(&margins(&[(1, 10.0, 0.0), (2, 25.0, 0.0)]), &f),
            Some(25.0)
        );
    }
}
