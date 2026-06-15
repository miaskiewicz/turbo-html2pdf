//! `/ToUnicode` CMap construction (`pdf-ua`, ISO 14289-1 §7.21.7). A tagged PDF
//! must let assistive tech recover the text behind each shown glyph; this builds
//! a CMap mapping each 2-byte subset glyph code (what the content stream shows
//! under `Identity-H`) to its Unicode scalar.
//!
//! Only compiled under the `pdf-ua` feature.

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
