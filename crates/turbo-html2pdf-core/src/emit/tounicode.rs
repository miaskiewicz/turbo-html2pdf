//! `/ToUnicode` CMap construction (ISO 32000-1 §9.10.3; required by tagged PDFs
//! per ISO 14289-1 §7.21.7). Extracts the text behind each shown glyph by
//! mapping the 2-byte subset glyph code (what the content stream shows under
//! `Identity-H`) to its Unicode scalar, so viewer search, copy-paste,
//! text-extraction pipelines, and assistive tech can all recover the source
//! text.
//!
//! Compiled under the `to-unicode` feature (standalone searchable text) or
//! the `pdf-ua` feature (a tagged PDF is required to carry the CMap).

use std::collections::BTreeMap;

use rustybuzz::ttf_parser::Face;

/// Reverse a font's Unicode `cmap` into a glyph-id → codepoint map by walking
/// every Unicode subtable. Keeps the lowest codepoint per glyph (insert-if-absent
/// in the ascending `codepoints` walk), so the `/ToUnicode` mapping a tagged PDF
/// needs is deterministic. A font with no `cmap` yields an empty map.
pub(crate) fn reverse_cmap(face: &Face) -> BTreeMap<u16, u32> {
    let mut out: BTreeMap<u16, u32> = BTreeMap::new();
    let Some(cmap) = face.tables().cmap else {
        return out;
    };
    for sub in cmap.subtables {
        if sub.is_unicode() {
            sub.codepoints(|cp| record_codepoint(&sub, cp, &mut out));
        }
    }
    out
}

/// Record `cp → glyph` reversed into `out`, keeping the first (lowest) codepoint
/// seen for each glyph so the `/ToUnicode` mapping is deterministic.
fn record_codepoint(
    sub: &rustybuzz::ttf_parser::cmap::Subtable,
    cp: u32,
    out: &mut BTreeMap<u16, u32>,
) {
    if let Some(gid) = sub.glyph_index(cp) {
        out.entry(gid.0).or_insert(cp);
    }
}

/// Build a `/ToUnicode` CMap stream body from `(code, codepoint)` pairs, where
/// `code` is the 2-byte subset glyph id shown in the content stream. The pairs
/// are sorted and chunked into `bfchar` blocks (the spec caps a block at 100
/// entries). An empty mapping still yields a structurally valid CMap.
pub(super) fn build(pairs: &[(u16, u32)]) -> String {
    let mut sorted: Vec<(u16, u32)> = pairs.to_vec();
    sorted.sort_unstable();
    sorted.dedup_by_key(|(code, _)| *code);
    let mut out = String::with_capacity(512 + sorted.len() * 16);
    out.push_str(HEADER);
    write_bfchars(&mut out, &sorted);
    out.push_str(FOOTER);
    out
}

/// Emit the `bfchar` blocks, at most 100 entries each (PDF spec limit).
fn write_bfchars(out: &mut String, pairs: &[(u16, u32)]) {
    for chunk in pairs.chunks(100) {
        out.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for &(code, cp) in chunk {
            write_entry(out, code, cp);
        }
        out.push_str("endbfchar\n");
    }
}

/// One `<src> <dst>` line: the 2-byte code mapped to its UTF-16BE destination.
fn write_entry(out: &mut String, code: u16, cp: u32) {
    out.push('<');
    out.push_str(&format!("{code:04X}"));
    out.push_str("> <");
    for unit in char_units(cp) {
        out.push_str(&format!("{unit:04X}"));
    }
    out.push_str(">\n");
}

/// The UTF-16BE code units for a Unicode scalar (a surrogate pair above the BMP),
/// falling back to U+FFFD for an invalid scalar so the CMap is always well-formed.
fn char_units(cp: u32) -> Vec<u16> {
    match char::from_u32(cp) {
        Some(c) => {
            let mut buf = [0u16; 2];
            c.encode_utf16(&mut buf).to_vec()
        }
        None => vec![0xFFFD],
    }
}

const HEADER: &str = "/CIDInit /ProcSet findresource begin\n\
12 dict begin\n\
begincmap\n\
/CIDSystemInfo <</Registry (Adobe) /Ordering (UCS) /Supplement 0>> def\n\
/CMapName /Adobe-Identity-UCS def\n\
/CMapType 2 def\n\
1 begincodespacerange\n\
<0000> <FFFF>\n\
endcodespacerange\n";

const FOOTER: &str = "endcmap\n\
CMapName currentdict /CMap defineresource pop\n\
end\n\
end\n";
