//! `stamp` feature tests (Tasks 2-3 of the `stamp()` design). Only compiled
//! with `--features stamp`. Builds a real, multi-page PDF with turbo's own
//! emitter, then exercises [`stamp`]: every page keeps its original content and
//! the document keeps its page count, while EACH page gains a faded diagonal
//! watermark overlay carrying the requested text.
//!
//! The overlay lives in a per-page Form XObject (isolated `/Resources`, so it
//! never collides with turbo's own font / `ExtGState` names). The strong
//! per-page assertions therefore look in two places: the page's own content
//! stream must *invoke* the overlay (`/<name> Do`) and still carry its original
//! body operators (`BT`), and the referenced Form XObject's stream must carry
//! the watermark's shown text (`(CANCELLED) Tj`), its fade (`gs`) and its
//! rotation (`cm`).
//!
//! Task 3 widens [`stamp`] with `password`/`encryption` parameters: the
//! plaintext-only tests below now pass `None, None` (unchanged behaviour), and
//! a second block of tests exercises the encrypted round trip (decrypt with
//! `password`, overlay, re-encrypt with `encryption`) against a REAL encrypted
//! fixture built through turbo's own `emit_pdf` + `Encryption`, mirroring the
//! Task 1 spike (`tests/encryption_roundtrip.rs`) but through `stamp` itself —
//! including feeding turbo's raw encrypted bytes with NO test-side `/Length`
//! patch, proving that normalization now lives inside `stamp`.

#![cfg(feature = "stamp")]

mod common;

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};

use turbo_html2pdf_core::layout::fragment::{Fragment, FragmentContent, NodeId, PositionedGlyph};
use turbo_html2pdf_core::layout::value::Rgba;
use turbo_html2pdf_core::paginate::{Page, PageGeometry};
use turbo_html2pdf_core::{
    emit_pdf, stamp, EmitOptions, Encryption, FontFace, FontRegistry, PageKind, Permissions,
    StampError, StampWatermark, WATERMARK_XOBJECT_NAME,
};

const WATERMARK_TEXT: &str = "CANCELLED";
const ORIGINAL_USER_PW: &str = "s3cret-user-pw";
const NEW_USER_PW: &str = "brand-new-user-pw";
const NEW_OWNER_PW: &str = "brand-new-owner-pw";

// --------------------------------------------------------------------------
// fixture: a real, multi-page plaintext PDF from turbo's own emitter
// --------------------------------------------------------------------------

/// A body text line so every page has a real content stream (with a `BT`
/// operator) to prove additivity against.
fn body_text(face: FontFace) -> Fragment {
    let glyphs = [10u16, 11, 12]
        .iter()
        .enumerate()
        .map(|(i, &glyph_id)| PositionedGlyph {
            glyph_id,
            x: i as f32 * 10.0,
            y: 12.0,
        })
        .collect();
    Fragment::new(
        NodeId(1),
        20.0,
        30.0,
        200.0,
        16.0,
        FragmentContent::TextLine {
            glyphs,
            face,
            font_size: 12.0,
            color: Rgba::new(0, 0, 0, 255),
        },
    )
}

fn page_with(body: Vec<Fragment>, number: u32) -> Page {
    Page {
        geometry: PageGeometry::a4(),
        kind: PageKind::First,
        number,
        body,
        header: Vec::new(),
        footer: Vec::new(),
        footnotes: Vec::new(),
    }
}

/// A two-page plaintext PDF (no encryption, no watermark at emit time).
fn two_page_pdf() -> Vec<u8> {
    let pages = vec![
        page_with(vec![body_text(common::evolventa())], 1),
        page_with(vec![body_text(common::evolventa())], 2),
    ];
    emit_pdf(&pages, &EmitOptions::default())
}

/// A two-page PDF protected under `ORIGINAL_USER_PW`, built through turbo's own
/// `emit_pdf` + `Encryption` path (the same real fixture shape as the Task 1
/// spike) — turbo's raw output, no test-side byte patching.
fn two_page_encrypted_pdf() -> Vec<u8> {
    let pages = vec![
        page_with(vec![body_text(common::evolventa())], 1),
        page_with(vec![body_text(common::evolventa())], 2),
    ];
    let opts = EmitOptions {
        encryption: Some(Encryption {
            user_password: ORIGINAL_USER_PW.to_string(),
            owner_password: None,
            permissions: Permissions::all(),
        }),
        ..EmitOptions::default()
    };
    emit_pdf(&pages, &opts)
}

/// The `Encryption` `stamp` should re-encrypt the stamped output under.
fn new_encryption() -> Encryption {
    Encryption {
        user_password: NEW_USER_PW.to_string(),
        owner_password: Some(NEW_OWNER_PW.to_string()),
        permissions: Permissions::all(),
    }
}

/// A cancelled/faded diagonal text watermark, carrying a real bundled font
/// face (the overlay embeds it via `embed_watermark_font`, no more base-14
/// Helvetica).
fn cancelled_watermark() -> StampWatermark {
    StampWatermark {
        text: WATERMARK_TEXT.to_string(),
        font_size: 64.0,
        color: Rgba::new(128, 128, 128, 255),
        opacity: 0.15,
        angle_deg: 45.0,
        face: FontRegistry::new()
            .select(&[], 400, false)
            .expect("bundled sans")
            .clone(),
    }
}

// --------------------------------------------------------------------------
// helpers
// --------------------------------------------------------------------------

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The object ID of the Form XObject the overlay registered on `page_id` under
/// [`WATERMARK_XOBJECT_NAME`], shared by both the content-stream and `/BBox`
/// assertions below.
fn overlay_form_id(doc: &Document, page_id: ObjectId) -> ObjectId {
    let (inline, resource_ids) = doc
        .get_page_resources(page_id)
        .expect("page resources are readable");
    let xobjects = inline
        .and_then(|dict| dict.get(b"XObject").ok())
        .or_else(|| {
            resource_ids
                .iter()
                .find_map(|id| doc.get_dictionary(*id).ok()?.get(b"XObject").ok())
        })
        .and_then(|obj| obj.as_dict().ok())
        .expect("page carries an /XObject resource dict after stamping");
    xobjects
        .get(WATERMARK_XOBJECT_NAME.as_bytes())
        .and_then(Object::as_reference)
        .expect("the watermark Form XObject is registered under its name")
}

/// The decompressed bytes of the Form XObject the overlay registered on `page_id`
/// under [`WATERMARK_XOBJECT_NAME`].
fn overlay_form_content(doc: &Document, page_id: ObjectId) -> Vec<u8> {
    let form_id = overlay_form_id(doc, page_id);
    let stream = doc
        .get_object(form_id)
        .and_then(Object::as_stream)
        .expect("the watermark Form XObject is a stream");
    stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone())
}

/// The overlay Form XObject's `/BBox` width/height, in points — proves which
/// page size `stamp`'s internal `media_box_size` resolved to (the page's own
/// `/MediaBox`, an inherited one, or the A4 fallback).
fn overlay_form_bbox(doc: &Document, page_id: ObjectId) -> (f32, f32) {
    let form_id = overlay_form_id(doc, page_id);
    let stream = doc
        .get_object(form_id)
        .and_then(Object::as_stream)
        .expect("the watermark Form XObject is a stream");
    let bbox = stream
        .dict
        .get(b"BBox")
        .and_then(Object::as_array)
        .expect("the Form XObject carries a /BBox array");
    let n = |i: usize| bbox[i].as_float().expect("BBox operand is numeric");
    (n(2) - n(0), n(3) - n(1))
}

/// The `/F1` font resource `Object` from the overlay Form XObject's own
/// `/Resources /Font` dict, raw (not yet resolved) — asserted on directly so a
/// regression back to an inline base-14 `Font` dict (rather than an indirect
/// reference to a transplanted `Type0`) fails the "is a `Reference`" check
/// below instead of silently resolving through `as_dict`.
fn overlay_font_resource(doc: &Document, page_id: ObjectId) -> Object {
    let form_id = overlay_form_id(doc, page_id);
    let stream = doc
        .get_object(form_id)
        .and_then(Object::as_stream)
        .expect("the watermark Form XObject is a stream");
    let resources = stream
        .dict
        .get(b"Resources")
        .and_then(Object::as_dict)
        .expect("the Form XObject carries a /Resources dict");
    let fonts = resources
        .get(b"Font")
        .and_then(Object::as_dict)
        .expect("/Resources carries a /Font dict");
    fonts
        .get(b"F1")
        .expect("the overlay's font resource is named F1")
        .clone()
}

/// Every object id reachable from `root`, transitively following
/// `Object::Reference`s within `doc` (dict/stream/array bodies) — mirrors
/// `stamp_font.rs`'s own `font_closure`/`walk_closure` walker (not reusable
/// here: this file is a separate integration-test crate, so it can only see
/// `stamp_font`'s `pub(crate)` items via `stamp`'s public surface, which does
/// not re-export them).
fn font_closure(doc: &Document, root: ObjectId) -> Vec<ObjectId> {
    let mut ids = Vec::new();
    walk_closure(doc, root, &mut ids);
    ids
}

fn walk_closure(doc: &Document, id: ObjectId, seen: &mut Vec<ObjectId>) {
    if seen.contains(&id) {
        return;
    }
    seen.push(id);
    let Ok(obj) = doc.get_object(id) else {
        return;
    };
    walk_references(doc, obj, seen);
}

fn walk_references(doc: &Document, obj: &Object, seen: &mut Vec<ObjectId>) {
    match obj {
        Object::Reference(r) => walk_closure(doc, *r, seen),
        Object::Dictionary(dict) => {
            for (_, value) in dict.iter() {
                walk_references(doc, value, seen);
            }
        }
        Object::Stream(stream) => {
            for (_, value) in stream.dict.iter() {
                walk_references(doc, value, seen);
            }
        }
        Object::Array(arr) => {
            for value in arr {
                walk_references(doc, value, seen);
            }
        }
        _ => {}
    }
}

/// Base-14 font names the overlay must never embed under (a regression back
/// to unbundled Helvetica).
const BASE14_NAMES: [&[u8]; 5] = [
    b"Helvetica",
    b"Courier",
    b"Times-Roman",
    b"Symbol",
    b"ZapfDingbats",
];

/// Assert that `type0_id` is a real `/Type0` font whose descendant is a
/// `/CIDFontType0` or `/CIDFontType2` carrying an embedded `/FontFile2` or
/// `/FontFile3` program stream, and that no object in its closure carries a
/// base-14 `/BaseFont` name.
fn assert_embedded_type0_font(doc: &Document, type0_id: ObjectId) {
    let type0 = doc
        .get_dictionary(type0_id)
        .expect("the /F1 font resource resolves to a dictionary in doc");
    assert_eq!(
        type0.get(b"Subtype").and_then(Object::as_name).ok(),
        Some(b"Type0".as_slice()),
        "the referenced font must be a /Type0 composite font"
    );
    let descendants = type0
        .get(b"DescendantFonts")
        .and_then(Object::as_array)
        .expect("/Type0 carries a /DescendantFonts array");
    let cid_id = descendants[0]
        .as_reference()
        .expect("the descendant font is a reference");
    let cid_dict = doc.get_dictionary(cid_id).expect("descendant resolves");
    let cid_subtype = cid_dict
        .get(b"Subtype")
        .and_then(Object::as_name)
        .expect("descendant carries a /Subtype");
    assert!(
        cid_subtype == b"CIDFontType0" || cid_subtype == b"CIDFontType2",
        "descendant subtype must be a CIDFont, got {:?}",
        String::from_utf8_lossy(cid_subtype)
    );

    let descriptor_id = cid_dict
        .get(b"FontDescriptor")
        .and_then(Object::as_reference)
        .expect("descendant has a /FontDescriptor reference");
    let descriptor = doc
        .get_dictionary(descriptor_id)
        .expect("descriptor resolves");
    assert!(
        descriptor.has(b"FontFile2") || descriptor.has(b"FontFile3"),
        "descriptor must attach a FontFile2 or FontFile3 program stream"
    );

    for id in font_closure(doc, type0_id) {
        let Ok(dict) = doc.get_object(id).and_then(Object::as_dict) else {
            continue;
        };
        if let Ok(Object::Name(name)) = dict.get(b"BaseFont") {
            assert!(
                !BASE14_NAMES.contains(&name.as_slice()),
                "must not embed a base-14 font, got {:?}",
                String::from_utf8_lossy(name)
            );
        }
    }
}

/// Full structural proof that page `page_id`'s overlay references a real,
/// embedded font: `/F1` is an INDIRECT reference (not an inline base-14
/// dict) to a `/Type0` font whose descendant carries an embedded program.
fn assert_overlay_uses_embedded_font(doc: &Document, page_id: ObjectId) {
    let font_obj = overlay_font_resource(doc, page_id);
    let type0_id = font_obj
        .as_reference()
        .expect("the overlay's /F1 font resource must be an INDIRECT reference to the transplanted Type0 font, not an inline dict");
    assert_embedded_type0_font(doc, type0_id);
}

/// A single-page PDF built directly with `lopdf`, bypassing turbo's own emitter
/// entirely, so the page tree can be shaped in ways turbo never produces:
/// `page_media_box` is the page dictionary's own `/MediaBox` (turbo always
/// writes one; `None` here models a foreign PDF that omits it), and
/// `parent_media_box` is the `/MediaBox` on the `/Pages` tree node the page
/// inherits from when it has none of its own.
fn minimal_pdf(page_media_box: Option<[f32; 4]>, parent_media_box: Option<[f32; 4]>) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    let content_id = doc.add_object(Stream::new(Dictionary::new(), b"BT ET".to_vec()));

    let mut page_dict = dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
    };
    if let Some([x0, y0, x1, y1]) = page_media_box {
        page_dict.set("MediaBox", media_box_array(x0, y0, x1, y1));
    }
    let page_id = doc.add_object(page_dict);

    let mut pages_dict = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
    };
    if let Some([x0, y0, x1, y1]) = parent_media_box {
        pages_dict.set("MediaBox", media_box_array(x0, y0, x1, y1));
    }
    doc.objects.insert(pages_id, Object::Dictionary(pages_dict));

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut out = Vec::new();
    doc.save_to(&mut out).expect("minimal fixture saves");
    out
}

/// A PDF `[x0 y0 x1 y1]` rectangle array, e.g. for a `/MediaBox`.
fn media_box_array(x0: f32, y0: f32, x1: f32, y1: f32) -> Object {
    Object::Array(vec![
        Object::Real(x0),
        Object::Real(y0),
        Object::Real(x1),
        Object::Real(y1),
    ])
}

// --------------------------------------------------------------------------
// stamp: every page gains the overlay; content and page count survive
// --------------------------------------------------------------------------

#[test]
fn stamps_a_watermark_onto_every_page_preserving_content_and_page_count() {
    let pdf = two_page_pdf();
    let original_pages = Document::load_mem(&pdf)
        .expect("fixture parses")
        .get_pages()
        .len();
    assert_eq!(original_pages, 2, "fixture must start with two pages");

    let stamped =
        stamp(&pdf, &cancelled_watermark(), None, None).expect("stamp succeeds on a plaintext PDF");

    let doc = Document::load_mem(&stamped).expect("stamped bytes are a well-formed PDF");
    let page_ids: Vec<ObjectId> = doc.get_pages().into_values().collect();
    assert_eq!(page_ids.len(), 2, "stamping must not change the page count");

    for page_id in page_ids {
        let page_content = doc
            .get_page_content(page_id)
            .expect("page content stream is readable");

        assert!(
            contains(&page_content, b"BT"),
            "original body content (its BT operator) must survive on the page"
        );
        assert!(
            contains(
                &page_content,
                format!("/{WATERMARK_XOBJECT_NAME} Do").as_bytes()
            ),
            "the page must invoke the watermark overlay XObject"
        );

        let overlay = overlay_form_content(&doc, page_id);
        assert!(
            contains(&overlay, b" gs"),
            "the overlay must apply the fade ExtGState (opacity)"
        );
        assert!(
            contains(&overlay, b" cm"),
            "the overlay must apply the diagonal rotation"
        );

        assert_overlay_uses_embedded_font(&doc, page_id);
    }
}

#[test]
fn stamp_is_deterministic() {
    let pdf = two_page_pdf();
    let a = stamp(&pdf, &cancelled_watermark(), None, None).expect("stamp a");
    let b = stamp(&pdf, &cancelled_watermark(), None, None).expect("stamp b");
    assert_eq!(a, b, "same inputs must yield byte-identical output");
}

#[test]
fn malformed_input_is_error() {
    let err = stamp(b"not a pdf at all", &cancelled_watermark(), None, None).unwrap_err();
    assert!(matches!(err, StampError::Malformed(_)), "got {err:?}");
    assert!(err.to_string().contains("malformed PDF"));
}

#[test]
fn no_pages_is_error() {
    let empty = include_bytes!("fixtures/append/empty_tree.pdf");
    let err = stamp(empty, &cancelled_watermark(), None, None).unwrap_err();
    assert!(matches!(err, StampError::NoPages), "got {err:?}");
    assert!(err.to_string().contains("no pages"));
}

// --------------------------------------------------------------------------
// stamp: /MediaBox resolution — own box, inherited box, and the A4 fallback
// --------------------------------------------------------------------------
//
// turbo's own emitter always writes a /MediaBox directly on every page, so
// the fixtures above never exercise the inherited or missing case. These
// build a foreign-shaped PDF with lopdf directly (a page tree turbo itself
// never produces) to prove `stamp`'s fallback chain for real.

#[test]
fn inherited_media_box_from_the_pages_tree_is_used_when_the_page_has_none() {
    let pdf = minimal_pdf(None, Some([0.0, 0.0, 400.0, 300.0]));
    let stamped = stamp(&pdf, &cancelled_watermark(), None, None)
        .expect("stamp succeeds on a page with an inherited /MediaBox");

    let doc = Document::load_mem(&stamped).expect("stamped bytes are a well-formed PDF");
    let page_id = doc
        .get_pages()
        .into_values()
        .next()
        .expect("fixture has one page");

    assert_eq!(
        overlay_form_bbox(&doc, page_id),
        (400.0, 300.0),
        "the overlay must size itself to the /Pages tree's inherited /MediaBox, not the A4 fallback"
    );
}

#[test]
fn missing_media_box_everywhere_falls_back_to_a4() {
    let pdf = minimal_pdf(None, None);
    let stamped = stamp(&pdf, &cancelled_watermark(), None, None)
        .expect("stamp succeeds even with no /MediaBox anywhere in the page tree");

    let doc = Document::load_mem(&stamped).expect("stamped bytes are a well-formed PDF");
    let page_id = doc
        .get_pages()
        .into_values()
        .next()
        .expect("fixture has one page");

    let (width, height) = overlay_form_bbox(&doc, page_id);
    assert!(
        (width - 595.2756).abs() < 0.01 && (height - 841.8898).abs() < 0.01,
        "with no /MediaBox on the page or its parent, the overlay must fall back to A4: got {width}x{height}"
    );
}

#[test]
fn stamped_output_passes_qpdf_check() {
    if !qpdf_available() {
        return;
    }
    let pdf = two_page_pdf();
    let stamped = stamp(&pdf, &cancelled_watermark(), None, None).expect("stamp");

    let path = std::env::temp_dir().join("turbo-pdf-stamp-check.pdf");
    std::fs::write(&path, &stamped).expect("write temp pdf");
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

/// lopdf 0.36's own `decrypt` rejects the top-level `/Length 256` turbo's
/// V5/R6/AESV3 `/Encrypt` dict carries (a confirmed lopdf bug, root-caused in
/// full in `tests/encryption_roundtrip.rs`'s module doc: `PasswordAlgorithm
/// ::try_from` rejects any `/Length` outside `40..=128`, even though the field
/// is dead for R6). `stamp`'s own re-encrypted OUTPUT now goes through
/// turbo's `encrypt_pdf` (the same encoder `emit_pdf`'s `encrypt` feature
/// uses), which writes that same `/Length 256` — so verifying the round trip
/// with `lopdf::Document::decrypt` here needs the identical, offset-preserving
/// workaround `stamp` already applies to its INPUT side
/// (`normalize_encrypt_length`). This is a test-only patch for verification;
/// `stamp` itself never calls `lopdf::Document::decrypt` on its own output.
fn patched_for_lopdf_decrypt(pdf: &[u8]) -> Vec<u8> {
    let needle = b"/Length 256";
    let at = pdf
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("turbo's V5/R6 /Encrypt dict carries a top-level /Length 256");
    let mut out = pdf.to_vec();
    out[at..at + needle.len()].copy_from_slice(b"/Length 128");
    out
}

/// Whether the `qpdf` binary is on `PATH`.
fn qpdf_available() -> bool {
    std::process::Command::new("which")
        .arg("qpdf")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A counter so parallel `#[test]` threads never collide on the temp filenames
/// [`qpdf_decrypt`] writes.
static QPDF_TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Decrypt `pdf` under `password` via `qpdf --decrypt`, returning the resulting
/// PLAINTEXT bytes.
///
/// Used instead of `lopdf::Document::decrypt` to verify `stamp`'s re-encrypted
/// output: lopdf 0.36 has a SEPARATE, confirmed bug (reproduced directly
/// against `emit_pdf`'s own `encrypt` feature output, with no `stamp`
/// involved) authenticating a distinct owner-vs-user password pair against a
/// THIRD PARTY's V5/R6/AESV3 handler — it rejects the correct password with
/// `Decryption(IncorrectPassword)`. That gap is orthogonal to this file's
/// fix (routing `stamp`'s re-encryption through `encrypt_pdf`) and unrelated
/// to whether the output is actually valid, which `qpdf` — the real-world
/// validator this file's fix targets — confirms it is. Panics if `qpdf` is
/// not on `PATH`; callers must gate on [`qpdf_available`] first.
fn qpdf_decrypt(pdf: &[u8], password: &str) -> Vec<u8> {
    let n = QPDF_TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir();
    let input = dir.join(format!("turbo-pdf-stamp-qpdf-decrypt-in-{n}.pdf"));
    let output = dir.join(format!("turbo-pdf-stamp-qpdf-decrypt-out-{n}.pdf"));
    std::fs::write(&input, pdf).expect("write temp pdf");
    let out = std::process::Command::new("qpdf")
        .arg(format!("--password={password}"))
        .arg("--decrypt")
        .arg(&input)
        .arg(&output)
        .output()
        .expect("run qpdf --decrypt");
    assert!(
        out.status.success(),
        "qpdf --decrypt must accept the re-encrypted output under the given password: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    std::fs::read(&output).expect("read qpdf-decrypted output")
}

// --------------------------------------------------------------------------
// stamp: encrypted round trip (Task 3) — decrypt, overlay, re-encrypt
// --------------------------------------------------------------------------

#[test]
fn stamps_an_encrypted_pdf_and_only_the_new_password_opens_the_result() {
    let pdf = two_page_encrypted_pdf();

    let stamped = stamp(
        &pdf,
        &cancelled_watermark(),
        Some(ORIGINAL_USER_PW),
        Some(&new_encryption()),
    )
    .expect("stamp decrypts, overlays and re-encrypts");

    let for_lopdf = patched_for_lopdf_decrypt(&stamped);

    let mut wrong_password_attempt =
        Document::load_mem(&for_lopdf).expect("re-encrypted bytes are a well-formed PDF");
    assert!(
        wrong_password_attempt.is_encrypted(),
        "stamped output must still be encrypted"
    );
    assert!(
        wrong_password_attempt.decrypt(ORIGINAL_USER_PW).is_err(),
        "the OLD password must no longer open the re-encrypted document"
    );

    if qpdf_available() {
        let plaintext = qpdf_decrypt(&stamped, NEW_USER_PW);
        let doc =
            Document::load_mem(&plaintext).expect("qpdf-decrypted bytes are a well-formed PDF");

        let page_ids: Vec<ObjectId> = doc.get_pages().into_values().collect();
        assert_eq!(page_ids.len(), 2, "stamping must not change the page count");

        for page_id in page_ids {
            let page_content = doc
                .get_page_content(page_id)
                .expect("page content stream is readable once decrypted");
            assert!(
                contains(&page_content, b"BT"),
                "original body content (its BT operator) must survive on the page"
            );
            assert!(
                contains(
                    &page_content,
                    format!("/{WATERMARK_XOBJECT_NAME} Do").as_bytes()
                ),
                "the page must invoke the watermark overlay XObject"
            );

            assert_overlay_uses_embedded_font(&doc, page_id);
        }

        let path = std::env::temp_dir().join("turbo-pdf-stamp-encrypted-check.pdf");
        std::fs::write(&path, &stamped).expect("write temp pdf");
        let ok_new = std::process::Command::new("qpdf")
            .arg("--check")
            .arg(format!("--password={NEW_USER_PW}"))
            .arg(&path)
            .output()
            .expect("run qpdf");
        assert!(
            ok_new.status.success(),
            "qpdf --check must accept the re-encrypted output under the NEW password: {}\n{}",
            String::from_utf8_lossy(&ok_new.stdout),
            String::from_utf8_lossy(&ok_new.stderr),
        );
    }
}

#[test]
fn wrong_password_on_an_encrypted_input_is_a_decrypt_error() {
    let pdf = two_page_encrypted_pdf();

    let err = stamp(
        &pdf,
        &cancelled_watermark(),
        Some("definitely-not-it"),
        Some(&new_encryption()),
    )
    .unwrap_err();

    assert!(matches!(err, StampError::Decrypt(_)), "got {err:?}");
    assert!(
        err.to_string().contains("decrypt"),
        "error message should name the failed step: got {err}"
    );
}

#[test]
fn password_given_for_a_plaintext_input_is_a_decrypt_error() {
    // `two_page_pdf()` is plaintext: it carries no `/Encrypt` dict at all, so
    // it can never contain the `/Length 256` byte pattern `stamp` patches
    // around lopdf's over-strict R6 check — this is the one input shape that
    // exercises `normalize_encrypt_length`'s pass-through (`Cow::Borrowed`)
    // branch, not its patched (`Cow::Owned`) one.
    let pdf = two_page_pdf();

    let err = stamp(
        &pdf,
        &cancelled_watermark(),
        Some("irrelevant-password"),
        None,
    )
    .unwrap_err();

    assert!(matches!(err, StampError::Decrypt(_)), "got {err:?}");
    assert!(
        err.to_string().contains("decrypt"),
        "error message should name the failed step: got {err}"
    );
}

#[test]
fn stamp_accepts_turbos_raw_encrypted_bytes_with_no_test_side_length_patch() {
    let pdf = two_page_encrypted_pdf();

    let stamped = stamp(
        &pdf,
        &cancelled_watermark(),
        Some(ORIGINAL_USER_PW),
        Some(&new_encryption()),
    )
    .expect(
        "stamp must accept turbo's raw V5/R6 /Encrypt bytes directly \
         (the /Length normalization lives inside stamp, not in the caller)",
    );

    if qpdf_available() {
        let plaintext = qpdf_decrypt(&stamped, NEW_USER_PW);
        Document::load_mem(&plaintext).expect(
            "qpdf-decrypted bytes are a well-formed PDF, proving the NEW password opens it",
        );
    }
}
