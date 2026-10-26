//! miaskiewicz/turbo-dom#52: an absolutely positioned box's `clip: rect()` in the PDF.
//!
//! turbo-dom-layout keeps a partially clipped box and records its region in
//! `Fragment::clip` (turbo-dom's Chrome-156 pixel control `scripts/paint-chrome-control.mjs`,
//! `clip-partial`: a 30x30 red square at (10,10) of a 50x50 red box). The emitter clips
//! the box and its subtree to that region (`q … re W n … Q`). When `pdftoppm` is on
//! `PATH` the page is rasterized at 96 dpi (1 device px per CSS px) and probed inside and
//! outside the region.

mod common;

use std::process::Command;

use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, compile, emit_pdf, render_pages, CompileOptions, Diagnostics, EmitOptions, Page,
    RenderInputs,
};

const CLIPPED: &str = "<div style=\"position:relative;height:100px\"><div style=\"position:absolute;\
    left:0;top:0;width:50px;height:50px;background:#ff0000;clip:rect(10px,40px,40px,10px)\"></div></div>";

fn pages(html: &str) -> Vec<Page> {
    let (program, _) = compile(html, &CompileOptions::default()).expect("compile");
    let cascade = build_cascade("", "", TokenSet::default());
    let fonts = common::registry();
    let inputs = RenderInputs {
        program: &program,
        data: &serde_json::Value::Null,
        cascade: &cascade,
        at_rules: &[],
        fonts: &fonts,
        images: &turbo_html2pdf_core::NoImages,
        now: Some(0),
    };
    render_pages(&inputs, &mut Diagnostics::default()).expect("render_pages")
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
#[ignore = "miaskiewicz/turbo-dom#52: fails until the fix"]
fn a_partially_clipped_box_paints_inside_its_clip_region() {
    let pdf = emit_pdf(&pages(CLIPPED), &EmitOptions::default());
    // The red box paints, inside a clipping path that is popped afterwards.
    assert!(contains(&pdf, b"1 0 0 rg"), "the red box paints");
    assert!(contains(&pdf, b"re\nW\nn"), "a rectangular clipping path");
    let text = String::from_utf8_lossy(&pdf);
    let clip = text.find("re\nW\nn").unwrap();
    assert!(text[..clip].contains("q\n"), "the clip is in a saved state");
    let (fill, restore) = (text[clip..].find("1 0 0 rg"), text[clip..].find("Q\n"));
    assert!(
        fill.unwrap() < restore.unwrap(),
        "the box paints before the state is restored"
    );
}

/// The emitted page rasterized at 96 dpi by `pdftoppm` (`None` when it is not on `PATH`).
fn raster(pdf: &[u8]) -> Option<(usize, Vec<u8>)> {
    let dir = std::env::temp_dir().join(format!("turbo_clip_rect_{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let input = dir.join("clip.pdf");
    std::fs::write(&input, pdf).ok()?;
    let out = Command::new("pdftoppm")
        .args(["-r", "96", "-singlefile"])
        .arg(&input)
        .arg(dir.join("page"))
        .output()
        .ok()?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ppm = std::fs::read(dir.join("page.ppm")).ok()?;
    std::fs::remove_dir_all(&dir).ok();
    // P6 header: "P6\n<w> <h>\n255\n".
    let mut fields = ppm.splitn(4, u8::is_ascii_whitespace);
    let (_, w, _) = (fields.next()?, fields.next()?, fields.next()?);
    let width: usize = std::str::from_utf8(w).ok()?.parse().ok()?;
    let header = ppm.len() - fields.next()?.len();
    let rest = &ppm[header..];
    let body = &rest[rest.iter().position(|&b| b == b'\n')? + 1..];
    Some((width, body.to_vec()))
}

#[test]
#[ignore = "miaskiewicz/turbo-dom#52: fails until the fix"]
fn a_partially_clipped_box_rasterizes_like_chrome_156() {
    let pages = pages(CLIPPED);
    let pdf = emit_pdf(&pages, &EmitOptions::default());
    let Some((width, rgb)) = raster(&pdf) else {
        eprintln!("pdftoppm not on PATH; skipping the raster probe");
        return;
    };
    let margin = pages[0].geometry.margin;
    let red = |x: f32, y: f32| {
        let (px, py) = ((margin.left + x) as usize, (margin.top + y) as usize);
        let p = &rgb[(py * width + px) * 3..][..3];
        p[0] > 160 && p[1] < 96 && p[2] < 96
    };
    // Chrome 156 (`clip-partial`): red inside (10,10)-(40,40), the page outside it.
    for (x, y) in [(25.0, 25.0), (11.0, 11.0), (38.0, 38.0)] {
        assert!(red(x, y), "({x},{y}) is red");
    }
    for (x, y) in [(5.0, 5.0), (45.0, 45.0), (25.0, 5.0), (5.0, 25.0)] {
        assert!(!red(x, y), "({x},{y}) is clipped away");
    }
}
