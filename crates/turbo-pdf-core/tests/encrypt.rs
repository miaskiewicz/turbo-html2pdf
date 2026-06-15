//! `encrypt` feature tests (PDF 2.0 AES-256, V5/R6 / AESV3). Built only with
//! `--features encrypt`. Encryption is intentionally non-deterministic (random
//! salts/IVs), so these tests assert the *round trip* and the `/Encrypt` dict
//! structure — never byte-equality of the encrypted output.
//!
//! When `qpdf` is on PATH the real validator runs: an encrypted PDF (a) opens +
//! `--check`s clean with the right password, and (b) is rejected with the wrong
//! one. Both the user-password-only and owner+user shapes are exercised. Without
//! qpdf we fall back to asserting the dict (V5/R6/AESV3, U/UE/O/OE/Perms present
//! at the right lengths) and that the info-dict title is no longer plaintext.
#![cfg(feature = "encrypt")]

mod common;

use std::io::Write;
use std::process::Command;

use turbo_pdf_core::layout::fragment::{Fragment, FragmentContent, NodeId, PositionedGlyph};
use turbo_pdf_core::layout::value::Rgba;
use turbo_pdf_core::paginate::{Page, PageGeometry};
use turbo_pdf_core::{emit_pdf, EmitOptions, Encryption, FontFace, Permissions};

// --------------------------------------------------------------------------
// fixtures
// --------------------------------------------------------------------------

/// A single A4 page wrapping the given body fragments.
fn page_with(body: Vec<Fragment>) -> Page {
    Page {
        geometry: PageGeometry::a4(),
        kind: turbo_pdf_core::PageKind::First,
        number: 1,
        body,
        header: Vec::new(),
        footer: Vec::new(),
        footnotes: Vec::new(),
    }
}

/// A body text line so the document has a real (font + content) stream to encrypt.
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

/// Emit options carrying an info title (forces a literal/hex string in the info
/// dict that the encryptor must rewrite) plus the given encryption.
fn opts(enc: Encryption) -> EmitOptions {
    EmitOptions {
        // A non-ASCII char makes pdf-writer emit a UTF-16 hex string, so both the
        // literal and hex string-scanning paths get exercised across the suite.
        title: Some("Quarterly Report \u{2014} Q3".to_string()),
        author: Some("turbo".to_string()),
        encryption: Some(enc),
        ..EmitOptions::default()
    }
}

fn user_only(pw: &str) -> Encryption {
    Encryption {
        user_password: pw.to_string(),
        owner_password: None,
        permissions: Permissions::all(),
    }
}

fn owner_user(user: &str, owner: &str) -> Encryption {
    Encryption {
        user_password: user.to_string(),
        owner_password: Some(owner.to_string()),
        permissions: Permissions {
            print: false,
            copy: false,
            ..Permissions::all()
        },
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// --------------------------------------------------------------------------
// structure: the /Encrypt dictionary
// --------------------------------------------------------------------------

#[test]
fn writes_v5_r6_aesv3_encrypt_dict() {
    let pdf = emit_pdf(&sample_pages(), &opts(user_only("open-me")));
    assert!(pdf.starts_with(b"%PDF-1.7"), "header preserved");
    assert!(contains(&pdf, b"%%EOF"), "trailer present");
    assert!(contains(&pdf, b"/Filter /Standard"), "standard handler");
    assert!(contains(&pdf, b"/V 5"), "V=5");
    assert!(contains(&pdf, b"/R 6"), "R=6");
    assert!(contains(&pdf, b"/Length 256"), "256-bit key");
    assert!(contains(&pdf, b"/CFM /AESV3"), "AESV3 crypt filter");
    assert!(contains(&pdf, b"/StmF /StdCF"), "streams via StdCF");
    assert!(contains(&pdf, b"/StrF /StdCF"), "strings via StdCF");
    for key in [&b"/U "[..], b"/UE ", b"/O ", b"/OE ", b"/Perms "] {
        assert!(contains(&pdf, key), "missing key-material entry");
    }
    assert!(contains(&pdf, b"/Encrypt "), "trailer references /Encrypt");
    assert!(contains(&pdf, b"/ID ["), "trailer has a file /ID");
}

#[test]
fn key_material_has_the_spec_lengths() {
    let pdf = emit_pdf(&sample_pages(), &opts(user_only("pw")));
    // /U and /O are 48 bytes -> 96 hex digits; /UE,/OE,/Perms are 32/32/16.
    assert_eq!(hex_len_after(&pdf, b"/U "), 96, "/U is 48 bytes");
    assert_eq!(hex_len_after(&pdf, b"/O "), 96, "/O is 48 bytes");
    assert_eq!(hex_len_after(&pdf, b"/UE "), 64, "/UE is 32 bytes");
    assert_eq!(hex_len_after(&pdf, b"/OE "), 64, "/OE is 32 bytes");
    assert_eq!(hex_len_after(&pdf, b"/Perms "), 32, "/Perms is 16 bytes");
}

/// Length (in hex digits) of the `<...>` hex string immediately after `key`.
fn hex_len_after(pdf: &[u8], key: &[u8]) -> usize {
    let at = find(pdf, key).expect("key present") + key.len();
    let open = at + pdf[at..].iter().position(|&b| b == b'<').expect("hex open");
    let close = open
        + pdf[open..]
            .iter()
            .position(|&b| b == b'>')
            .expect("hex close");
    close - open - 1
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

// --------------------------------------------------------------------------
// the plaintext is gone
// --------------------------------------------------------------------------

#[test]
fn plaintext_strings_and_streams_are_encrypted() {
    let plain = emit_pdf(&sample_pages(), &EmitOptions::default());
    let enc = emit_pdf(&sample_pages(), &opts(user_only("pw")));
    // The unencrypted producer/author literals must not survive in the open.
    assert!(
        contains(&plain, b"turbo-pdf"),
        "plaintext baseline has producer"
    );
    assert!(
        !contains(&enc, b"(turbo-pdf)"),
        "producer literal encrypted"
    );
    assert!(!contains(&enc, b"(turbo)"), "author literal encrypted");
    // A content stream's text operators must not appear in the clear.
    assert!(contains(&plain, b"\nBT\n"), "baseline has a text block");
    assert!(!contains(&enc, b"\nBT\n"), "content stream encrypted");
}

#[test]
fn permissions_round_trip_through_p() {
    // print+copy cleared -> their bits are 0 in /P; granted bits stay 1.
    let p = Permissions {
        print: false,
        copy: false,
        ..Permissions::all()
    }
    .to_p() as u32;
    assert_eq!(p & (1 << 2), 0, "print bit (3) cleared");
    assert_eq!(p & (1 << 4), 0, "copy bit (5) cleared");
    assert_ne!(p & (1 << 3), 0, "modify bit (4) granted");
    assert_eq!(
        Permissions::all().to_p() as u32 & 0xF3C,
        0xF3C,
        "all grants its bits"
    );
    assert_eq!(
        Permissions::default(),
        Permissions::all(),
        "default grants all"
    );
}

// --------------------------------------------------------------------------
// qpdf round trip (only when qpdf is on PATH)
// --------------------------------------------------------------------------

#[test]
fn user_password_round_trips_through_qpdf() {
    let Some(qpdf) = qpdf_path() else {
        eprintln!("qpdf not on PATH; skipping round-trip");
        return;
    };
    let pdf = emit_pdf(&sample_pages(), &opts(user_only("s3cret")));
    let path = write_temp("enc-user.pdf", &pdf);
    assert!(
        opens_with(&qpdf, &path, Some("s3cret")),
        "opens with right pw"
    );
    assert!(check_clean(&qpdf, &path, "s3cret"), "qpdf --check is clean");
    assert!(
        !opens_with(&qpdf, &path, None),
        "rejected without a password"
    );
    assert!(
        !opens_with(&qpdf, &path, Some("wrong")),
        "rejected with wrong pw"
    );
}

#[test]
fn owner_and_user_passwords_round_trip_through_qpdf() {
    let Some(qpdf) = qpdf_path() else {
        eprintln!("qpdf not on PATH; skipping round-trip");
        return;
    };
    let pdf = emit_pdf(&sample_pages(), &opts(owner_user("user-pw", "owner-pw")));
    let path = write_temp("enc-owner.pdf", &pdf);
    // Both passwords open the document; the owner password grants full rights.
    assert!(opens_with(&qpdf, &path, Some("user-pw")), "user pw opens");
    assert!(opens_with(&qpdf, &path, Some("owner-pw")), "owner pw opens");
    assert!(
        check_clean(&qpdf, &path, "owner-pw"),
        "qpdf --check is clean"
    );
    assert!(!opens_with(&qpdf, &path, Some("nope")), "wrong pw rejected");
}

fn qpdf_path() -> Option<String> {
    let out = Command::new("which").arg("qpdf").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

fn write_temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("turbo-pdf-{}-{name}", std::process::id()));
    let mut f = std::fs::File::create(&path).expect("create temp pdf");
    f.write_all(bytes).expect("write temp pdf");
    path
}

/// `true` iff qpdf can decrypt `path` with `password` (None = supply none).
///
/// qpdf exit codes: 0 = success, 3 = success-with-warnings, 2 = error. A wrong
/// or missing password is an *error* (2); a valid open may emit warnings (3) when
/// the encrypted streams confuse its pre-decrypt tokenizer, but still succeeds —
/// so "opened" is "exit code is not the error code".
fn opens_with(qpdf: &str, path: &std::path::Path, password: Option<&str>) -> bool {
    let mut cmd = Command::new(qpdf);
    if let Some(pw) = password {
        cmd.arg(format!("--password={pw}"));
    }
    cmd.arg("--decrypt").arg(path).arg("-");
    matches!(exit_code(&mut cmd), Some(0) | Some(3))
}

/// `true` iff `qpdf --check` (with the password) finds no structural errors
/// (exit 0 or warnings-only 3; an error is exit 2).
fn check_clean(qpdf: &str, path: &std::path::Path, password: &str) -> bool {
    let mut cmd = Command::new(qpdf);
    cmd.arg(format!("--password={password}"))
        .arg("--check")
        .arg(path);
    matches!(exit_code(&mut cmd), Some(0) | Some(3))
}

/// Run `cmd` to completion and return its process exit code.
fn exit_code(cmd: &mut Command) -> Option<i32> {
    cmd.output().ok().and_then(|o| o.status.code())
}
