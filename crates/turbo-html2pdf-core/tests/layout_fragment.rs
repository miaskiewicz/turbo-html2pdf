//! Galley fragment types (§5.5): construction, `bottom`, recursive `translate`,
//! break metadata, and node-id round-trip (AC-5.11).

mod common;

use turbo_html2pdf_core::layout::fragment::*;
use turbo_html2pdf_core::layout::value::{BorderEdges, BreakRule, Rgba};
use turbo_html2pdf_core::node::TKind;

fn box_content() -> FragmentContent {
    FragmentContent::Box {
        background: Some(Rgba::new(10, 20, 30, 255)),
        border: BorderEdges::default(),
        border_radius: 0.0,
        shadow: None,
        gradient: None,
    }
}

#[test]
fn fragment_new_defaults_and_bottom() {
    let f = Fragment::new(NodeId(7), 1.0, 2.0, 100.0, 50.0, box_content());
    assert_eq!(f.node_id, NodeId(7));
    assert_eq!(f.break_meta, BreakMeta::default());
    assert!(f.children.is_empty());
    assert_eq!(f.bottom(), 52.0);
}

#[test]
fn break_meta_defaults() {
    let m = BreakMeta::default();
    assert_eq!(m.break_before, BreakRule::Auto);
    assert_eq!(m.break_after, BreakRule::Auto);
    assert!(!m.break_inside_avoid);
    assert_eq!(m.orphans, 2);
    assert_eq!(m.widows, 2);
    assert_eq!(m.repeatable, None);
}

#[test]
fn repeatable_marks() {
    let header = BreakMeta {
        repeatable: Some(RepeatKind::Header),
        ..Default::default()
    };
    assert_eq!(header.repeatable, Some(RepeatKind::Header));
    let footer = BreakMeta {
        repeatable: Some(RepeatKind::Footer),
        ..Default::default()
    };
    assert_eq!(footer.repeatable, Some(RepeatKind::Footer));
}

#[test]
fn translate_moves_subtree() {
    let child = Fragment::new(NodeId(2), 5.0, 5.0, 10.0, 10.0, box_content());
    let mut parent = Fragment::new(NodeId(1), 0.0, 0.0, 20.0, 20.0, box_content());
    parent.children.push(child);
    parent.translate(3.0, 4.0);
    assert_eq!((parent.x, parent.y), (3.0, 4.0));
    assert_eq!((parent.children[0].x, parent.children[0].y), (8.0, 9.0));
}

#[test]
fn text_line_holds_glyphs_and_face() {
    let face = common::evolventa();
    let glyphs = vec![
        PositionedGlyph {
            glyph_id: 5,
            x: 0.0,
            y: 0.0,
        },
        PositionedGlyph {
            glyph_id: 8,
            x: 12.0,
            y: 0.0,
        },
    ];
    let content = FragmentContent::TextLine {
        glyphs: glyphs.clone(),
        face: face.clone(),
        font_size: 16.0,
        color: Rgba::BLACK,
    };
    let f = Fragment::new(NodeId(3), 0.0, 0.0, 24.0, 18.0, content);
    match &f.content {
        FragmentContent::TextLine {
            glyphs, font_size, ..
        } => {
            assert_eq!(glyphs.len(), 2);
            assert_eq!(glyphs[1].x, 12.0);
            assert_eq!(*font_size, 16.0);
        }
        _ => panic!("expected text line"),
    }
}

#[test]
fn directive_content_roundtrips() {
    let f = Fragment::new(
        NodeId(9),
        0.0,
        0.0,
        0.0,
        0.0,
        FragmentContent::Directive(TKind::Footnote),
    );
    match f.content {
        FragmentContent::Directive(k) => assert_eq!(k, TKind::Footnote),
        _ => panic!("expected directive"),
    }
}

#[test]
fn node_id_default_is_zero() {
    assert_eq!(NodeId::default(), NodeId(0));
}
