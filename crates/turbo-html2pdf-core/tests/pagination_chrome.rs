//! miaskiewicz/turbo-html2pdf#31: paginated box geometry against Chrome 156 `page.pdf()`.
//!
//! The control is `tests/fixtures/pagination_chrome156.json` (`scripts/pagination-chrome-
//! control.mjs`): text-free documents of solid-colour blocks on `600px x 400px` pages with
//! 40px margins, printed by Chrome for Testing 156 and rasterized at 96 dpi; per page and
//! colour, the inclusive device-pixel bounding box of that colour and whether its first /
//! last row is a full edge. Here the same documents run through `render_pages`, and the
//! same per-page boxes are measured on the page fragments painted into a 1 px/CSS px
//! buffer (backgrounds and border sides of `Box` fragments, in page coordinates).
//!
//! What it pins: a block's top margin is kept on page 1 and after a forced break and
//! truncated after an unforced one (CSS Fragmentation §5.2), and `<html>`/`<body>` borders
//! and backgrounds paint on every page fragment, sliced (`box-decoration-break: slice`:
//! the top edge on the first page, the bottom edge on the last, the box running to the
//! page area's end in between).
//!
//! miaskiewicz/turbo-html2pdf#32 extends it: fixed boxes repeat on every page and absolute
//! boxes sit relative to their containing block; the canvas (`Page::canvas`, painted here
//! over the page area as the emitter paints it) covers the page area only; a forced break
//! before the first content makes no blank page; a split block keeps its top border and
//! padding at a page start; after a forced break only the next block's own top margin is
//! kept; and a block that exactly fills a page moves on with its ancestors' bottom padding.
//! Fragments paint in their parent's paint order (`Fragment::paint_order`, positioned
//! boxes over the flow), as the emitter paints them.

mod common;

use std::collections::BTreeMap;

use turbo_html2pdf_core::layout::fragment::{Fragment, FragmentContent};
use turbo_html2pdf_core::layout::value::Rgba;
use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, compile, render_pages, CompileOptions, Diagnostics, Page, RenderInputs,
};

fn pages(html: &str) -> Vec<Page> {
    let (program, _) = compile(html, &CompileOptions::default()).expect("compile");
    let cascade = build_cascade("", "", TokenSet::default());
    let fonts = common::registry();
    let data = serde_json::Value::Null;
    let inputs = RenderInputs {
        program: &program,
        data: &data,
        cascade: &cascade,
        at_rules: &[],
        fonts: &fonts,
        images: &turbo_html2pdf_core::NoImages,
        now: Some(0),
    };
    render_pages(&inputs, &mut Diagnostics::default()).expect("render_pages")
}

/// Paint the page's `Box` fragments (background, then the four border sides; parents
/// before children, in list order — the paint order of these flat test documents) into a
/// `width x height` RGB buffer at 1 px per CSS px, white paper.
fn rasterize(page: &Page) -> (usize, usize, Vec<[u8; 3]>) {
    let (w, h) = (page.geometry.width as usize, page.geometry.height as usize);
    let mut buf = vec![[255u8; 3]; w * h];
    fn fill(buf: &mut [[u8; 3]], w: usize, (x, y, rw, rh): (f32, f32, f32, f32), c: Rgba) {
        let h = buf.len() / w;
        let span = |a: f32, len: f32, max: usize| {
            (
                (a.round().max(0.0) as usize).min(max),
                ((a + len).round().max(0.0) as usize).min(max),
            )
        };
        let ((x0, x1), (y0, y1)) = (span(x, rw, w), span(y, rh, h));
        for row in buf.chunks_mut(w).take(y1).skip(y0) {
            for px in &mut row[x0..x1] {
                *px = [c.r, c.g, c.b];
            }
        }
    }
    fn walk(buf: &mut [[u8; 3]], w: usize, frags: &[Fragment]) {
        for f in frags {
            for (rect, c) in box_paint(f) {
                fill(buf, w, rect, c);
            }
            let kids: Vec<Fragment> = f
                .paint_order()
                .into_iter()
                .map(|i| f.children[i].clone())
                .collect();
            walk(buf, w, &kids);
        }
    }
    if let Some(c) = page.canvas {
        let g = &page.geometry;
        let area = (
            g.margin.left,
            g.margin.top,
            g.width - g.margin.horizontal(),
            g.height - g.margin.vertical(),
        );
        fill(&mut buf, w, area, c);
    }
    walk(&mut buf, w, &in_paint_order(&page.body));
    (w, h, buf)
}

/// A page body's top-level fragments in paint order (as `Fragment::paint_order` orders a
/// parent's children).
fn in_paint_order(body: &[Fragment]) -> Vec<Fragment> {
    let mut sorted = body.to_vec();
    sorted.sort_by_key(|f| (f.paint_z(), f.is_positioned));
    sorted
}

type Rect = (f32, f32, f32, f32);

/// The opaque rectangles a `Box` fragment paints: its background, then its border sides.
fn box_paint(f: &Fragment) -> Vec<(Rect, Rgba)> {
    let FragmentContent::Box {
        background, border, ..
    } = &f.content
    else {
        return Vec::new();
    };
    let (x, y, fw, fh) = (f.x, f.y, f.width, f.height);
    let (t, b) = (f32::from(border.top.width), f32::from(border.bottom.width));
    let (l, r) = (f32::from(border.left.width), f32::from(border.right.width));
    let sides = [
        (border.top, (x, y, fw, t)),
        (border.bottom, (x, y + fh - b, fw, b)),
        (border.left, (x, y, l, fh)),
        (border.right, (x + fw - r, y, r, fh)),
    ];
    let fill = background.map(|c| ((x, y, fw, fh), c));
    let edges = sides
        .into_iter()
        .filter(|(side, _)| side.width > 0)
        .filter_map(|(side, rect)| side.color.map(|c| (rect, c)));
    fill.into_iter()
        .chain(edges)
        .filter(|(_, c)| c.a == 255)
        .collect()
}

fn rgb(css: &str) -> [u8; 3] {
    let v = |i: usize| u8::from_str_radix(&css[i..i + 2], 16).unwrap();
    [v(1), v(3), v(5)]
}

/// The control's measurement of one colour: `[x0, y0, x1, y1]` (inclusive px) and whether
/// the first / last row is that colour across >= 90% of the box's width.
fn measure(
    (w, h, buf): &(usize, usize, Vec<[u8; 3]>),
    color: [u8; 3],
) -> Option<(Vec<i64>, bool, bool)> {
    let is = |x: usize, y: usize| buf[y * w + x] == color;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    let mut any = false;
    for y in 0..*h {
        for x in 0..*w {
            if is(x, y) {
                any = true;
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    if !any {
        return None;
    }
    let row = |y: usize| (x0..=x1).filter(|&x| is(x, y)).count() * 10 >= (x1 - x0 + 1) * 9;
    let bbox = [x0, y0, x1, y1].map(|v| v as i64).to_vec();
    Some((bbox, row(y0), row(y1)))
}

type Measured = Option<(Vec<i64>, bool, bool)>;

/// The control's record for one colour on one page (`null` = the colour is absent).
fn chrome_box(want: &serde_json::Value) -> Measured {
    (!want.is_null()).then(|| {
        let bbox = want["bbox"].as_array().unwrap();
        (
            bbox.iter().map(|v| v.as_i64().unwrap()).collect(),
            want["top"].as_bool().unwrap(),
            want["bottom"].as_bool().unwrap(),
        )
    })
}

/// Every colour of a case compared on one page; mismatches are pushed to `failures`.
fn compare_page(
    id: &str,
    n: usize,
    (page, want): (&Page, &serde_json::Value),
    colors: &BTreeMap<String, String>,
    failures: &mut Vec<String>,
) {
    let raster = rasterize(page);
    for (name, css) in colors {
        let (got, want) = (measure(&raster, rgb(css)), chrome_box(&want["boxes"][name]));
        let known = KNOWN.contains(&(id, n, name.as_str()));
        failures.extend(verdict(known, &got, &want).map(|v| format!("{id} page {n} {name}: {v}")));
    }
}

/// The failure for one measurement: a difference not listed in [`KNOWN`], or a listed
/// one that now matches (stale).
fn verdict(known: bool, got: &Measured, want: &Measured) -> Option<String> {
    match (known, got == want) {
        (true, true) => Some("matches Chrome, remove it from KNOWN".to_string()),
        (false, false) => Some(format!("{got:?}, Chrome {want:?}")),
        _ => None,
    }
}

fn compare_case(case: &serde_json::Value, failures: &mut Vec<String>) {
    let id = case["id"].as_str().unwrap();
    let ours = pages(case["html"].as_str().unwrap());
    let chrome = case["pages"].as_array().unwrap();
    if ours.len() != chrome.len() {
        failures.push(format!(
            "{id}: {} pages, Chrome {}",
            ours.len(),
            chrome.len()
        ));
        return;
    }
    let colors: BTreeMap<String, String> = serde_json::from_value(case["colors"].clone()).unwrap();
    for (n, pair) in ours.iter().zip(chrome).enumerate() {
        compare_page(id, n + 1, pair, &colors, failures);
    }
}

/// Measurements (case, page, colour) that do not match Chrome yet; each must still differ,
/// so a fixed one is removed from the list. The rest of those cases is compared as usual.
/// (`exact-fill-child-margin` matches since miaskiewicz/turbo-dom#41: the paged layout keeps
/// a last child's bottom margin inside a padded parent.)
const KNOWN: &[(&str, usize, &str)] = &[
    // miaskiewicz/turbo-html2pdf#41: a static-position absolute box in a split relative
    // block is placed after the block's later flow content.
    ("absolute-static-mid-split-relative", 2, "red"),
    // miaskiewicz/turbo-html2pdf#42: the part of a split relative block shifted across a
    // page boundary is not painted on the page it crosses into (Chrome also bleeds the
    // shifted continuation 1px above the page area).
    ("relative-bottom-split", 1, "magenta"),
    ("relative-bottom-split", 2, "magenta"),
    ("split-decorated-relative", 2, "orange"),
];

/// miaskiewicz/turbo-html2pdf#39: the print initial containing block (the page area).
fn icb_case(id: &str) -> bool {
    id.starts_with("icb-")
}

fn fixture() -> serde_json::Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/pagination_chrome156.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).unwrap()
}

#[test]
fn the_page_area_is_the_initial_containing_block_like_chrome_156() {
    let mut failures = Vec::new();
    let mut compared = 0;
    for case in fixture()["cases"].as_array().unwrap() {
        if icb_case(case["id"].as_str().unwrap()) {
            compared += 1;
            compare_case(case, &mut failures);
        }
    }
    assert_eq!(compared, 4);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn paginated_boxes_match_chrome_156() {
    let mut failures = Vec::new();
    for case in fixture()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| !icb_case(case["id"].as_str().unwrap()))
    {
        compare_case(case, &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn an_absolute_box_stays_inside_its_positioned_parent_placed_whole() {
    // Its containing block moves as a whole, so the box stays its child (its paint and
    // tagged structure with it) rather than being re-placed at the page level.
    let html = r#"<!DOCTYPE html><html><head><style>@page{size:600px 400px;margin:40px}body{margin:0}</style></head><body><div style="height:300px"></div><div style="position:relative;height:200px;break-inside:avoid"><div style="position:absolute;top:20px;left:30px;width:50px;height:30px;background:#f00"></div></div></body></html>"#;
    let pages = pages(html);
    assert_eq!(pages.len(), 2);
    let parent = &pages[1].body[0];
    assert_eq!(parent.children.len(), 1);
    assert_eq!(parent.children[0].y - parent.y, 20.0);
}
