//! Offline harnesses for absolute-positioning containing-block resolution — the
//! behavior Wikipedia's taxobox timeline + cladogram rely on (absolute bars/labels
//! anchored to a `position:relative` ancestor, moved with it). Each lays a small
//! fixture through the real engine and asserts the absolute box lands on its
//! nearest positioned ancestor, not an outer one.

use turbo_html2pdf_core::FontRegistry;
use turbo_html2pdf_core::{layout_html, Diagnostics, Fragment, FragmentContent, Rgba};

fn lay(html: &str, w: f32) -> Fragment {
    let mut d = Diagnostics::default();
    layout_html(html, "", w, &FontRegistry::new(), &mut d).expect("layout")
}

fn rect(f: &Fragment, rgb: (u8, u8, u8)) -> Option<[f32; 4]> {
    // This fragment's own filled-box rect, if it paints exactly color `t`.
    fn own_rect(f: &Fragment, t: (u8, u8, u8)) -> Option<[f32; 4]> {
        let FragmentContent::Box {
            background: Some(Rgba { r, g, b, a }),
            ..
        } = &f.content
        else {
            return None;
        };
        (*a > 0 && (*r, *g, *b) == t).then_some([f.x, f.y, f.width, f.height])
    }
    fn go(f: &Fragment, t: (u8, u8, u8), o: &mut Option<[f32; 4]>) {
        // Pre-order, first match wins: once `o` is set it is never overwritten.
        if o.is_none() {
            *o = own_rect(f, t);
        }
        for c in &f.children {
            go(c, t, o);
        }
    }
    let mut o = None;
    go(f, rgb, &mut o);
    o
}

const RED: (u8, u8, u8) = (255, 0, 0);
const GREEN: (u8, u8, u8) = (0, 255, 0);

/// Assert an absolute child sits on its relative parent (`px`,`py`), not escaped.
fn assert_anchored(html: &str, parent: (u8, u8, u8)) {
    let f = lay(html, 1000.0);
    let p = rect(&f, parent).expect("relative parent");
    let c = rect(&f, RED).expect("absolute child");
    assert!(
        (c[0] - p[0]).abs() < 40.0 && (c[1] - p[1]).abs() < 40.0,
        "absolute child anchors to its relative ancestor (parent {:?}, child {:?})",
        p,
        c
    );
}

#[test]
fn absolute_anchors_to_relative_block() {
    assert_anchored(
        r#"<body><div style="height:100px"></div>
            <div style="position:relative;margin-left:400px;width:200px;height:50px;background-color:#00ff00">
              <div style="position:absolute;left:0;top:0;width:40px;height:40px;background-color:#ff0000"></div>
            </div></body>"#,
        GREEN,
    );
}

#[test]
fn absolute_anchors_to_relative_inside_inline_block() {
    assert_anchored(
        r#"<body><span style="display:inline-block">
            <div style="position:relative;width:150px;height:40px;background-color:#00ff00">
              <div style="position:absolute;left:0;top:0;width:30px;height:30px;background-color:#ff0000"></div>
            </div></span></body>"#,
        GREEN,
    );
}

#[test]
fn absolute_anchors_to_relative_table_cell() {
    // The cell is laid at origin then translated to its grid slot — the absolute
    // child must move with it (Wikipedia's clade bars are `position:relative` tds).
    assert_anchored(
        r#"<body><table style="margin-left:400px"><tbody><tr>
            <td style="position:relative;width:200px;height:40px;background-color:#00ff00">
              <div style="position:absolute;left:10px;top:5px;width:20px;height:20px;background-color:#ff0000"></div>
            </td></tr></tbody></table></body>"#,
        GREEN,
    );
}

#[test]
fn absolute_anchors_to_relative_inside_float() {
    // Wikipedia's taxobox floats right; its timeline bars are absolute inside a
    // relative row within that float.
    assert_anchored(
        r#"<body><div style="float:right;width:250px;background-color:#cccccc">
            <div style="position:relative;width:200px;height:40px;background-color:#00ff00">
              <div style="position:absolute;left:0;top:0;width:30px;height:30px;background-color:#ff0000"></div>
            </div></div><p>body text body text body text</p></body>"#,
        GREEN,
    );
}

#[test]
fn nested_absolute_anchors_to_absolute_ancestor() {
    // A label absolutely positioned inside an absolutely positioned bar anchors to
    // the bar (the bar is itself a positioned containing block).
    let f = lay(
        r#"<body><div style="height:60px"></div>
            <div style="position:relative;margin-left:400px;width:250px;height:20px;background-color:#0000ff">
              <div style="position:absolute;left:50px;top:0;width:100px;height:20px;background-color:#00ff00">
                <div style="position:absolute;left:0;top:0;width:20px;height:20px;background-color:#ff0000"></div>
              </div>
            </div></body>"#,
        1000.0,
    );
    let bar = rect(&f, GREEN).expect("bar");
    let label = rect(&f, RED).expect("label");
    assert!(
        (label[0] - bar[0]).abs() < 2.0,
        "nested absolute label anchors to its absolute bar (bar x {}, label x {})",
        bar[0],
        label[0]
    );
}
