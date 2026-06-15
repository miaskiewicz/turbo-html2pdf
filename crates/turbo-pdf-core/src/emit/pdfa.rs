//! PDF/A-2b archival conformance (AC-11.2), gated behind `#[cfg(feature =
//! "pdf-a")]`. The "keep-forever" PDF: everything needed to render the file
//! identically in decades is baked in, and anything that could render
//! differently later is forbidden.
//!
//! This module adds the three pieces the default emitter omits:
//!
//! 1. A vendored **sRGB ICC profile** (`assets/icc/sRGB-IEC61966-2.1.icc`, the
//!    canonical IEC 61966-2.1 monitor profile) written as an `ICCBased` stream
//!    and referenced from an **OutputIntent** (`GTS_PDFA`, output condition
//!    `sRGB IEC61966-2.1`). This pins the meaning of the document's DeviceRGB
//!    colours.
//! 2. An **XMP metadata packet** (RDF/XML) declaring `pdfaid:part=2`,
//!    `pdfaid:conformance=B`, with `dc:`/`xmp:`/`pdf:` properties kept
//!    consistent with the info dictionary (veraPDF clause 6.1 metadata
//!    consistency). The packet is built from the same [`EmitOptions`] and the
//!    same date sentinel as the info dict, so output stays byte-deterministic
//!    (AC-7.6) — no clock, no entropy.
//! 3. A trailer **`/ID`** (required by PDF/A), derived deterministically from
//!    the document metadata so identical inputs round-trip to identical bytes.
//!
//! The remaining PDF/A-2b rules are enforced elsewhere under the same gate:
//! **no transparency** (the watermark's `/ca` fade is disabled — see
//! `watermark::opacity` and `document::write_resources`), **device colour via
//! the OutputIntent** (the default build is already DeviceRGB-only), and **all
//! fonts embedded** (already true since Phase 9 — PDF/A mainly *forbids*
//! leaving fonts out).

use pdf_writer::types::OutputIntentSubtype;
use pdf_writer::{Finish, Pdf, Ref, TextStr};

use super::meta::{xmp_timestamp, PRODUCER};
use super::{EmitOptions, SENTINEL_DATE};

/// The vendored sRGB ICC profile (IEC 61966-2.1), embedded at compile time so
/// the archival colour space travels inside the binary — no runtime file
/// dependency, and byte-identical across builds.
const SRGB_ICC: &[u8] = include_bytes!("../../assets/icc/sRGB-IEC61966-2.1.icc");

/// The well-known output-condition identifier for the embedded profile.
const OUTPUT_CONDITION: &str = "sRGB IEC61966-2.1";

/// Write the OutputIntent (`GTS_PDFA`, level-B archival) into the catalog's
/// `/OutputIntents` array, pointing `DestOutputProfile` at the ICC stream that
/// [`write_icc_profile`] writes under `icc`, and reference the XMP packet at
/// `metadata` via the catalog's `/Metadata` entry.
pub fn write_catalog_entries(catalog: &mut pdf_writer::writers::Catalog, icc: Ref, metadata: Ref) {
    catalog.metadata(metadata);
    let mut intents = catalog.output_intents();
    intents
        .push()
        .subtype(OutputIntentSubtype::PDFA)
        .output_condition_identifier(TextStr(OUTPUT_CONDITION))
        .output_condition(TextStr(OUTPUT_CONDITION))
        .info(TextStr(OUTPUT_CONDITION))
        .dest_output_profile(icc);
    intents.finish();
}

/// Write the embedded sRGB profile as an `ICCBased` stream object at `icc`. The
/// `/N 3` component count and `sRGB` alternate are required for PDF/A.
pub fn write_icc_profile(pdf: &mut Pdf, icc: Ref) {
    let mut profile = pdf.icc_profile(icc, SRGB_ICC);
    profile.n(3);
    profile.alternate().srgb();
    profile.range([0.0, 1.0, 0.0, 1.0, 0.0, 1.0]);
    profile.finish();
}

/// Write the XMP metadata packet as a `/Metadata` stream object at `metadata`,
/// built from the same options/date as the info dict so the two agree.
pub fn write_metadata(pdf: &mut Pdf, metadata: Ref, opts: &EmitOptions) {
    let packet = xmp_packet(opts);
    pdf.metadata(metadata, packet.as_bytes());
}

/// A deterministic trailer `/ID` pair, required by PDF/A. Both halves are the
/// same 16-byte digest of the document metadata: with no clock or entropy to
/// draw on, a stable hash keeps the file byte-reproducible while still being a
/// well-formed file identifier.
pub fn file_id(opts: &EmitOptions) -> (Vec<u8>, Vec<u8>) {
    let digest = metadata_digest(opts).to_vec();
    (digest.clone(), digest)
}

/// A 16-byte FNV-1a-derived digest over the metadata that distinguishes one
/// document from another. Pure and total — same inputs, same bytes.
fn metadata_digest(opts: &EmitOptions) -> [u8; 16] {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for field in [&opts.title, &opts.author, &opts.subject, &opts.keywords] {
        fnv_bytes(&mut hash, field.as_deref().unwrap_or("").as_bytes());
        fnv_bytes(&mut hash, &[0]);
    }
    fnv_bytes(&mut hash, PRODUCER.as_bytes());
    fnv_bytes(
        &mut hash,
        &opts.creation_date.unwrap_or(SENTINEL_DATE).to_be_bytes(),
    );
    // Spread the 64-bit state across 16 bytes by mixing a second derived word.
    let mut second = hash;
    fnv_bytes(&mut second, &hash.to_be_bytes());
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&hash.to_be_bytes());
    id[8..].copy_from_slice(&second.to_be_bytes());
    id
}

/// Fold `bytes` into the running FNV-1a hash.
fn fnv_bytes(hash: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *hash ^= u64::from(b);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

/// Build the XMP RDF/XML packet for the document. One `rdf:Description` carries
/// every namespace (dc/xmp/pdf/pdfaid) — valid RDF and the form veraPDF's
/// consistency check expects — mirroring the info dict's
/// title/author/subject/keywords/producer plus the create/modify dates, and
/// declaring the PDF/A-2b conformance (`pdfaid:part=2`, `conformance=B`).
fn xmp_packet(opts: &EmitOptions) -> String {
    let date = xmp_timestamp(opts.creation_date.unwrap_or(SENTINEL_DATE));
    let mut body = String::new();
    push_dc_title(&mut body, opts.title.as_deref());
    push_dc_creator(&mut body, opts.author.as_deref());
    push_dc_description(&mut body, opts.subject.as_deref());
    push_dates(&mut body, &date);
    push_producer(&mut body);
    push_keywords(&mut body, opts.keywords.as_deref());
    push_pdfaid(&mut body);
    wrap_packet(&body)
}

/// Wrap the RDF property body in the `<?xpacket?>` envelope, the `<x:xmpmeta>`
/// root and one all-namespaces `<rdf:Description>`.
fn wrap_packet(body: &str) -> String {
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
         <rdf:Description rdf:about=\"\"\n\
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n\
         xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n\
         xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"\n\
         xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\">\n\
         {body}</rdf:Description>\n\
         </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>"
    )
}

/// `dc:title` is a language-alternative; omit it entirely when there is no
/// title, so the packet carries no empty property to disagree with the info
/// dict.
fn push_dc_title(out: &mut String, title: Option<&str>) {
    if let Some(t) = title {
        out.push_str("<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">");
        push_escaped(out, t);
        out.push_str("</rdf:li></rdf:Alt></dc:title>\n");
    }
}

/// `dc:creator` is an ordered list (`rdf:Seq`); the info dict's `/Author` maps
/// to a single entry.
fn push_dc_creator(out: &mut String, author: Option<&str>) {
    if let Some(a) = author {
        out.push_str("<dc:creator><rdf:Seq><rdf:li>");
        push_escaped(out, a);
        out.push_str("</rdf:li></rdf:Seq></dc:creator>\n");
    }
}

/// `dc:description` mirrors the info dict's `/Subject` (a language-alternative).
fn push_dc_description(out: &mut String, subject: Option<&str>) {
    if let Some(s) = subject {
        out.push_str("<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">");
        push_escaped(out, s);
        out.push_str("</rdf:li></rdf:Alt></dc:description>\n");
    }
}

/// The `xmp:` create/modify dates, both the info dict's creation date.
fn push_dates(out: &mut String, date: &str) {
    out.push_str("<xmp:CreateDate>");
    out.push_str(date);
    out.push_str("</xmp:CreateDate>\n<xmp:ModifyDate>");
    out.push_str(date);
    out.push_str("</xmp:ModifyDate>\n");
}

/// `pdf:Producer`, always present and equal to the info dict's `/Producer`.
fn push_producer(out: &mut String) {
    out.push_str("<pdf:Producer>");
    push_escaped(out, PRODUCER);
    out.push_str("</pdf:Producer>\n");
}

/// `pdf:Keywords`, mirroring the info dict's `/Keywords` when present.
fn push_keywords(out: &mut String, keywords: Option<&str>) {
    if let Some(k) = keywords {
        out.push_str("<pdf:Keywords>");
        push_escaped(out, k);
        out.push_str("</pdf:Keywords>\n");
    }
}

/// The `pdfaid:` part and conformance — the heart of the PDF/A claim.
fn push_pdfaid(out: &mut String) {
    out.push_str("<pdfaid:part>2</pdfaid:part>\n<pdfaid:conformance>B</pdfaid:conformance>\n");
}

/// Append `s` to `out`, escaping the five XML metacharacters.
fn push_escaped(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
}
