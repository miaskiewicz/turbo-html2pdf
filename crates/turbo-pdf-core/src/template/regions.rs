//! Running header/footer region extraction (§3.0, §6.5–6.8), performed as a
//! compile-time source transform — the same family as `switch::desugar`.
//!
//! A `<t:running-header>…</t:running-header>` (or `…-footer…`) placed anywhere in
//! the flow is *not* body content: its inner markup is pulled out of the template
//! source so it never renders in body flow (where the per-page `page.*` context
//! is undefined), registered as its own MiniJinja template (`__header__` /
//! `__footer__`), and re-rendered per page against a `{ page, data }` context.
//!
//! Inside an extracted region, the convenience elements `<t:page/>` and
//! `<t:pages/>` desugar to `{{ page.number }}` / `{{ page.total }}` so the
//! ergonomic "Page X of N" footer (AC-3.0.3) needs no expression syntax.
//!
//! TODO(phase7b): page masters (`<t:page-master>`/`<t:variant>`/`<t:use-master>`),
//! the general `<t:counter>`, leaders, and mirrored-margin duplex are out of this
//! slice; only the master-less running header/footer path is handled here.

/// Internal template name for the extracted running header.
pub const HEADER: &str = "__header__";
/// Internal template name for the extracted running footer.
pub const FOOTER: &str = "__footer__";

/// The result of pulling the running regions out of a template source: the body
/// with the region elements removed, plus each region's inner markup (already
/// `<t:page/>`/`<t:pages/>`-desugared) when present.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Extracted {
    pub body: String,
    pub header: Option<String>,
    pub footer: Option<String>,
}

/// Pull the first running-header and running-footer out of `src`. The remaining
/// body keeps every other byte unchanged (regions without these elements are
/// returned structurally unchanged, mirroring `switch::desugar`'s no-op path).
pub fn extract(src: &str) -> Extracted {
    let (body, header) = take_region(src, "running-header");
    let (body, footer) = take_region(&body, "running-footer");
    Extracted {
        body,
        header: header.map(|inner| desugar_codes(&inner)),
        footer: footer.map(|inner| desugar_codes(&inner)),
    }
}

/// Remove the first `<t:NAME …>…</t:NAME>` element from `src`, returning the body
/// with it cut out and its inner markup. A region missing its close tag is left
/// in place (treated as ordinary body so the markup parser reports it), keeping
/// this transform total.
fn take_region(src: &str, name: &str) -> (String, Option<String>) {
    let open_tag = format!("<t:{name}");
    let close_tag = format!("</t:{name}>");
    let Some(found) = locate(src, &open_tag, &close_tag) else {
        return (src.to_string(), None);
    };
    let mut body = String::with_capacity(src.len());
    body.push_str(&src[..found.open_start]);
    body.push_str(&src[found.close_end..]);
    let inner = src[found.inner_start..found.inner_end].to_string();
    (body, Some(inner))
}

/// The byte boundaries of a located region element.
struct Region {
    open_start: usize,
    inner_start: usize,
    inner_end: usize,
    close_end: usize,
}

/// Find the open tag, the `>` ending it, and its matching close tag.
fn locate(src: &str, open_tag: &str, close_tag: &str) -> Option<Region> {
    let open_start = src.find(open_tag)?;
    let after_open = open_start + open_tag.len();
    let gt = src[after_open..].find('>')? + after_open;
    let inner_start = gt + 1;
    let close_rel = src[inner_start..].find(close_tag)?;
    let inner_end = inner_start + close_rel;
    Some(Region {
        open_start,
        inner_start,
        inner_end,
        close_end: inner_end + close_tag.len(),
    })
}

/// Rewrite the page-number field codes inside an extracted region: `<t:page/>` →
/// `{{ page.number }}` and `<t:pages/>` → `{{ page.total }}`. Both the
/// self-closing and explicit-pair spellings are accepted.
///
/// `pages` is rewritten *before* `page`: the `page` needle is a byte prefix of
/// `pages`, so replacing `page` first would mis-rewrite `<t:pages/>` into the
/// number code. Handling the longer code first sidesteps that without a tokenizer.
fn desugar_codes(inner: &str) -> String {
    let with_pages = replace_code(inner, "pages", "{{ page.total }}");
    replace_code(&with_pages, "page", "{{ page.number }}")
}

/// Replace every `<t:NAME/>` and `<t:NAME></t:NAME>` in `src` with `repl`.
fn replace_code(src: &str, name: &str, repl: &str) -> String {
    let pair = format!("<t:{name}></t:{name}>");
    let with_pairs = src.replace(&pair, repl);
    replace_self_closing(&with_pairs, name, repl)
}

/// Replace `<t:NAME/>` and `<t:NAME …/>` (self-closing, attributes ignored in
/// this slice) with `repl`.
fn replace_self_closing(src: &str, name: &str, repl: &str) -> String {
    let needle = format!("<t:{name}");
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(idx) = rest.find(&needle) {
        match self_closing_end(&rest[idx..], needle.len()) {
            Some(consumed) => {
                out.push_str(&rest[..idx]);
                out.push_str(repl);
                rest = &rest[idx + consumed..];
            }
            None => {
                let keep = idx + needle.len();
                out.push_str(&rest[..keep]);
                rest = &rest[keep..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// If `tag` is a self-closing `<t:NAME …/>` starting at offset `name_len`, return
/// the byte length to consume; otherwise `None` (a non-self-closing match).
fn self_closing_end(tag: &str, name_len: usize) -> Option<usize> {
    let gt = tag.find('>')?;
    if gt >= name_len && tag[..gt].ends_with('/') {
        Some(gt + 1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_region_is_a_noop() {
        let out = extract("<p>hi</p>");
        assert_eq!(out.body, "<p>hi</p>");
        assert!(out.header.is_none() && out.footer.is_none());
    }

    #[test]
    fn extracts_both_regions_and_desugars_codes() {
        let out = extract(
            "<t:running-header>H</t:running-header><p>x</p>\
             <t:running-footer>Page <t:page/> of <t:pages/></t:running-footer>",
        );
        assert_eq!(out.body, "<p>x</p>");
        assert_eq!(out.header.as_deref(), Some("H"));
        assert_eq!(
            out.footer.as_deref(),
            Some("Page {{ page.number }} of {{ page.total }}")
        );
    }

    #[test]
    fn explicit_pair_field_codes_desugar() {
        let out =
            extract("<t:running-footer><t:page></t:page>/<t:pages></t:pages></t:running-footer>");
        assert_eq!(
            out.footer.as_deref(),
            Some("{{ page.number }}/{{ page.total }}")
        );
    }

    #[test]
    fn region_without_close_tag_is_left_in_body() {
        // No matching close: the transform is total and leaves it for the parser.
        let out = extract("<t:running-footer>unterminated");
        assert!(out.footer.is_none());
        assert_eq!(out.body, "<t:running-footer>unterminated");
    }

    #[test]
    fn non_self_closing_field_code_is_left_alone() {
        // A bare `<t:page>` (no slash) is not a field code; the self-closing pass
        // leaves it untouched (exercises the non-self-closing branch).
        let out = extract("<t:running-footer><t:page>x</t:running-footer>");
        assert_eq!(out.footer.as_deref(), Some("<t:page>x"));
    }

    #[test]
    fn self_closing_end_classifies_tags() {
        assert_eq!(self_closing_end("<t:page/>", "<t:page".len()), Some(9));
        assert_eq!(self_closing_end("<t:page>", "<t:page".len()), None);
        assert_eq!(self_closing_end("<t:page", "<t:page".len()), None);
    }
}
