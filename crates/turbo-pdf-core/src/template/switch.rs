//! The `{% switch %}` extension (§2.6), implemented as a source-level desugaring
//! to `{% if/elif/else %}` performed before MiniJinja parses the template. The
//! subject is bound once with `{% set %}` (AC-2.9b), comma-separated case values
//! become `==` membership (AC-2.9c), the first match wins (AC-2.9d), and
//! `{% default %}` must be last and unique (AC-2.9e). Whitespace-control markers
//! (`{%- … -%}`) on the switch tags trim adjacent text (AC-2.9i).

use crate::error::{CompileError, ErrorCode, Span};

/// Rewrite every `{% switch %}` block in `src` into equivalent `{% if %}` chains.
/// Templates without `switch` are returned structurally unchanged (AC-2.9j).
pub fn desugar(src: &str) -> Result<String, CompileError> {
    let mut state = Desugarer {
        counter: 0,
        errors: Vec::new(),
    };
    let (out, boundary) = state.read_body(src, 0, false);
    if let Some(e) = state.errors.into_iter().next() {
        return Err(e);
    }
    match boundary {
        Boundary::Eof(_) => Ok(out),
        Boundary::Case { .. } => Err(err("{% case %} outside of {% switch %}")),
        Boundary::Default { .. } => Err(err("{% default %} outside of {% switch %}")),
        Boundary::EndSwitch { .. } => Err(err("{% endswitch %} without {% switch %}")),
    }
}

fn err(message: &str) -> CompileError {
    CompileError {
        code: ErrorCode::TemplateSyntax,
        message: message.into(),
        span: Span::default(),
    }
}

struct Desugarer {
    counter: usize,
    errors: Vec<CompileError>,
}

/// A `{% %}` statement tag located in the source.
struct Tag<'a> {
    start: usize,
    inner: &'a str,
    after: usize,
}

fn find_tag(src: &str, from: usize) -> Option<Tag<'_>> {
    let start = src[from..].find("{%").map(|i| i + from)?;
    let rel = src[start + 2..].find("%}")?;
    let end = start + 2 + rel;
    Some(Tag {
        start,
        inner: &src[start + 2..end],
        after: end + 2,
    })
}

fn keyword(inner: &str) -> &str {
    inner
        .trim_start_matches(|c: char| c.is_whitespace() || c == '-')
        .split_whitespace()
        .next()
        .unwrap_or("")
}

fn args_after(inner: &str, kw: &str) -> String {
    inner
        .trim()
        .trim_matches('-')
        .trim()
        .strip_prefix(kw)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// The tag that terminates a body segment inside switch processing.
enum Boundary {
    Case {
        args: String,
        after: usize,
        lstrip: bool,
        rstrip: bool,
    },
    Default {
        after: usize,
        lstrip: bool,
        rstrip: bool,
    },
    EndSwitch {
        after: usize,
        lstrip: bool,
        rstrip: bool,
    },
    Eof(usize),
}

fn boundary_for(kw: &str, tag: &Tag) -> Option<Boundary> {
    let lstrip = tag.inner.starts_with('-');
    let rstrip = tag.inner.ends_with('-');
    match kw {
        "case" => {
            let args = args_after(tag.inner, "case");
            Some(Boundary::Case {
                args,
                after: tag.after,
                lstrip,
                rstrip,
            })
        }
        "default" => Some(Boundary::Default {
            after: tag.after,
            lstrip,
            rstrip,
        }),
        "endswitch" => Some(Boundary::EndSwitch {
            after: tag.after,
            lstrip,
            rstrip,
        }),
        _ => None,
    }
}

fn push_raw(out: &mut String, slice: &str, trim_next: &mut bool) {
    if *trim_next {
        out.push_str(slice.trim_start());
        *trim_next = false;
    } else {
        out.push_str(slice);
    }
}

impl Desugarer {
    /// Copy/transform source from `start` until a switch boundary tag (or EOF),
    /// recursing into nested `{% switch %}`. `trim_open` left-trims the result
    /// when the preceding tag carried a `-%}` marker.
    fn read_body(&mut self, src: &str, start: usize, trim_open: bool) -> (String, Boundary) {
        let mut out = String::new();
        let mut cur = start;
        let mut trim_next = false;
        loop {
            let Some(tag) = find_tag(src, cur) else {
                push_raw(&mut out, &src[cur..], &mut trim_next);
                return (finish_body(out, trim_open), Boundary::Eof(src.len()));
            };
            push_raw(&mut out, &src[cur..tag.start], &mut trim_next);
            let kw = keyword(tag.inner);
            if let Some(next) = self.dispatch(src, &tag, kw, &mut out, &mut trim_next) {
                cur = next;
            } else if let Some(boundary) = boundary_for(kw, &tag) {
                return (finish_body(out, trim_open), boundary);
            } else {
                out.push_str(&src[tag.start..tag.after]);
                cur = tag.after;
            }
        }
    }

    /// Handle a `{% switch %}` opener inline; returns the cursor past its
    /// `{% endswitch %}`, or `None` for any other tag.
    fn dispatch(
        &mut self,
        src: &str,
        tag: &Tag,
        kw: &str,
        out: &mut String,
        trim_next: &mut bool,
    ) -> Option<usize> {
        if kw != "switch" {
            return None;
        }
        if tag.inner.starts_with('-') {
            truncate_trailing_ws(out);
        }
        let subject = args_after(tag.inner, "switch");
        let var = self.fresh_var();
        let (branches, after, end_rstrip) = self.collect_branches(src, tag.after);
        out.push_str(&render_switch(&var, &subject, &branches));
        *trim_next = end_rstrip;
        Some(after)
    }

    fn fresh_var(&mut self) -> String {
        let v = format!("__tpdf_switch_{}", self.counter);
        self.counter += 1;
        v
    }

    fn collect_branches(&mut self, src: &str, body_start: usize) -> (Vec<Branch>, usize, bool) {
        let (pre, first) = self.read_body(src, body_start, false);
        if !pre.trim().is_empty() {
            self.errors
                .push(err("text before first {% case %} in {% switch %}"));
        }
        let mut branches = Vec::new();
        let (after, rstrip) = self.gather(src, first, &mut branches);
        (branches, after, rstrip)
    }

    fn gather(&mut self, src: &str, first: Boundary, branches: &mut Vec<Branch>) -> (usize, bool) {
        let mut boundary = first;
        let mut seen_default = false;
        loop {
            match self.step(src, boundary, &mut seen_default, branches) {
                Step::Done { after, rstrip } => return (after, rstrip),
                Step::Next(next) => boundary = next,
            }
        }
    }

    fn step(
        &mut self,
        src: &str,
        boundary: Boundary,
        seen_default: &mut bool,
        branches: &mut Vec<Branch>,
    ) -> Step {
        match boundary {
            Boundary::EndSwitch { after, rstrip, .. } => Step::Done { after, rstrip },
            Boundary::Eof(after) => self.unterminated(after),
            Boundary::Case {
                args,
                after,
                rstrip,
                ..
            } => self.push_case(src, args, after, rstrip, *seen_default, branches),
            Boundary::Default { after, rstrip, .. } => {
                *seen_default = true;
                self.push_default(src, after, rstrip, branches)
            }
        }
    }

    fn unterminated(&mut self, after: usize) -> Step {
        self.errors.push(err("unterminated {% switch %}"));
        Step::Done {
            after,
            rstrip: false,
        }
    }

    fn push_case(
        &mut self,
        src: &str,
        args: String,
        after: usize,
        rstrip: bool,
        seen_default: bool,
        branches: &mut Vec<Branch>,
    ) -> Step {
        let (body, next) = self.read_body(src, after, rstrip);
        if seen_default {
            self.errors.push(err("{% case %} after {% default %}"));
        }
        branches.push(Branch::Case {
            args,
            body: apply_close_trim(body, &next),
        });
        Step::Next(next)
    }

    fn push_default(
        &mut self,
        src: &str,
        after: usize,
        rstrip: bool,
        branches: &mut Vec<Branch>,
    ) -> Step {
        let (body, next) = self.read_body(src, after, rstrip);
        if has_prior_default(branches) {
            self.errors
                .push(err("multiple {% default %} in {% switch %}"));
        }
        branches.push(Branch::Default {
            body: apply_close_trim(body, &next),
        });
        Step::Next(next)
    }
}

fn truncate_trailing_ws(out: &mut String) {
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
}

fn has_prior_default(branches: &[Branch]) -> bool {
    branches.iter().any(|b| matches!(b, Branch::Default { .. }))
}

enum Step {
    Next(Boundary),
    Done { after: usize, rstrip: bool },
}

enum Branch {
    Case { args: String, body: String },
    Default { body: String },
}

fn finish_body(body: String, trim_open: bool) -> String {
    if trim_open {
        body.trim_start().to_string()
    } else {
        body
    }
}

fn close_lstrip(next: &Boundary) -> bool {
    match next {
        Boundary::Case { lstrip, .. }
        | Boundary::Default { lstrip, .. }
        | Boundary::EndSwitch { lstrip, .. } => *lstrip,
        Boundary::Eof(_) => false,
    }
}

fn apply_close_trim(body: String, next: &Boundary) -> String {
    if close_lstrip(next) {
        body.trim_end().to_string()
    } else {
        body
    }
}

// --------------------------------------------------------------------------
// rendering the desugared if/elif/else chain
// --------------------------------------------------------------------------

fn scan_comma(
    args: &str,
    ch: char,
    i: usize,
    depth: &mut i32,
    last: &mut usize,
    parts: &mut Vec<String>,
) {
    match ch {
        '(' | '[' => *depth += 1,
        ')' | ']' => *depth -= 1,
        ',' if *depth == 0 => {
            push_part(args, *last, i, parts);
            *last = i + 1;
        }
        _ => {}
    }
}

fn split_top_commas(args: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut last = 0usize;
    for (i, ch) in args.char_indices() {
        scan_comma(args, ch, i, &mut depth, &mut last, &mut parts);
    }
    push_part(args, last, args.len(), &mut parts);
    parts
}

fn push_part(args: &str, from: usize, to: usize, parts: &mut Vec<String>) {
    let piece = args[from..to].trim();
    if !piece.is_empty() {
        parts.push(piece.to_string());
    }
}

fn build_cond(var: &str, args: &str) -> String {
    let terms: Vec<String> = split_top_commas(args)
        .iter()
        .map(|v| format!("{var} == {v}"))
        .collect();
    terms.join(" or ")
}

fn append_branch(var: &str, branch: &Branch, first: bool, out: &mut String) {
    match branch {
        Branch::Case { args, body } => {
            let cond = build_cond(var, args);
            let kw = if first { "if" } else { "elif" };
            out.push_str(&format!("{{% {kw} {cond} %}}{body}"));
        }
        Branch::Default { body } => append_default(body, first, out),
    }
}

fn append_default(body: &str, first: bool, out: &mut String) {
    if first {
        out.push_str(&format!("{{% if true %}}{body}"));
    } else {
        out.push_str(&format!("{{% else %}}{body}"));
    }
}

fn render_switch(var: &str, subject: &str, branches: &[Branch]) -> String {
    let mut out = format!("{{% set {var} = {subject} %}}");
    let mut first = true;
    for branch in branches {
        append_branch(var, branch, first, &mut out);
        first = false;
    }
    if !first {
        out.push_str("{% endif %}");
    }
    out
}
