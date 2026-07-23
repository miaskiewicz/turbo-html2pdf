//! Unit tests for [`super::embed_watermark_font`] (Task 1 of the `stamp`
//! embedded-font feature).
//!
//! **Why this lives at `tests/stamp_font/mod.rs`, not `tests/stamp_font.rs`.**
//! `embed_watermark_font`/`EmbeddedWatermarkFont` are `pub(crate)` — the fixed
//! interface Tasks 2-4 depend on — and `pub(crate)` never crosses a crate
//! boundary. A file directly under `tests/` (e.g. `tests/stamp_font.rs`) is
//! auto-discovered by Cargo as its OWN separate integration-test crate, which
//! can only see this crate's `pub` surface; empirically (`cargo check --tests`)
//! that produced real "cannot find function `embed_watermark_font`" errors when
//! tried. Cargo's autodiscovery only scans DIRECT children of `tests/`, not
//! files in a subdirectory — the same reason `tests/stamp.rs`'s own `mod
//! common;` lives at `tests/common/mod.rs` rather than `tests/common.rs`. So
//! this file is included as `src/stamp_font.rs`'s own `#[cfg(test)] #[path =
//! "../tests/stamp_font/mod.rs"] mod tests;`, making it a crate-internal unit
//! test (full `pub(crate)` access) while keeping the test code physically out
//! of `stamp_font.rs` and under `tests/`, matching the brief's intent as
//! closely as Rust's privacy model allows. Run with `cargo test --features
//! stamp --lib stamp_font::tests`.

use super::*;

use lopdf::{Document, Object, ObjectId};

use crate::text::FontFace;
use crate::FontRegistry;

fn bundled_face() -> FontFace {
    FontRegistry::new()
        .select(&[], 400, false)
        .expect("bundled sans")
        .clone()
}

// --------------------------------------------------------------------------
// Step 1 — spike probes for the two runtime unknowns. Not exhaustive specs;
// they exist to print/assert the answer once, which is then recorded in
// `font-task1-report.md` and baked into `embed_watermark_font`'s real
// implementation below.
// --------------------------------------------------------------------------

mod step1_spike {
    use super::*;
    use crate::emit::{EmitOptions, FontStore, RefAlloc};
    use pdf_writer::Pdf;

    /// ANSWER(a): `Pdf::finish()`'s output loads cleanly in
    /// `lopdf::Document::load_mem` with NO `/Catalog`/`/Root` at all — despite
    /// `Pdf::catalog` being documented "Required", `finish()` only omits
    /// `/Root` from the trailer when none was set, and `lopdf` parses the
    /// object table + xref + trailer independently of whether `/Root` is
    /// present (it's read lazily via `get_pages()`/`trailer.get("Root")`,
    /// never validated at `load_mem` time). So `embed_watermark_font` skips
    /// the stub-catalog workaround entirely: `Pdf::new()` +
    /// `RefAlloc::new(1)` is enough.
    #[test]
    fn probe_a_no_catalog_load_mem_succeeds() {
        let face = bundled_face();
        let gids: Vec<u16> = face.shape("CANCELLED").iter().map(|g| g.glyph_id).collect();
        let mut store = FontStore::default();
        store.record_glyphs(&face, &gids);

        let mut pdf = Pdf::new();
        let mut alloc = RefAlloc::new(1);
        let refs = store.write(&mut pdf, &mut alloc, &EmitOptions::default());
        let bytes = pdf.finish();

        let doc = Document::load_mem(&bytes)
            .expect("ANSWER(a): load_mem WITHOUT any catalog/root succeeds");
        assert_eq!(
            doc.objects.len(),
            4,
            "default EmitOptions writes exactly 4 objects/face"
        );

        // ANSWER(b), same no-catalog bytes: the Type0 lands at exactly the id
        // `refs[0]` (the first Ref `RefAlloc::new(1)` handed out) reported —
        // `load_mem` preserves the original object numbers verbatim (it builds
        // its object table by parsing each `N 0 obj` header, it does not
        // renumber), so `embed_watermark_font` can index by `refs[0].get()`
        // directly instead of scanning for `/Subtype /Type0`.
        let type0_ref = refs[0];
        let expected_id: ObjectId = (type0_ref.get() as u32, 0);
        let dict = doc
            .get_dictionary(expected_id)
            .expect("object exists at the id FontStore::write's Ref reported");
        let subtype = dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok());
        assert_eq!(
            subtype,
            Some(b"Type0".as_slice()),
            "ANSWER(b): the object at the REPORTED ref number is the Type0 font — ids are stable, no scan needed"
        );
    }
}

// --------------------------------------------------------------------------
// Step 2/3 — `embed_watermark_font` structural assertions.
// --------------------------------------------------------------------------

/// Collect every `/BaseFont` name across the font's whole object closure —
/// the Type0 AND its CIDFont descendant — by reusing the production
/// [`super::font_closure`] walker. Reusing it (rather than a second hand-rolled
/// walk) keeps this assertion from re-introducing the very array-recursion bug
/// `walk_references` already fixed: `/DescendantFonts` is an array wrapping the
/// CIDFont reference, so a walker that only follows a dict's immediate
/// references never inspects the descendant's `/BaseFont`.
fn base_font_names(doc: &Document, id: ObjectId, out: &mut Vec<String>) {
    for obj_id in font_closure(doc, id) {
        let Ok(dict) = doc.get_object(obj_id).and_then(|o| o.as_dict()) else {
            continue;
        };
        if let Ok(Object::Name(name)) = dict.get(b"BaseFont") {
            out.push(String::from_utf8_lossy(name).to_string());
        }
    }
}

/// Every `Object::Reference` reachable from `id`, transitively.
fn referenced_ids(doc: &Document, id: ObjectId) -> Vec<ObjectId> {
    let mut seen = Vec::new();
    collect_refs(doc, id, &mut seen);
    seen
}

fn collect_refs(doc: &Document, id: ObjectId, seen: &mut Vec<ObjectId>) {
    if seen.contains(&id) {
        return;
    }
    seen.push(id);
    let Ok(obj) = doc.get_object(id) else {
        return;
    };
    walk_object(doc, obj, seen);
}

fn walk_object(doc: &Document, obj: &Object, seen: &mut Vec<ObjectId>) {
    match obj {
        Object::Dictionary(dict) => {
            for (_, value) in dict.iter() {
                walk_value(doc, value, seen);
            }
        }
        Object::Stream(stream) => {
            for (_, value) in stream.dict.iter() {
                walk_value(doc, value, seen);
            }
        }
        Object::Array(arr) => {
            for value in arr {
                walk_value(doc, value, seen);
            }
        }
        _ => {}
    }
}

fn walk_value(doc: &Document, value: &Object, seen: &mut Vec<ObjectId>) {
    match value {
        Object::Reference(r) => collect_refs(doc, *r, seen),
        Object::Array(_) | Object::Dictionary(_) => walk_object(doc, value, seen),
        _ => {}
    }
}

#[test]
fn embeds_a_real_type0_font_no_base14() {
    let mut doc = Document::with_version("1.7");
    let face = bundled_face();

    let embedded = embed_watermark_font(&mut doc, &face, "CANCELLED", 48.0);

    let type0 = doc
        .get_dictionary(embedded.type0_id)
        .expect("type0_id resolves to a dictionary in doc");
    assert_eq!(
        type0.get(b"Type").and_then(Object::as_name).ok(),
        Some(b"Font".as_slice())
    );
    assert_eq!(
        type0.get(b"Subtype").and_then(Object::as_name).ok(),
        Some(b"Type0".as_slice())
    );
    assert_eq!(
        type0.get(b"Encoding").and_then(Object::as_name).ok(),
        Some(b"Identity-H".as_slice())
    );
    let descendants = type0
        .get(b"DescendantFonts")
        .and_then(Object::as_array)
        .expect("DescendantFonts is an array");
    assert_eq!(descendants.len(), 1);

    let cid_id = descendants[0]
        .as_reference()
        .expect("descendant is a reference");
    let cid_dict = doc.get_dictionary(cid_id).expect("descendant resolves");
    let cid_subtype = cid_dict.get(b"Subtype").and_then(Object::as_name).unwrap();
    assert!(
        cid_subtype == b"CIDFontType0" || cid_subtype == b"CIDFontType2",
        "descendant subtype must be a CIDFont, got {:?}",
        String::from_utf8_lossy(cid_subtype)
    );

    let descriptor_id = cid_dict
        .get(b"FontDescriptor")
        .and_then(Object::as_reference)
        .expect("descendant has a FontDescriptor reference");
    let descriptor = doc
        .get_dictionary(descriptor_id)
        .expect("descriptor resolves");
    let has_program = descriptor.has(b"FontFile2") || descriptor.has(b"FontFile3");
    assert!(
        has_program,
        "descriptor must attach a FontFile2 or FontFile3 stream"
    );

    let program_id = descriptor
        .get(b"FontFile2")
        .or_else(|_| descriptor.get(b"FontFile3"))
        .and_then(Object::as_reference)
        .expect("program ref present");
    assert!(
        matches!(doc.get_object(program_id), Ok(Object::Stream(_))),
        "the font program object is a stream"
    );

    let mut names = Vec::new();
    base_font_names(&doc, embedded.type0_id, &mut names);
    assert!(!names.is_empty(), "expected at least one BaseFont name");
    for name in &names {
        assert_ne!(name, "Helvetica", "must not embed a base-14 name");
        assert!(
            ![
                "Helvetica",
                "Courier",
                "Times-Roman",
                "Symbol",
                "ZapfDingbats"
            ]
            .contains(&name.as_str()),
            "must not embed any base-14 name, got {name}"
        );
    }

    let shaped = face.shape("CANCELLED");
    assert_eq!(embedded.codes.len(), 2 * shaped.len());
    assert!(embedded.advance_pt > 0.0);

    for id in referenced_ids(&doc, embedded.type0_id) {
        assert!(
            doc.get_object(id).is_ok(),
            "every reference inside the transplanted closure must resolve within doc, dangling: {id:?}"
        );
    }
}

#[test]
fn codes_are_two_byte_be_pairs_matching_shaped_glyph_count() {
    let mut doc = Document::with_version("1.7");
    let face = bundled_face();
    let embedded = embed_watermark_font(&mut doc, &face, "Ab", 32.0);
    let shaped = face.shape("Ab");
    assert_eq!(embedded.codes.len(), shaped.len() * 2);
    assert_eq!(embedded.codes.len() % 2, 0);
}

// --------------------------------------------------------------------------
// `walk_closure`'s two defensive branches: neither a real `FontStore`-written
// mini PDF ever cycles or dangles, so nothing above exercises them; covered
// directly here against a hand-built `Document`.
// --------------------------------------------------------------------------

#[test]
fn walk_closure_stops_at_an_already_seen_id() {
    let mini = Document::with_version("1.7");
    let id: ObjectId = (1, 0);
    let mut seen = vec![id];
    walk_closure(&mini, id, &mut seen);
    assert_eq!(
        seen,
        vec![id],
        "an already-seen id must not be revisited or grown"
    );
}

#[test]
fn walk_closure_skips_a_dangling_reference() {
    let mini = Document::with_version("1.7");
    let dangling: ObjectId = (99, 0);
    let mut seen = Vec::new();
    walk_closure(&mini, dangling, &mut seen);
    assert_eq!(
        seen,
        vec![dangling],
        "a dangling id is recorded as visited but contributes no further ids"
    );
}

// --------------------------------------------------------------------------
// Step 4 — structural render proof: build a one-page doc, invoke the embedded
// font from a content stream, and (if `qpdf` is on PATH) validate it.
// --------------------------------------------------------------------------

fn qpdf_available() -> bool {
    std::process::Command::new("which")
        .arg("qpdf")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Add a page/pages/catalog around the already-transplanted font, into the
/// SAME `doc` (so `doc.add_object`'s `max_id`-based allocation can't collide
/// with the font's ids), invoking the font from the page's content stream.
fn add_one_page_invoking_font(doc: &mut Document, embedded: &EmbeddedWatermarkFont) {
    let content = format!(
        "q BT /F1 24 Tf 100 700 Td <{}> Tj ET Q",
        embedded
            .codes
            .chunks(2)
            .map(|c| format!("{:02X}{:02X}", c[0], c[1]))
            .collect::<String>()
    );
    let content_id = doc.add_object(Object::Stream(lopdf::Stream::new(
        lopdf::Dictionary::new(),
        content.into_bytes(),
    )));

    let mut font_res = lopdf::Dictionary::new();
    font_res.set("F1", Object::Reference(embedded.type0_id));
    let mut fonts = lopdf::Dictionary::new();
    fonts.set("Font", Object::Dictionary(font_res));

    let mut page = lopdf::Dictionary::new();
    page.set("Type", "Page");
    page.set("MediaBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
    page.set("Resources", Object::Dictionary(fonts));
    page.set("Contents", Object::Reference(content_id));
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = lopdf::Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![Object::Reference(page_id)]);
    pages.set("Count", 1);
    let pages_id = doc.add_object(Object::Dictionary(pages));
    doc.get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .expect("page dict")
        .set("Parent", Object::Reference(pages_id));

    let mut catalog = lopdf::Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", Object::Reference(pages_id));
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", Object::Reference(catalog_id));
}

#[test]
fn structural_render_passes_qpdf_check() {
    let face = bundled_face();
    let mut doc = Document::with_version("1.7");
    let embedded = embed_watermark_font(&mut doc, &face, "CANCELLED", 48.0);
    add_one_page_invoking_font(&mut doc, &embedded);

    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).expect("save_to");

    if !qpdf_available() {
        return;
    }
    let path = std::env::temp_dir().join("turbo-pdf-stamp-font-check.pdf");
    std::fs::write(&path, &bytes).expect("write temp pdf");
    let out = std::process::Command::new("qpdf")
        .arg("--check")
        .arg(&path)
        .output()
        .expect("run qpdf");
    assert!(
        out.status.success(),
        "qpdf --check failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
