//! Hostile SVG images (`--features svg`): every case runs under a watchdog so a
//! hang fails the test instead of wedging CI, and asserts what Chrome 156 does with
//! the same bytes in an `<img>` (an SVG image never fetches anything external).

#![cfg(feature = "svg")]

use std::sync::mpsc;
use std::time::Duration;

use turbo_html2pdf_core::svg::{rasterize, Oversize, PDF_SVG_SCALE};

/// Rasterize `svg` on a worker thread, failing if it takes over 60 s. Returns the
/// RGBA raster (or `None` for an unparseable / degenerate image).
fn raster(svg: String) -> Option<(u32, u32, Vec<u8>)> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let r = rasterize(svg.as_bytes(), PDF_SVG_SCALE, Oversize::Clamp)
                .map(|i| (i.width, i.height, i.rgba));
            tx.send(r).ok();
        })
        .expect("spawn");
    rx.recv_timeout(Duration::from_secs(60))
        .expect("rasterize did not finish within 60 s")
}

fn red_pixels(rgba: &[u8]) -> usize {
    rgba.chunks(4)
        .filter(|p| p[0] > 200 && p[1] < 50 && p[3] > 200)
        .count()
}

#[test]
fn billion_laughs_terminates() {
    let mut dtd = String::from("<!ENTITY a0 \"lol\">");
    for i in 1..10 {
        let prev = format!("&a{};", i - 1);
        dtd.push_str(&format!("<!ENTITY a{i} \"{}\">", prev.repeat(10)));
    }
    let svg = format!(
        "<?xml version=\"1.0\"?><!DOCTYPE svg [{dtd}]><svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"4\"><text>&a9;</text></svg>"
    );
    // Chrome refuses the document (entity expansion limit): no image. Either way it
    // must terminate; a parsed tree would hold 10^9 characters.
    let _ = raster(svg);
}

#[test]
#[ignore = "fails until miaskiewicz/turbo-dom#197 lands (turbo-dom-layout svg.rs)"]
fn deep_nesting_terminates_without_overflow() {
    let depth = 100_000;
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"4\">{}<rect width=\"4\" height=\"4\"/>{}</svg>",
        "<g>".repeat(depth),
        "</g>".repeat(depth)
    );
    let _ = raster(svg);
}

#[test]
#[ignore = "fails until miaskiewicz/turbo-dom#196 lands (turbo-dom-layout svg.rs)"]
fn external_file_image_is_not_loaded() {
    let dir = std::env::temp_dir().join(format!("h2p-hostile-svg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let secret = dir.join("secret.svg");
    std::fs::write(
        &secret,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="#ff0000"/></svg>"##,
    )
    .expect("write");
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="4" height="4"><image width="4" height="4" xlink:href="{}"/></svg>"##,
        secret.display()
    );
    let (_, _, rgba) = raster(svg).expect("a parseable SVG");
    std::fs::remove_dir_all(&dir).ok();
    // Chrome 156: an SVG loaded as an image never fetches subresources -> transparent.
    assert_eq!(red_pixels(&rgba), 0, "a hostile SVG read a local file");
}

#[test]
#[ignore = "fails until miaskiewicz/turbo-dom#198 lands (turbo-dom-layout svg.rs)"]
fn oversized_svg_is_scaled_down_not_cropped() {
    // 10000x10000 CSS px with the only paint in the bottom-right quarter. Chrome
    // draws the whole image into its box; a clamped raster must still show it.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10000" height="10000"><rect x="5000" y="5000" width="5000" height="5000" fill="#ff0000"/></svg>"##;
    let (w, h, rgba) = raster(svg.to_string()).expect("a parseable SVG");
    assert!(w <= 8192 && h <= 8192);
    assert!(
        red_pixels(&rgba) > 0,
        "the clamped raster cropped the image to its top-left corner"
    );
}

#[test]
fn huge_viewbox_terminates() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4" viewBox="0 0 1e30 1e30"><rect width="1e30" height="1e30" fill="#ff0000"/></svg>"##;
    let (_, _, rgba) = raster(svg.to_string()).expect("a parseable SVG");
    assert!(red_pixels(&rgba) > 0);
}
