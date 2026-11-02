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

/// Whether `pdftoppm` (poppler) is on `PATH`.
fn have_pdftoppm() -> bool {
    Command::new("pdftoppm").arg("-v").output().is_ok()
}

/// The emitted page rasterized at 96 dpi by `pdftoppm`: its width and RGB bytes.
fn raster(pdf: &[u8]) -> (usize, Vec<u8>) {
    let dir = std::env::temp_dir().join(format!("turbo_clip_rect_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let input = dir.join("clip.pdf");
    std::fs::write(&input, pdf).expect("write pdf");
    let out = Command::new("pdftoppm")
        .args(["-r", "96", "-singlefile"])
        .arg(&input)
        .arg(dir.join("page"))
        .output()
        .expect("run pdftoppm");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ppm = std::fs::read(dir.join("page.ppm")).expect("read ppm");
    std::fs::remove_dir_all(&dir).ok();
    parse_ppm(&ppm)
}

/// A binary PPM (`P6\n<w> <h>\n255\n<rgb>`): its width and RGB bytes.
fn parse_ppm(ppm: &[u8]) -> (usize, Vec<u8>) {
    let text = String::from_utf8_lossy(&ppm[..ppm.len().min(64)]);
    let fields: Vec<&str> = text.split_ascii_whitespace().take(4).collect();
    let width: usize = fields[1].parse().expect("ppm width");
    // The header is the first three lines ("P6", "<w> <h>", "255").
    let mut newlines = ppm.iter().enumerate().filter(|(_, &b)| b == b'\n');
    let start = newlines.nth(2).expect("ppm header").0 + 1;
    (width, ppm[start..].to_vec())
}

#[test]
fn a_partially_clipped_box_rasterizes_like_chrome_156() {
    let pages = pages(CLIPPED);
    let pdf = emit_pdf(&pages, &EmitOptions::default());
    if !have_pdftoppm() {
        eprintln!("pdftoppm not on PATH; skipping the raster probe");
        return;
    }
    let (width, rgb) = raster(&pdf);
    // The containing block (the relative `<div>`), in page coordinates.
    let cb = &pages[0].body[0];
    let red = |x: f32, y: f32| {
        let (px, py) = ((cb.x + x) as usize, (cb.y + y) as usize);
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
