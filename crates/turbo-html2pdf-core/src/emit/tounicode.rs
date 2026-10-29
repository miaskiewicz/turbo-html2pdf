//! `/ToUnicode` CMap construction for searchable, extractable PDF text. This builds
//! a CMap mapping each 2-byte subset glyph code (what the content stream shows
//! under `Identity-H`) to its source text (several characters for a ligature).
//!
//! Emitted for every embedded font, independently of PDF/UA tagging. The
//! glyph -> codepoint reversal itself (`reverse_cmap`) lives in
//! `turbo_dom_layout::cmap`, shared with the text layer.

/// Build a `/ToUnicode` CMap stream body from `(code, text)` pairs, where
/// `code` is the 2-byte subset glyph id shown in the content stream. The pairs
/// are sorted and chunked into `bfchar` blocks (the spec caps a block at 100
/// entries). An empty mapping still yields a structurally valid CMap.
pub(super) fn build(pairs: &[(u16, String)]) -> String {
    let mut sorted: Vec<(u16, String)> = pairs.to_vec();
    sorted.sort_unstable();
    sorted.dedup_by_key(|(code, _)| *code);
    let mut out = String::with_capacity(512 + sorted.len() * 16);
    out.push_str(HEADER);
    write_bfchars(&mut out, &sorted);
    out.push_str(FOOTER);
    out
}

/// Emit the `bfchar` blocks, at most 100 entries each (PDF spec limit).
fn write_bfchars(out: &mut String, pairs: &[(u16, String)]) {
    for chunk in pairs.chunks(100) {
        out.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (code, text) in chunk {
            write_entry(out, *code, text);
        }
        out.push_str("endbfchar\n");
    }
}

/// One `<src> <dst>` line: the 2-byte code mapped to its text as UTF-16BE (a
/// surrogate pair above the BMP).
fn write_entry(out: &mut String, code: u16, text: &str) {
    out.push('<');
    out.push_str(&format!("{code:04X}"));
    out.push_str("> <");
    for unit in text.encode_utf16() {
        out.push_str(&format!("{unit:04X}"));
    }
    out.push_str(">\n");
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

#[cfg(test)]
mod tests {
    use super::*;
    use rustybuzz::ttf_parser::Face;
    use turbo_dom_layout::cmap::reverse_cmap;

    #[test]
    fn reverse_mapping_uses_original_font_and_handles_stripped_cmap() {
        let bytes = &turbo_dom_layout::test_support::font_bytes("Go-Regular.ttf")[..];
        let face = Face::parse(bytes, 0).unwrap();
        let gid = face.glyph_index('é').unwrap().0;
        assert_eq!(reverse_cmap(&face).get(&gid), Some(&0xE9));
        let remapper = subsetter::GlyphRemapper::new_from_glyphs(&[gid]);
        let subset = subsetter::subset(bytes, 0, &remapper).unwrap();
        let subset_face = Face::parse(&subset, 0).unwrap();
        assert!(reverse_cmap(&subset_face).is_empty());
    }

    #[test]
    fn unicode_destinations_use_utf16_and_codes_are_sorted_and_unique() {
        let pairs: Vec<(u16, String)> = [(3, "😀"), (2, "é"), (1, "A"), (2, "é"), (4, "->")]
            .iter()
            .map(|&(code, text)| (code, text.to_string()))
            .collect();
        let cmap = build(&pairs);
        assert!(cmap.contains("4 beginbfchar\n<0001> <0041>\n<0002> <00E9>\n<0003> <D83DDE00>\n<0004> <002D003E>\nendbfchar"));
    }

    #[test]
    fn cmap_blocks_are_limited_to_one_hundred_entries() {
        let pairs: Vec<_> = (0..201).map(|code| (code, "a".to_string())).collect();
        let cmap = build(&pairs);
        assert_eq!(cmap.matches("100 beginbfchar").count(), 2);
        assert_eq!(cmap.matches("1 beginbfchar").count(), 1);
        assert_eq!(cmap.matches("endbfchar").count(), 3);
        assert!(build(&[]).contains("endcodespacerange\nendcmap"));
    }
}
