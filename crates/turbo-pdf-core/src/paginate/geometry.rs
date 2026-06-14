//! Page geometry resolution (§6.1). Turns the `@page` at-rule (or a built-in
//! default) into a concrete [`PageGeometry`] the fragmenter measures against.
//!
//! **Internal unit: CSS pixels at 96dpi**, matching layout (`value.rs`). Named
//! sizes are stored in their physical dimensions and converted on resolution.
//! Source order (§6.1): an `@page` at-rule's body → built-in default (A4, 20mm).
//! A `defaultPage` render option slots in between once render options land
//! (Phase 7); the resolver already takes the default as a parameter.

use crate::error::{ErrorCode, RenderError, Span};
use crate::layout::value::{parse_px, Edges};
use crate::style::AtRule;

/// Millimetres-to-pixel factor at 96dpi (`96 / 25.4`).
const MM: f32 = 96.0 / 25.4;
/// Inches-to-pixel factor at 96dpi.
const IN: f32 = 96.0;
/// The built-in default page margin (`20mm`) applied when `@page` sets none.
const DEFAULT_MARGIN: f32 = 20.0 * MM;

/// A concrete page box: physical size plus the non-body bands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageGeometry {
    pub width: f32,
    pub height: f32,
    pub margin: Edges,
    /// Height reserved at the top for a running header (0 until Phase 7).
    pub header_extent: f32,
    /// Height reserved at the bottom for a running footer (0 until Phase 7).
    pub footer_extent: f32,
}

impl PageGeometry {
    /// The built-in default: A4 portrait with uniform 20mm margins.
    pub fn a4() -> PageGeometry {
        let (width, height) = named_size("a4").expect("a4 is a known size");
        PageGeometry {
            width,
            height,
            margin: Edges::all(DEFAULT_MARGIN),
            header_extent: 0.0,
            footer_extent: 0.0,
        }
    }

    /// Usable body width: page width minus the left/right margins.
    pub fn content_width(&self) -> f32 {
        self.width - self.margin.horizontal()
    }

    /// Usable body height: page height minus margins and the header/footer bands.
    pub fn body_height(&self) -> f32 {
        self.height - self.margin.vertical() - self.header_extent - self.footer_extent
    }

    /// Top-left corner of the body area in page coordinates.
    pub fn body_origin(&self) -> (f32, f32) {
        (self.margin.left, self.margin.top + self.header_extent)
    }
}

/// Physical size in px for a named page size, or `None` if unknown.
fn named_size(name: &str) -> Option<(f32, f32)> {
    let (w, h) = match name {
        "a3" => (297.0 * MM, 420.0 * MM),
        "a4" => (210.0 * MM, 297.0 * MM),
        "a5" => (148.0 * MM, 210.0 * MM),
        "letter" => (8.5 * IN, 11.0 * IN),
        "legal" => (8.5 * IN, 14.0 * IN),
        _ => return None,
    };
    Some((w, h))
}

/// Swap width/height for landscape orientation.
fn orient(size: (f32, f32), landscape: bool) -> (f32, f32) {
    if landscape {
        (size.1, size.0)
    } else {
        size
    }
}

fn unknown_size(token: &str) -> RenderError {
    RenderError {
        code: ErrorCode::Render,
        message: format!(
            "unknown @page size `{token}` (valid: A3, A4, A5, Letter, Legal, or `W H` lengths)"
        ),
        span: Span::default(),
    }
}

/// Resolve a `size:` value: a named size, a single `W` (square), or `W H`, each
/// with an optional `portrait`/`landscape` keyword.
fn parse_size(value: &str) -> Result<(f32, f32), RenderError> {
    let mut landscape = false;
    let mut named: Option<(f32, f32)> = None;
    let mut dims: Vec<f32> = Vec::new();
    for token in value.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        match keyword_or_dim(&lower) {
            SizeToken::Landscape => landscape = true,
            SizeToken::Portrait => landscape = false,
            SizeToken::Named(size) => named = Some(size),
            SizeToken::Length(px) => dims.push(px),
            SizeToken::Unknown => return Err(unknown_size(token)),
        }
    }
    let base = named.unwrap_or_else(|| dims_to_size(&dims));
    Ok(orient(base, landscape))
}

enum SizeToken {
    Landscape,
    Portrait,
    Named((f32, f32)),
    Length(f32),
    Unknown,
}

/// Classify one whitespace-separated `size:` token (already lowercased).
fn keyword_or_dim(token: &str) -> SizeToken {
    match token {
        "landscape" => SizeToken::Landscape,
        "portrait" => SizeToken::Portrait,
        _ => named_or_length(token),
    }
}

fn named_or_length(token: &str) -> SizeToken {
    if let Some(size) = named_size(token) {
        return SizeToken::Named(size);
    }
    match parse_px(token, 16.0) {
        Some(px) => SizeToken::Length(px),
        None => SizeToken::Unknown,
    }
}

/// Turn collected explicit dimensions into a `(w, h)`; 1 value is a square, 2+
/// uses the first two, 0 falls back to A4.
fn dims_to_size(dims: &[f32]) -> (f32, f32) {
    match dims {
        [w] => (*w, *w),
        [w, h, ..] => (*w, *h),
        _ => named_size("a4").expect("a4 is a known size"),
    }
}

/// Expand a `margin:` shorthand (1, 2, or 4 lengths) into [`Edges`].
fn parse_margin(value: &str) -> Option<Edges> {
    let parts: Vec<f32> = value
        .split_whitespace()
        .map(|t| parse_px(t, 16.0))
        .collect::<Option<Vec<f32>>>()?;
    edges_from(&parts)
}

fn edges_from(parts: &[f32]) -> Option<Edges> {
    match parts {
        [a] => Some(Edges::all(*a)),
        [v, h] => Some(Edges {
            top: *v,
            right: *h,
            bottom: *v,
            left: *h,
        }),
        [t, r, b, l] => Some(Edges {
            top: *t,
            right: *r,
            bottom: *b,
            left: *l,
        }),
        _ => None,
    }
}

/// Split an at-rule body into `(property, value)` declaration pairs.
fn declarations(body: &str) -> impl Iterator<Item = (String, &str)> {
    body.split(';').filter_map(|decl| {
        let (prop, value) = decl.split_once(':')?;
        Some((prop.trim().to_ascii_lowercase(), value.trim()))
    })
}

/// Apply one `@page` declaration to the geometry under construction.
fn apply_decl(geo: &mut PageGeometry, prop: &str, value: &str) -> Result<(), RenderError> {
    match prop {
        "size" => {
            let (w, h) = parse_size(value)?;
            geo.width = w;
            geo.height = h;
        }
        "margin" => {
            if let Some(edges) = parse_margin(value) {
                geo.margin = edges;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The first `@page` at-rule (ignoring named pseudo-pages until Phase 7).
fn find_page(at_rules: &[AtRule]) -> Option<&AtRule> {
    at_rules
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case("page"))
}

/// Resolve the effective page geometry from the stylesheet's at-rules, falling
/// back to `default`. Unknown named sizes are a fatal render error (§6.1).
pub fn resolve_geometry(
    at_rules: &[AtRule],
    default: PageGeometry,
) -> Result<PageGeometry, RenderError> {
    let mut geo = default;
    let Some(page) = find_page(at_rules) else {
        return Ok(geo);
    };
    for (prop, value) in declarations(&page.body) {
        apply_decl(&mut geo, &prop, value)?;
    }
    Ok(geo)
}
