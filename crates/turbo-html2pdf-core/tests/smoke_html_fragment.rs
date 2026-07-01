//! Prove the public Jinja-free drive `layout_html` turns a raw HTML string
//! (with a `<style>` block + inline styles) into a positioned `Fragment` galley
//! with real glyph coordinates, font faces, and box colors — the foundation a
//! synthetic-screenshot rasterizer builds on. Uses `FontRegistry::new()`
//! (bundled-fonts default) so no font fixture is required.

use turbo_html2pdf_core::layout::fragment::{Fragment, FragmentContent};
use turbo_html2pdf_core::text::FontRegistry;
use turbo_html2pdf_core::{layout_html, Diagnostics, Rgba};

fn walk<'a>(f: &'a Fragment, out: &mut Vec<&'a Fragment>) {
    out.push(f);
    for c in &f.children {
        walk(c, out);
    }
}

#[test]
fn raw_html_to_positioned_fragment() {
    // Author CSS (a class selector, as turbo-surf will pass head `<style>` via
    // `extra_css`) drives the box color; inline style drives the text. The
    // `{{ }}` in the script must survive verbatim (Jinja is bypassed).
    let html = r#"<html><body>
        <div class="card">
          <p style="color:#0000ff;font-size:20px">Hello Screenshot</p>
        </div>
        <script>const t = `{{ not_a_template }}`;</script>
      </body></html>"#;
    let author_css = ".card { background-color: #ff0000; padding: 10px; }";

    let mut diags = Diagnostics::default();
    let galley =
        layout_html(html, author_css, 1280.0, &FontRegistry::new(), &mut diags).expect("layout");

    let mut frags = Vec::new();
    walk(&galley, &mut frags);
    assert!(
        frags.len() > 3,
        "expected a real fragment tree, got {}",
        frags.len()
    );

    // Text line: positioned glyphs + a font face whose bytes we can outline.
    let (glyphs, face, font_size, color) = frags
        .iter()
        .find_map(|f| match &f.content {
            FragmentContent::TextLine {
                glyphs,
                face,
                font_size,
                color,
            } => Some((glyphs.clone(), face.clone(), *font_size, *color)),
            _ => None,
        })
        .expect("a TextLine fragment");
    assert!(!glyphs.is_empty());
    assert_eq!(font_size, 20.0);
    assert_eq!(
        color,
        Rgba {
            r: 0,
            g: 0,
            b: 255,
            a: 255
        },
        "inline color should win"
    );
    assert!(
        !face.data().is_empty(),
        "face must expose font bytes for outlining"
    );
    assert!(face.units_per_em() > 0);

    // Box fill from the `<style>` block (proves author-CSS collection).
    let red = frags.iter().any(|f| {
        matches!(&f.content, FragmentContent::Box { background: Some(bg), .. }
            if *bg == (Rgba { r: 255, g: 0, b: 0, a: 255 }))
    });
    assert!(
        red,
        "expected the .card red background from the <style> block"
    );

    eprintln!(
        "SMOKE OK: {} frags; glyph0@({:.1},{:.1}); size={font_size}; face={}B",
        frags.len(),
        glyphs[0].x,
        glyphs[0].y,
        face.data().len(),
    );
}
