//! Offline layout harnesses for the Hacker News render patterns — link colour
//! overrides, full-width main table, and footer spacing. Same approach as the
//! Wikipedia harness: lay a small hand-authored fixture through the real engine
//! (no browser/network/addon) and assert geometry + colours, so an HN regression
//! fails here fast. Text colours are read off the laid-out `TextLine` fragments;
//! boxes are tagged with distinct background colours and found by colour.

use turbo_html2pdf_core::FontRegistry;
use turbo_html2pdf_core::{layout_html, Diagnostics, Fragment, FragmentContent, Rgba};

fn lay(html: &str, w: f32) -> Fragment {
    let mut d = Diagnostics::default();
    layout_html(html, "", w, &FontRegistry::new(), &mut d).expect("layout")
}

/// The colour of the first laid-out text line (document order).
fn first_text_color(f: &Fragment) -> Option<(u8, u8, u8)> {
    if let FragmentContent::TextLine { color, .. } = &f.content {
        return Some((color.r, color.g, color.b));
    }
    f.children.iter().find_map(first_text_color)
}

type ColoredRect = ((u8, u8, u8), [f32; 4]);

fn collect(f: &Fragment, out: &mut Vec<ColoredRect>) {
    if let FragmentContent::Box {
        background: Some(Rgba { r, g, b, a }),
        ..
    } = &f.content
    {
        if *a > 0 {
            out.push(((*r, *g, *b), [f.x, f.y, f.width, f.height]));
        }
    }
    for c in &f.children {
        collect(c, out);
    }
}

fn rect(f: &Fragment, rgb: (u8, u8, u8)) -> Option<[f32; 4]> {
    let mut v = Vec::new();
    collect(f, &mut v);
    v.into_iter().find(|(c, _)| *c == rgb).map(|(_, r)| r)
}

const BLACK: (u8, u8, u8) = (0, 0, 0);

// --------------------------------------------------------------------------
// link colour: an author `a:link` / `.class a` rule beats the UA blue
// --------------------------------------------------------------------------

#[test]
fn a_link_pseudo_color_overrides_ua_blue() {
    // HN colours story titles with `a:link { color:#000 }`. `:link` must match a
    // real link (an `<a href>`), so the title renders black, not UA blue.
    let html = r#"<body><style>a:link { color: #000000 }</style>
        <a href="https://example.com">Title</a></body>"#;
    let f = lay(html, 800.0);
    assert_eq!(
        first_text_color(&f),
        Some(BLACK),
        "a:link colour must win over the UA link blue"
    );
}

#[test]
fn class_descendant_color_overrides_ua_blue() {
    // A plain `.titleline a` rule (specificity beats UA `a`) must also apply.
    let html = r#"<body><style>.titleline a { color: #000000 }</style>
        <span class="titleline"><a href="page">Title</a></span></body>"#;
    let f = lay(html, 800.0);
    assert_eq!(
        first_text_color(&f),
        Some(BLACK),
        "class rule beats UA link blue"
    );
}

// --------------------------------------------------------------------------
// main table stretches to its declared width
// --------------------------------------------------------------------------

#[test]
fn table_width_percent_stretches_to_container() {
    // HN's `<table width="85%">` (and inner `width:100%` tables) must take their
    // declared share of the width, not shrink to content.
    let html = r#"<body>
        <table width="100%"><tbody><tr>
            <td style="background-color:#ff0000">a</td>
        </tr></tbody></table></body>"#;
    let f = lay(html, 800.0);
    let cell = rect(&f, (255, 0, 0)).expect("cell");
    assert!(
        cell[2] > 700.0,
        "width:100% table cell fills the row, got {}",
        cell[2]
    );
}

#[test]
fn percent_width_table_column_fills_not_double_applied() {
    // A `width:85%` table's column must fill the table width, not collapse to 85%
    // of 85% — HN's `<table width="85%">` whose footer `<table width="100%">` must
    // span the full main column, not ~72% of the viewport.
    let html = r#"<body><table width="85%"><tbody>
            <tr><td>short</td></tr>
            <tr><td><table width="100%"><tbody><tr>
                <td style="background-color:#ff0000">footer</td>
            </tr></tbody></table></td></tr>
        </tbody></table></body>"#;
    let f = lay(html, 1280.0);
    let footer = rect(&f, (255, 0, 0)).expect("footer cell");
    // 85% of 1280 = 1088; the inner 100% cell must be ~that, not 0.85*1088=925.
    assert!(
        footer[2] > 1050.0,
        "85% table column fills (want ~1088), got {}",
        footer[2]
    );
}

#[test]
fn nested_full_width_table_fills_its_cell() {
    // HN nests its item list table inside an 85%-wide outer table; the inner table
    // (width:100%) must fill the outer cell rather than shrink to content.
    let html = r#"<body>
        <table width="600"><tbody><tr><td>
            <table width="100%"><tbody><tr>
                <td style="background-color:#00ff00">item</td>
            </tr></tbody></table>
        </td></tr></tbody></table></body>"#;
    let f = lay(html, 1000.0);
    let inner = rect(&f, (0, 255, 0)).expect("inner cell");
    assert!(
        inner[2] > 500.0,
        "inner width:100% table fills outer 600px cell, got {}",
        inner[2]
    );
}

// --------------------------------------------------------------------------
// footer spacing: <br><br> makes vertical gaps
// --------------------------------------------------------------------------

#[test]
fn double_br_creates_vertical_gap() {
    // HN's footer separates blocks with `<br><br>`. Two line breaks must push the
    // following content down by roughly two line heights.
    let html = r#"<body>
        <div style="background-color:#ff0000;height:10px">top</div>
        <br><br>
        <div style="background-color:#0000ff;height:10px">bottom</div>
      </body>"#;
    let f = lay(html, 800.0);
    let top = rect(&f, (255, 0, 0)).expect("top");
    let bottom = rect(&f, (0, 0, 255)).expect("bottom");
    let gap = bottom[1] - (top[1] + top[3]);
    assert!(gap > 15.0, "two <br> create a multi-line gap, got {}", gap);
}

// --------------------------------------------------------------------------
// nav logo: a sized <img> lays out at the top-left of the header
// --------------------------------------------------------------------------

#[test]
fn sized_img_lays_out_at_top_left_at_its_size() {
    // HN's `<img src="y18.svg" width=18 height=18>` logo. Even with no image bytes
    // (image-free fallback) the box must occupy its declared 18x18 at the top-left,
    // so the logo has a slot to paint into.
    let html = r#"<body><table><tbody><tr>
            <td><img src="y18.svg" width="18" height="18" style="background-color:#ff0000"></td>
            <td><b>Hacker News</b></td>
        </tr></tbody></table></body>"#;
    let f = lay(html, 1280.0);
    let logo = rect(&f, (255, 0, 0)).expect("logo box");
    assert!(
        logo[0] < 5.0 && logo[1] < 5.0,
        "logo at top-left, got x={} y={}",
        logo[0],
        logo[1]
    );
    assert!(
        (logo[2] - 18.0).abs() < 1.0,
        "logo 18px wide, got {}",
        logo[2]
    );
    assert!(
        (logo[3] - 18.0).abs() < 1.0,
        "logo 18px tall, got {}",
        logo[3]
    );
}
