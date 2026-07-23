//! Feasibility spike (Task 1 of the `stamp()` design): can a PDF that turbo's
//! own `encrypt` feature password-protected be decrypted and re-encrypted by
//! something already in the dependency graph, so a later `stamp()` can open an
//! encrypted PDF, paint a watermark into it, and hand back a document that is
//! still protected?
//!
//! **DECISION: `lopdf` 0.36 (already pulled in by this crate's `append`
//! feature) suffices — its decrypt/encrypt engine fully implements the same
//! handler turbo does — but it needs ONE targeted, documented workaround to
//! accept turbo's exact bytes. No new crate is needed.**
//!
//! turbo's emitter (`emit::encrypt::encrypt_pdf`) hand-implements the PDF 2.0
//! Standard Security Handler V=5/R=6 (AESV3, the ISO 32000-2 §7.6.4.3
//! "hardened" Algorithm 2.B hash). lopdf 0.36 ships a full implementation of
//! the *same* handler (`src/encryption.rs` + `src/encryption/algorithms.rs`),
//! exposed as:
//!
//! - **Decrypt**: `lopdf::Document::load_mem(bytes)` to parse the encrypted
//!   file (turbo writes a classic xref table + trailer, which is exactly what
//!   `load_mem` expects), then `doc.is_encrypted() -> bool` and
//!   `doc.decrypt(password: &str) -> lopdf::Result<()>`. `decrypt` runs
//!   `PasswordAlgorithm::try_from(&doc)` (reads `/Encrypt`'s `/V`, `/R`, `/O`,
//!   `/U`, `/OE`, `/UE`, `/CF`, `/StmF`, `/StrF`, `/P`), authenticates the
//!   password against `/U`/`/O`, then walks every object and replaces its
//!   strings/streams with plaintext in place.
//! - **Re-encrypt**: build an `lopdf::EncryptionVersion::V5 { .. }` (crypt
//!   filter `lopdf::encryption::crypt_filters::Aes256CryptFilter`, a fresh
//!   32-byte file-encryption key, owner/user passwords, `lopdf::Permissions`),
//!   turn it into an `lopdf::EncryptionState` via `TryFrom`, then
//!   `doc.encrypt(&state) -> lopdf::Result<()>`. `doc.save_to`/`doc.save_mem`
//!   then serialises a fresh classic-xref PDF carrying a new `/Encrypt` dict.
//!
//! **The workaround (a confirmed lopdf 0.36 bug, not a missing capability).**
//! Fed turbo's *unmodified* output, `doc.decrypt(password)` fails with
//! `Decryption(InvalidKeyLength)`. Root cause, read directly in lopdf 0.36:
//! `PasswordAlgorithm::try_from(&Document)`
//! (`src/encryption/algorithms.rs:104-114`) unconditionally rejects any
//! `/Encrypt` dict whose top-level `/Length` falls outside `40..=128` —
//! *regardless of `/V`/`/R`* — even though that field is read ONLY by the
//! legacy R2-R4 MD5 key derivation (`self.length.unwrap_or(40) / 8` at
//! `algorithms.rs:309`, `:603`, `:840`, all unreachable for R6). turbo (like
//! Adobe Acrobat and most real-world V5/AES-256 encoders) writes `/Length 256`
//! for documentation/compat even though ISO 32000-2 Table 20 only mandates the
//! key for `V` 2 or 3; lopdf's own R5/R6 *encoder* (`EncryptionState::encode`
//! at `encryption.rs:624`) in fact never emits `/Length` at all for V5, which
//! is exactly why lopdf-encrypted-then-lopdf-decrypted round trips never hit
//! this — only interop with a third party's V5 output does. [`patched_for_lopdf`]
//! below applies the one-line, offset-preserving fix demonstrating the exact
//! shape `stamp()` would need: rewrite the ASCII digits after the top-level
//! `/Length` key (kept out-of-band from the `/CF/StdCF/Length 32` crypt-filter
//! entry, which is untouched and semantically different) to any in-range
//! value; since the field is provably unused for R6, this changes nothing
//! about the actual AES-256 key material or ciphertext. This should be
//! reported upstream to `lopdf`; failing that, turbo's `emit::encrypt`
//! serializer could stop writing the top-level `/Length` for V5/R6 (spec-legal
//! since Table 20 only requires it for V 2/3) — left to a human call since
//! that changes an already-shipped feature's byte output and its existing
//! `tests/encrypt.rs` assertion (`/Length 256`).
//!
//! Both directions are proven below end to end against a *real* fixture built
//! through turbo's actual render + `Encryption` path (not a hand-rolled PDF):
//! decrypt recovers a known marker from both the Info dict (`/Title`, an
//! encrypted PDF string) and the page content stream (the encrypted `BT`
//! operator bytes), a wrong password is rejected, and the decrypted document
//! re-encrypted through lopdf only re-opens with its own new password.
//!
//! Built only with `--features encrypt,append`: `encrypt` is turbo's own
//! password-protection emitter (the fixture producer), `append` is what makes
//! `lopdf` a compiled dependency of this crate at all.
#![cfg(all(feature = "encrypt", feature = "append"))]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{
    Dictionary, Document, EncryptionState, EncryptionVersion, Permissions as LopdfPermissions,
};

use turbo_html2pdf_core::layout::fragment::{Fragment, FragmentContent, NodeId, PositionedGlyph};
use turbo_html2pdf_core::layout::value::Rgba;
use turbo_html2pdf_core::paginate::{Page, PageGeometry};
use turbo_html2pdf_core::{emit_pdf, EmitOptions, Encryption, FontFace, Permissions};

// --------------------------------------------------------------------------
// fixture: a real encrypted PDF from turbo's own render + Encryption path
// --------------------------------------------------------------------------

const MARKER: &str = "ROUNDTRIP-MARKER-4711";
const USER_PW: &str = "s3cret-user-pw";

/// A single A4 page wrapping the given body fragments (mirrors `tests/encrypt.rs`).
fn page_with(body: Vec<Fragment>) -> Page {
    Page {
        geometry: PageGeometry::a4(),
        kind: turbo_html2pdf_core::PageKind::First,
        number: 1,
        body,
        header: Vec::new(),
        footer: Vec::new(),
        footnotes: Vec::new(),
    }
}

/// A body text line so the document has a real (font + content) stream to
/// decrypt back out.
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

fn sample_pages() -> Vec<Page> {
    vec![page_with(vec![body_text(common::evolventa())])]
}

/// Render a small encrypted PDF (user password only) whose Info `/Title`
/// carries `MARKER` in the clear before encryption.
fn encrypted_fixture() -> Vec<u8> {
    let enc = Encryption {
        user_password: USER_PW.to_string(),
        owner_password: None,
        permissions: Permissions::all(),
    };
    let opts = EmitOptions {
        title: Some(MARKER.to_string()),
        encryption: Some(enc),
        ..EmitOptions::default()
    };
    emit_pdf(&sample_pages(), &opts)
}

// --------------------------------------------------------------------------
// helpers
// --------------------------------------------------------------------------

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Work around the lopdf 0.36 `/Length`-range validation bug documented in the
/// module header: rewrite the top-level `/Encrypt` dict's `/Length 256` (an
/// R6-unused, spec-optional-for-V5 field) to an in-range value, byte-for-byte
/// the same length so no object offset in the file shifts. `/CF/StdCF/Length
/// 32` (the crypt filter's own, semantically different, byte-length entry) is
/// untouched because it does not match this exact, unique substring.
fn patched_for_lopdf(pdf: &[u8]) -> Vec<u8> {
    let needle = b"/Length 256";
    let at = pdf
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("turbo's V5/R6 /Encrypt dict carries a top-level /Length 256");
    let mut out = pdf.to_vec();
    // Swap the R6-unused key length to an in-range value ONLY to satisfy
    // lopdf 0.36's over-strict `/Length` check — same byte length so no offset
    // shifts, and the field is dead for R6, so AES-256 key/ciphertext are untouched.
    out[at..at + needle.len()].copy_from_slice(b"/Length 128");
    out
}

/// The `/Info` dictionary referenced by the trailer, if present.
fn info_dict(doc: &Document) -> Option<&Dictionary> {
    let info_id = doc.trailer.get(b"Info").ok()?.as_reference().ok()?;
    doc.get_dictionary(info_id).ok()
}

/// The decrypted `Info`/`Title` string, read back through lopdf's object graph.
fn info_title(doc: &Document) -> Option<String> {
    let dict = info_dict(doc)?;
    let bytes = dict.get(b"Title").ok()?.as_str().ok()?;
    Some(String::from_utf8_lossy(bytes).into_owned())
}

/// The first page's decrypted content-stream bytes.
fn first_page_content(doc: &Document) -> Vec<u8> {
    let page_id = doc
        .get_pages()
        .into_values()
        .next()
        .expect("fixture has one page");
    doc.get_page_content(page_id)
        .expect("page content stream is readable once decrypted")
}

/// Build a fresh V5/AES-256 `EncryptionState` (a new random file-encryption
/// key each call, exactly the entropy turbo's own encryptor waives the
/// "no randomness" rule for).
fn v5_state(user_password: &str, owner_password: &str) -> EncryptionState {
    let mut file_encryption_key = [0u8; 32];
    getrandom::getrandom(&mut file_encryption_key).expect("OS CSPRNG must be available");
    let crypt_filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let version = EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), crypt_filter)]),
        file_encryption_key: &file_encryption_key,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password,
        user_password,
        permissions: LopdfPermissions::all(),
    };
    EncryptionState::try_from(version).expect("V5 crypt filter/state is always constructible")
}

// --------------------------------------------------------------------------
// decrypt: lopdf recovers the marker turbo encrypted
// --------------------------------------------------------------------------

#[test]
fn lopdf_decrypts_turbos_v5_r6_aesv3_output_and_recovers_the_marker() {
    let pdf = patched_for_lopdf(&encrypted_fixture());

    let mut doc = Document::load_mem(&pdf).expect("lopdf parses turbo's encrypted PDF");
    assert!(doc.is_encrypted(), "fixture is actually encrypted");

    doc.decrypt(USER_PW)
        .expect("lopdf decrypts turbo's V5/R6/AESV3 handler with the right password");

    assert_eq!(
        info_title(&doc).as_deref(),
        Some(MARKER),
        "decrypted Info/Title recovers the known marker"
    );

    let body = first_page_content(&doc);
    assert!(
        contains(&body, b"BT"),
        "decrypted content stream recovers the text-showing operator"
    );
}

#[test]
fn lopdf_rejects_the_wrong_password() {
    let pdf = patched_for_lopdf(&encrypted_fixture());
    let mut doc = Document::load_mem(&pdf).expect("lopdf parses turbo's encrypted PDF");

    let err = doc.decrypt("definitely-not-it");
    assert!(
        err.is_err(),
        "a wrong password must not be accepted as a decrypt key"
    );
}

// --------------------------------------------------------------------------
// re-encrypt: lopdf can hand back a document only the NEW password opens
// --------------------------------------------------------------------------

#[test]
fn lopdf_reencrypts_the_decrypted_document_and_only_the_new_password_opens_it() {
    let pdf = patched_for_lopdf(&encrypted_fixture());

    let mut doc = Document::load_mem(&pdf).expect("lopdf parses turbo's encrypted PDF");
    doc.decrypt(USER_PW)
        .expect("decrypt with the original password");

    let new_user_pw = "brand-new-user-pw";
    let new_owner_pw = "brand-new-owner-pw";
    let state = v5_state(new_user_pw, new_owner_pw);
    doc.encrypt(&state)
        .expect("lopdf re-encrypts the now-plaintext document under a fresh V5 handler");

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .expect("lopdf serialises the re-encrypted document");

    let mut reopened =
        Document::load_mem(&out).expect("the re-encrypted bytes are a well-formed PDF");
    assert!(
        reopened.is_encrypted(),
        "re-encrypted output is protected again"
    );

    assert!(
        reopened.decrypt("definitely-not-it").is_err(),
        "the old/garbage password must not open the re-encrypted document"
    );

    let mut reopened =
        Document::load_mem(&out).expect("the re-encrypted bytes are a well-formed PDF");
    reopened
        .decrypt(new_user_pw)
        .expect("the NEW password opens the re-encrypted document");
    assert_eq!(
        info_title(&reopened).as_deref(),
        Some(MARKER),
        "the marker survives decrypt -> re-encrypt -> decrypt intact"
    );
}
