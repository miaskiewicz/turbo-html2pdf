//! Complex real-site layout harnesses: class-driven styling, nested positioning,
//! and hide/show patterns as used on Nike / Wikipedia. Offline, through the real
//! engine. These pin down whether (a) global class rules apply and (b) the
//! visually-hidden / clipped / collapsed patterns are honored — the two things a
//! "pile of overlapping text" on those pages comes down to.

use turbo_html2pdf_core::FontRegistry;
use turbo_html2pdf_core::{layout_html, Diagnostics, Fragment, FragmentContent, Rgba};

fn lay(html: &str, w: f32) -> Fragment {
    let mut d = Diagnostics::default();
    layout_html(html, "", w, &FontRegistry::new(), &mut d).expect("layout")
}

fn text_line_count(f: &Fragment) -> usize {
    let mut n = 0;
    fn go(f: &Fragment, n: &mut usize) {
        if matches!(f.content, FragmentContent::TextLine { .. }) {
            *n += 1;
        }
        for c in &f.children {
            go(c, n);
        }
    }
    go(f, &mut n);
    n
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

// --------------------------------------------------------------------------
// (a) global class rules apply (the "maybe classes don't cascade" hypothesis)
// --------------------------------------------------------------------------

#[test]
fn class_rule_from_style_block_applies() {
    let f = lay(
        r#"<body><style>.box{width:50px;height:40px;background-color:#ff0000}</style>
           <div class="box"></div></body>"#,
        800.0,
    );
    let b = rect(&f, (255, 0, 0)).expect("class-styled box");
    assert!(
        (b[2] - 50.0).abs() < 1.0 && (b[3] - 40.0).abs() < 1.0,
        "class width/height applied, got {b:?}"
    );
}

#[test]
fn multi_class_element_gets_all_matching_rules() {
    let f = lay(
        r#"<body><style>.a{width:80px} .b{height:30px;background-color:#00ff00}</style>
           <div class="a b"></div></body>"#,
        800.0,
    );
    let b = rect(&f, (0, 255, 0)).expect("multi-class box");
    assert!(
        (b[2] - 80.0).abs() < 1.0,
        "class .a width applied, got w={}",
        b[2]
    );
    assert!(
        (b[3] - 30.0).abs() < 1.0,
        "class .b height applied, got h={}",
        b[3]
    );
}

// --------------------------------------------------------------------------
// (b) visually-hidden / sr-only patterns must NOT paint their text
// --------------------------------------------------------------------------

#[test]
fn sr_only_clip_rect_is_not_painted() {
    // The classic screen-reader-only pattern: absolute, 1px box, clipped away.
    // Wikipedia/Nike use dozens ("Jump to content" etc.). The text must not paint.
    let f = lay(
        r#"<body><span style="position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0,0,0,0)">HIDDEN A</span></body>"#,
        800.0,
    );
    assert_eq!(
        text_line_count(&f),
        0,
        "clip:rect(0,0,0,0) sr-only text not painted"
    );
}

#[test]
fn sr_only_clip_path_inset_is_not_painted() {
    let f = lay(
        r#"<body><span style="position:absolute;width:1px;height:1px;overflow:hidden;clip-path:inset(50%)">HIDDEN B</span></body>"#,
        800.0,
    );
    assert_eq!(
        text_line_count(&f),
        0,
        "clip-path:inset(50%) sr-only text not painted"
    );
}

#[test]
fn sr_only_via_class_not_painted() {
    // Same, but the pattern comes from a class rule (tests cascade + honoring).
    let f = lay(
        r#"<body><style>.sr-only{position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(1px,1px,1px,1px)}</style>
           <span class="sr-only">HIDDEN C</span></body>"#,
        800.0,
    );
    assert_eq!(text_line_count(&f), 0, "sr-only class text not painted");
}

#[test]
fn tiny_overflow_hidden_box_clips_its_text() {
    // width:1px/height:1px + overflow:hidden (no clip) — content is clipped away.
    let f = lay(
        r#"<body><div style="width:1px;height:1px;overflow:hidden"><span>HIDDEN D</span></div></body>"#,
        800.0,
    );
    assert_eq!(text_line_count(&f), 0, "1px overflow:hidden clips text");
}

// --------------------------------------------------------------------------
// nested positioning (timeline bars/labels, clade trees)
// --------------------------------------------------------------------------

#[test]
fn absolute_siblings_no_offset_share_cb_origin() {
    // Multiple absolute children with no offset legitimately overlap at the CB
    // origin — but only when their container is visible. Here they should sit at
    // the relative parent's origin (this is the taxobox-timeline shape).
    let f = lay(
        r#"<body><div style="position:relative;margin-left:300px;width:200px;height:20px">
             <div style="position:absolute;left:0;top:0;width:20px;height:20px;background-color:#ff0000"></div>
             <div style="position:absolute;left:40px;top:0;width:20px;height:20px;background-color:#00ff00"></div>
           </div></body>"#,
        800.0,
    );
    let a = rect(&f, (255, 0, 0)).expect("bar a");
    let b = rect(&f, (0, 255, 0)).expect("bar b");
    assert!(
        (a[0] - 300.0).abs() < 2.0,
        "bar a at parent origin, got x={}",
        a[0]
    );
    assert!(
        (b[0] - 340.0).abs() < 2.0,
        "bar b offset within parent, got x={}",
        b[0]
    );
}

#[test]
fn auto_inset_absolute_uses_static_position_not_cb_origin() {
    // An absolute element with NO top/left resolves to its STATIC position (where it
    // would sit in flow) — not the containing-block origin. Otherwise every
    // no-offset absolute deep in a page (navbox labels, decorations) jumps to the
    // top and piles up.
    let f = lay(
        r#"<body><div style="height:300px"></div>
           <div style="position:absolute;width:20px;height:20px;background-color:#ff0000"></div></body>"#,
        800.0,
    );
    let b = rect(&f, (255, 0, 0)).expect("absolute box");
    assert!(
        b[1] >= 290.0,
        "auto-inset absolute at its static flow y (~300), got y={}",
        b[1]
    );
}
