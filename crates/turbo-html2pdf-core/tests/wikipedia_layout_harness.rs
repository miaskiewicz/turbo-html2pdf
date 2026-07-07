//! Offline layout harnesses for the page patterns that turbo-surf's Wikipedia
//! screenshot exercises — menus, top-down page flow, and the 3-column Vector
//! grid. Each test lays a small hand-authored HTML/CSS fixture out through the
//! real engine (no browser, no network, no addon rebuild) and asserts geometry,
//! so a layout regression fails here fast instead of only showing up as a garbled
//! screenshot. Elements under test are tagged with distinct `background-color`s so
//! their fragments can be found by colour and their rectangles asserted.

use turbo_html2pdf_core::FontRegistry;
use turbo_html2pdf_core::{layout_html, Diagnostics, Fragment, FragmentContent, Rgba};

/// Lay `html` out at content width `w` over the bundled fonts.
fn lay(html: &str, w: f32) -> Fragment {
    let mut d = Diagnostics::default();
    layout_html(html, "", w, &FontRegistry::new(), &mut d).expect("layout")
}

/// A painted box: its `(r, g, b)` background and `[x, y, w, h]` rectangle.
type ColoredRect = ((u8, u8, u8), [f32; 4]);

/// Every painted box rectangle `[x, y, w, h]` keyed by its background colour.
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

/// The rectangle of the (first) box with background colour `rgb`, if painted.
fn rect(f: &Fragment, rgb: (u8, u8, u8)) -> Option<[f32; 4]> {
    let mut v = Vec::new();
    collect(f, &mut v);
    v.into_iter().find(|(c, _)| *c == rgb).map(|(_, r)| r)
}

const RED: (u8, u8, u8) = (255, 0, 0);
const GREEN: (u8, u8, u8) = (0, 255, 0);
const BLUE: (u8, u8, u8) = (0, 0, 255);

/// Every rendered text line's rectangle `[x, y, w, h]`. A normal block box spans
/// the full column; a float only shortens the *line boxes* inside it, so float
/// wrapping is asserted against these, not the (full-width) paragraph box.
fn text_lines(f: &Fragment, out: &mut Vec<[f32; 4]>) {
    if matches!(f.content, FragmentContent::TextLine { .. }) {
        out.push([f.x, f.y, f.width, f.height]);
    }
    for c in &f.children {
        text_lines(c, out);
    }
}

/// The article text lines wrapping beside a `float:right` box: those starting left
/// of `float[x]` whose top is above the float's bottom. Excludes the float's own
/// text (which starts at/after the float's left edge).
fn lines_beside_right_float(f: &Fragment, float: [f32; 4]) -> Vec<[f32; 4]> {
    let mut v = Vec::new();
    text_lines(f, &mut v);
    v.retain(|l| l[1] < float[1] + float[3] - 1.0 && l[0] < float[0] - 1.0);
    v
}

// --------------------------------------------------------------------------
// harness 1: menus (dropdowns hidden at rest)
// --------------------------------------------------------------------------

#[test]
fn hidden_dropdown_menu_does_not_paint() {
    // Vector's dropdowns: content hidden by default, revealed only by a checked
    // toggle (`input:checked ~ .content`). At rest (unchecked) it must not paint.
    let html = r#"<body><style>
        .menu { visibility: hidden }
        input:checked ~ .menu { visibility: visible }
      </style>
      <div>
        <input type="checkbox">
        <div class="menu" style="background-color:#ff0000;height:50px">MENU</div>
      </div></body>"#;
    let f = lay(html, 800.0);
    assert!(
        rect(&f, RED).is_none(),
        "unchecked dropdown content must stay hidden"
    );
}

#[test]
fn checked_toggle_reveals_menu() {
    // The same markup with the box checked *does* reveal the menu.
    let html = r#"<body><style>
        .menu { visibility: hidden }
        input:checked ~ .menu { visibility: visible }
      </style>
      <div>
        <input type="checkbox" checked>
        <div class="menu" style="background-color:#ff0000;height:50px">MENU</div>
      </div></body>"#;
    let f = lay(html, 800.0);
    assert!(rect(&f, RED).is_some(), "checked dropdown content shows");
}

#[test]
fn display_none_menu_does_not_paint() {
    let html = r#"<body>
        <div style="display:none;background-color:#ff0000;height:40px">hidden</div>
        <div style="background-color:#00ff00;height:40px">shown</div>
      </body>"#;
    let f = lay(html, 800.0);
    assert!(rect(&f, RED).is_none(), "display:none dropped");
    assert!(rect(&f, GREEN).is_some(), "sibling still shown");
}

// --------------------------------------------------------------------------
// harness 2: top-down page flow (no overlap)
// --------------------------------------------------------------------------

#[test]
fn page_sections_stack_without_overlap() {
    let html = r#"<body>
        <div style="background-color:#ff0000;height:40px">header</div>
        <div style="background-color:#00ff00;height:60px">content</div>
        <div style="background-color:#0000ff;height:30px">footer</div>
      </body>"#;
    let f = lay(html, 800.0);
    let h = rect(&f, RED).expect("header");
    let c = rect(&f, GREEN).expect("content");
    let ft = rect(&f, BLUE).expect("footer");
    assert!(c[1] >= h[1] + h[3] - 0.5, "content starts below header");
    assert!(ft[1] >= c[1] + c[3] - 0.5, "footer starts below content");
}

// --------------------------------------------------------------------------
// harness 3: 3-column grid (Vector: sidebar | body | rail)
// --------------------------------------------------------------------------

#[test]
fn three_column_grid_places_columns_side_by_side() {
    let html = r#"<body><style>
        .page { display: grid; grid-template-columns: 200px 1fr 150px }
        .l { background-color:#ff0000; height:100px }
        .m { background-color:#00ff00; height:100px }
        .r { background-color:#0000ff; height:100px }
      </style>
      <div class="page">
        <div class="l"></div><div class="m"></div><div class="r"></div>
      </div></body>"#;
    let f = lay(html, 900.0);
    let l = rect(&f, RED).expect("left col");
    let m = rect(&f, GREEN).expect("middle col");
    let r = rect(&f, BLUE).expect("right col");
    // Same row.
    assert!(
        (l[1] - m[1]).abs() < 1.0 && (m[1] - r[1]).abs() < 1.0,
        "columns share a row"
    );
    // Left→middle→right, no overlap.
    assert!(
        m[0] >= l[0] + l[2] - 1.0,
        "middle right of left (l end {}, m start {})",
        l[0] + l[2],
        m[0]
    );
    assert!(
        r[0] >= m[0] + m[2] - 1.0,
        "right col right of middle (m end {}, r start {})",
        m[0] + m[2],
        r[0]
    );
    // Fixed sidebars keep their track widths; the middle takes the rest.
    assert!((l[2] - 200.0).abs() < 2.0, "left track 200px, got {}", l[2]);
    assert!(
        (r[2] - 150.0).abs() < 2.0,
        "right track 150px, got {}",
        r[2]
    );
}

// --------------------------------------------------------------------------
// harness 4: navbox (nested-table taxonomy box must not overlap itself)
// --------------------------------------------------------------------------

/// A cut-down Wikipedia navbox (the Carnivora/taxonomy box): a `<table class=
/// navbox>` whose cells nest further `navbox-subgroup` tables. This is the
/// structure that rendered as an overlapping pile in the Cat article. The title,
/// a top-level group cell, and the deeply-nested leaf cell must stack top-down
/// without landing on top of each other.
const NAVBOX: &str = r#"<body><style>
    table { border-collapse: collapse }
    .navbox { width: 100%; border: 1px solid #a2a9b1 }
    .navbox-inner, .navbox-subgroup { width: 100% }
    .navbox-group { white-space: nowrap; text-align: right; background-color: #ff0000 }
    .navbox-list { line-height: 1.5em; text-align: left }
    .navbox-title { background-color: #ccccff }
  </style>
  <table class="navbox"><tbody>
    <tr><th class="navbox-title" colspan="2">Carnivora</th></tr>
    <tr>
      <th class="navbox-group">Feliformia</th>
      <td class="navbox-list navbox-odd"><div>
        <table class="navbox-subgroup"><tbody>
          <tr>
            <th class="navbox-group">Felidae</th>
            <td class="navbox-list"><div style="background-color:#00ff00;height:24px">Panthera</div></td>
          </tr>
          <tr>
            <th class="navbox-group">Herpestidae</th>
            <td class="navbox-list"><div style="background-color:#0000ff;height:24px">Mongoose</div></td>
          </tr>
        </tbody></table>
      </div></td>
    </tr>
  </tbody></table></body>"#;

#[test]
fn navbox_rows_stack_without_overlap() {
    let f = lay(NAVBOX, 800.0);
    let title = rect(&f, (204, 204, 255)).expect("navbox title");
    let leaf1 = rect(&f, GREEN).expect("nested leaf 1");
    let leaf2 = rect(&f, BLUE).expect("nested leaf 2");
    // The nested leaves sit below the title, not piled on top of it.
    assert!(
        leaf1[1] >= title[1] + title[3] - 0.5,
        "nested content below title (title bottom {}, leaf1 top {})",
        title[1] + title[3],
        leaf1[1]
    );
    // The two nested rows don't overlap each other.
    assert!(
        leaf2[1] >= leaf1[1] + leaf1[3] - 0.5,
        "nested rows stack (leaf1 bottom {}, leaf2 top {})",
        leaf1[1] + leaf1[3],
        leaf2[1]
    );
}

#[test]
fn navbox_nested_subgroup_has_height() {
    // The whole navbox must be at least as tall as its title + two nested rows —
    // if the nested subgroup table collapses to zero, everything piles up.
    let f = lay(NAVBOX, 800.0);
    assert!(
        f.bottom() >= 24.0 + 24.0,
        "navbox taller than its nested rows, got {}",
        f.bottom()
    );
}

// --------------------------------------------------------------------------
// harness 5: grid-template shorthand (Vector's main 2-column layout)
// --------------------------------------------------------------------------

#[test]
fn grid_template_shorthand_sets_fixed_tracks() {
    // Vector lays its page out with `grid-template: <rows> / <cols>` + named areas
    // (`grid-template: min-content 1fr / 12.25rem minmax(0,1fr)`). The shorthand
    // must yield the column tracks, so the fixed sidebar column stays fixed. Before
    // the fix the axis fell back to AUTO tracks which, with named areas, made taffy
    // content-size the whole subtree per track — a pathological hang.
    let html = r#"<body><style>
        .page { display: grid; grid-template: min-content 1fr / 200px minmax(0,1fr);
                grid-template-areas: 'head head' 'side main' }
        .side { grid-area: side; background-color:#ff0000; height:50px }
        .main { grid-area: main; background-color:#00ff00; height:50px }
      </style>
      <div class="page"><div class="side"></div><div class="main"></div></div></body>"#;
    let f = lay(html, 1000.0);
    let side = rect(&f, RED).expect("sidebar");
    let main = rect(&f, GREEN).expect("main");
    assert!(
        (side[2] - 200.0).abs() < 3.0,
        "fixed sidebar track = 200px, got {}",
        side[2]
    );
    // main sits to the right of the sidebar (2-column), same row.
    assert!(main[0] >= side[0] + side[2] - 1.0, "main right of sidebar");
    assert!(
        (side[1] - main[1]).abs() < 2.0,
        "sidebar + main share the row"
    );
}

// --------------------------------------------------------------------------
// harness 6: full Vector 3-column (Contents | main | Appearance) + min-content
// --------------------------------------------------------------------------

#[test]
fn nested_vector_three_columns_place_side_by_side() {
    // Vector nests two grids: outer `columnStart | pageContent`, and inside
    // pageContent an `.mw-body` grid `content | columnEnd`. The three visible
    // columns (Contents / main / Appearance) must sit left-to-right.
    let html = r#"<body><style>
        .inner { display: grid;
                 grid-template: min-content 1fr / 12.25rem minmax(0,1fr);
                 grid-template-areas: 'siteNotice siteNotice' 'columnStart pageContent' }
        .start { grid-area: columnStart; background-color:#ff0000; height:60px }
        .page  { grid-area: pageContent }
        .body  { display: grid; grid-template: 1fr / minmax(0,59.25rem) min-content;
                 grid-template-areas: 'content columnEnd' }
        .content { grid-area: content; background-color:#00ff00; height:60px }
        .end   { grid-area: columnEnd; background-color:#0000ff; height:60px }
      </style>
      <div class="inner">
        <div class="start">Contents</div>
        <div class="page"><div class="body">
          <div class="content">Main</div>
          <div class="end">Appearance settings</div>
        </div></div>
      </div></body>"#;
    let f = lay(html, 1280.0);
    let c = rect(&f, RED).expect("Contents");
    let m = rect(&f, GREEN).expect("Main");
    let a = rect(&f, BLUE).expect("Appearance");
    assert!(
        m[0] >= c[0] + c[2] - 1.0,
        "Main right of Contents (c end {}, m start {})",
        c[0] + c[2],
        m[0]
    );
    assert!(
        a[0] >= m[0] + m[2] - 1.0,
        "Appearance right of Main (m end {}, a start {})",
        m[0] + m[2],
        a[0]
    );
    assert!(
        (c[2] - 196.0).abs() < 3.0,
        "Contents ~12.25rem, got {}",
        c[2]
    );
    assert!(
        a[2] > 20.0,
        "Appearance (min-content) sized to its text, got {}",
        a[2]
    );
}

// --------------------------------------------------------------------------
// harness 7: float text wrap (infobox/taxobox with text beside it)
// --------------------------------------------------------------------------

#[test]
fn text_wraps_beside_a_right_float() {
    // Wikipedia's taxobox is `float:right; width:22em`; the intro paragraphs must
    // flow in the narrowed column to its LEFT (beside it), not stack below it.
    let html = r#"<body>
        <div style="float:right;width:300px;height:200px;background-color:#ff0000"></div>
        <p style="background-color:#00ff00">The cat is a small domesticated carnivorous mammal and a member of Felidae.</p>
      </body>"#;
    let f = lay(html, 1000.0);
    let float_box = rect(&f, RED).expect("float");
    // The paragraph box spans the full column (CSS: a float shortens line boxes,
    // not the block); assert the *text lines* beside the float stay left of it.
    let beside = lines_beside_right_float(&f, float_box);
    assert!(
        !beside.is_empty(),
        "text flows beside the float, not below it"
    );
    for l in &beside {
        assert!(
            l[0] + l[2] <= float_box[0] + 1.0,
            "line stays left of the float (line end {}, float x {})",
            l[0] + l[2],
            float_box[0],
        );
    }
}

#[test]
fn infobox_floats_right_via_no_space_media_query() {
    // Wikipedia floats its taxobox with `.mw-parser-output .infobox{float:right}`
    // inside `@media(min-width:640px)` (no space). At desktop width the infobox
    // must float right with the article text wrapping to its left.
    let css = "@media(min-width:640px){.mw-parser-output .infobox{float:right;width:300px}}";
    let html = r#"<body><div class="mw-parser-output">
        <table class="infobox" style="background-color:#ff0000"><tbody><tr><td style="height:200px">Cat</td></tr></tbody></table>
        <p style="background-color:#00ff00">The cat is a small domesticated carnivorous mammal member of Felidae with lots and lots and lots of words here to fill several lines beside the tall infobox</p>
      </div></body>"#;
    let mut d = Diagnostics::default();
    let f = layout_html(html, css, 1000.0, &FontRegistry::new(), &mut d).expect("layout");
    let info = rect(&f, RED).expect("infobox");
    assert!(info[0] > 500.0, "infobox floats right, got x={}", info[0]);
    let beside = lines_beside_right_float(&f, info);
    assert!(!beside.is_empty(), "article text wraps beside the infobox");
    for l in &beside {
        assert!(l[0] + l[2] <= info[0] + 1.0, "text wraps left of infobox");
    }
}

// --------------------------------------------------------------------------
// harness 8: float escapes its section — text in a LATER sibling section wraps
// --------------------------------------------------------------------------

#[test]
fn float_wraps_content_in_a_later_sibling_section() {
    // The Wikipedia bug: the taxobox (`float:right`) lives in the lead `<section>`,
    // but the following section (Etymology) is a *sibling* block. A float belongs
    // to its block formatting context, not to the section it sits in — so the later
    // section's text must wrap in the narrowed column beside the float, not clear
    // below it. Before the BFC-scoped float fix the float was trapped in section 0
    // (which grew to contain it), so section 1 stacked full-width below the box.
    let html = r#"<body><div class="mw-parser-output">
        <section>
          <div style="float:right;width:300px;height:600px;background-color:#ff0000"></div>
          <p>Lead paragraph one with a handful of words to fill a line.</p>
        </section>
        <section>
          <p style="background-color:#00ff00">Etymology text that must wrap beside the tall floated infobox above.</p>
        </section>
      </div></body>"#;
    let f = lay(html, 1000.0);
    let float_box = rect(&f, RED).expect("float");
    // The later section's text lines beside the float must sit left of it.
    let beside = lines_beside_right_float(&f, float_box);
    assert!(
        !beside.is_empty(),
        "later section flows beside the float, not below it",
    );
    for l in &beside {
        assert!(
            l[0] + l[2] <= float_box[0] + 1.0,
            "later-section line stays left of the float (line end {}, float x {})",
            l[0] + l[2],
            float_box[0],
        );
    }
}

#[test]
fn bfc_container_contains_its_float_but_a_plain_block_does_not() {
    // A block that establishes a BFC (`overflow:hidden`) grows to contain a tall
    // float inside it, so following content clears below the whole box. A plain
    // block does not — the float escapes it (asserted above). Here the floated box
    // is taller than the text; the BFC wrapper must be at least the float's height.
    let html = r#"<body>
        <div style="overflow:hidden;background-color:#0000ff">
          <div style="float:left;width:100px;height:400px;background-color:#ff0000"></div>
          <p>short</p>
        </div>
      </body>"#;
    let f = lay(html, 1000.0);
    let wrapper = rect(&f, BLUE).expect("bfc wrapper");
    let float_box = rect(&f, RED).expect("float");
    assert!(
        wrapper[3] >= float_box[3] - 1.0,
        "BFC wrapper contains the float (wrapper h {}, float h {})",
        wrapper[3],
        float_box[3],
    );
}

#[test]
fn paragraph_lines_widen_below_the_float() {
    // Per-line wrapping: one long paragraph beside a short `float:right` box. Lines
    // whose top is above the float bottom wrap in the narrow column (stay left of
    // the box); lines below it use the full width and extend past the box's left
    // edge. Block-level narrowing kept the whole paragraph narrow (the bug the
    // Wikipedia screenshot showed below the taxobox).
    let words = "cat ".repeat(200);
    let html = format!(
        r#"<body>
        <div style="float:right;width:300px;height:80px;background-color:#ff0000"></div>
        <p>{words}</p>
      </body>"#
    );
    let f = lay(&html, 1000.0);
    let float_box = rect(&f, RED).expect("float");
    let float_left = float_box[0];
    let float_bottom = float_box[1] + float_box[3];
    let mut lines = Vec::new();
    text_lines(&f, &mut lines);
    let above: Vec<_> = lines.iter().filter(|l| l[1] < float_bottom - 1.0).collect();
    let below: Vec<_> = lines.iter().filter(|l| l[1] > float_bottom + 1.0).collect();
    assert!(
        !above.is_empty() && !below.is_empty(),
        "paragraph spans the float bottom"
    );
    for l in &above {
        assert!(
            l[0] + l[2] <= float_left + 1.0,
            "line beside the float stays left of it (end {}, float x {})",
            l[0] + l[2],
            float_left,
        );
    }
    assert!(
        below.iter().any(|l| l[0] + l[2] > float_left + 50.0),
        "lines below the float widen past its left edge (full width)",
    );
}

// --------------------------------------------------------------------------
// harness 9: skin-chrome gating — @supports, html/body-class ancestors, calc()
// media. Together these un-hide Vector's sidebar TOC + appearance panel.
// --------------------------------------------------------------------------

#[test]
fn html_class_gates_descendant_rule() {
    // Real skins gate huge amounts of CSS on `<html>`/`<body>` classes (Wikipedia's
    // `vector-feature-*`, `client-js`, theme classes). Those elements aren't laid
    // out, but the cascade seeds them as match-only ancestors, so a descendant rule
    // keyed on the html class must still apply.
    let html = r#"<html class="feat"><body><style>
        .box{display:none}
        .feat .box{display:block;background-color:#ff0000;height:20px}
      </style><div class="box">x</div></body></html>"#;
    let f = lay(html, 800.0);
    assert!(
        rect(&f, RED).is_some(),
        "html-class-gated rule un-hides the box"
    );
}

#[test]
fn supports_grid_applies_and_negation_dropped() {
    // `@supports (display:grid)` is satisfied (we render like a modern browser), so
    // its body applies; the `not(display:grid)` legacy fallback stays dropped. This
    // is exactly how Vector un-hides `.vector-pinned-container` (the sidebar TOC +
    // appearance panel live inside it).
    let html = r#"<body><style>
        .b{display:none}
        @supports (display:grid){ .b{display:block;background-color:#ff0000;height:20px} }
        @supports not (display:grid){ .c{background-color:#0000ff;height:20px} }
      </style><div class="b">x</div><div class="c">y</div></body>"#;
    let f = lay(html, 800.0);
    assert!(
        rect(&f, RED).is_some(),
        "@supports (display:grid) body applies"
    );
    assert!(
        rect(&f, BLUE).is_none(),
        "@supports not(display:grid) fallback stays dropped"
    );
}

#[test]
fn media_max_width_calc_breakpoint() {
    // `@media (max-width: calc(1120px - 1px))` is a 1119px breakpoint: excluded at
    // 1280 (desktop), applied at 1000 (mobile). A calc() that failed to parse would
    // make the block always-match — wrongly hiding the desktop sidebar columns.
    let css =
        "@media screen and (max-width:calc(1120px - 1px)){.m{background-color:#ff0000;height:20px}}";
    let html = r#"<body><div class="m">x</div></body>"#;
    let mut d = Diagnostics::default();
    let wide = layout_html(html, css, 1280.0, &FontRegistry::new(), &mut d).expect("layout");
    let narrow = layout_html(html, css, 1000.0, &FontRegistry::new(), &mut d).expect("layout");
    assert!(
        rect(&wide, RED).is_none(),
        "mobile @media block excluded at desktop width (calc breakpoint parsed)"
    );
    assert!(
        rect(&narrow, RED).is_some(),
        "mobile @media block applies below the 1120px breakpoint"
    );
}
