//! Typed CSS value resolution (§5, §4.1). The cascade ([`ComputedStyle`]) keeps
//! values as raw strings; layout needs typed numbers. This module parses lengths,
//! colors, and keywords into the [`BoxStyle`] a box uses for layout.
//!
//! **Internal unit: CSS pixels at 96dpi.** Absolute units convert on parse
//! (`1pt = 1/72in`, `1in = 96px`); `em` resolves against the parent font size and
//! `%` against the containing-block width at resolution time.

use crate::style::ComputedStyle;

/// Pixels per inch for absolute-unit conversion (the CSS reference pixel).
const DPI: f32 = 96.0;
/// The initial font size when none is inherited or set (`16px`).
pub const DEFAULT_FONT_SIZE: f32 = 16.0;

/// Four edge values (margin, padding, border widths), in px.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    /// A uniform edge set.
    pub fn all(v: f32) -> Edges {
        Edges {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    /// Total horizontal extent (left + right).
    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    /// Total vertical extent (top + bottom).
    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

/// A `<length-percentage>` or `auto`, resolved against context as needed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthPct {
    Auto,
    Px(f32),
    Pct(f32),
}

impl LengthPct {
    /// Resolve to px against a percentage basis, or `None` for `auto`.
    pub fn resolve(&self, basis: f32) -> Option<f32> {
        match self {
            LengthPct::Auto => None,
            LengthPct::Px(v) => Some(*v),
            LengthPct::Pct(p) => Some(p / 100.0 * basis),
        }
    }
}

/// CSS `display` (the v1 subset, §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    InlineBlock,
    Flex,
    Grid,
    None,
    Table,
    TableRow,
    TableCell,
    TableHeaderGroup,
    TableFooterGroup,
    ListItem,
}

/// CSS `box-sizing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

/// CSS `vertical-align` (the inline/table-cell subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VAlign {
    #[default]
    Baseline,
    Sub,
    Super,
    Middle,
    Top,
    Bottom,
}

/// CSS fragmentation rule (`break-before`/`break-after`/`break-inside`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreakRule {
    #[default]
    Auto,
    Avoid,
    Page,
}

/// An RGBA color, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const BLACK: Rgba = Rgba {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };

    pub fn new(r: u8, g: u8, b: u8, a: u8) -> Rgba {
        Rgba { r, g, b, a }
    }
}

/// One border side: width (px) and color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BorderSide {
    pub width: u16,
    pub color: Option<Rgba>,
}

/// The four border sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BorderEdges {
    pub top: BorderSide,
    pub right: BorderSide,
    pub bottom: BorderSide,
    pub left: BorderSide,
}

impl BorderEdges {
    /// Whether any side has a non-zero width, i.e. the border paints something
    /// (used by the `pdf-ua` emitter to decide if a box is decoration). Gated so
    /// it adds nothing to the default build's coverage surface (AC-11.1).
    #[cfg(feature = "pdf-ua")]
    pub fn any_visible(&self) -> bool {
        self.top.width > 0 || self.right.width > 0 || self.bottom.width > 0 || self.left.width > 0
    }

    /// Border widths as plain px [`Edges`].
    pub fn widths(&self) -> Edges {
        Edges {
            top: f32::from(self.top.width),
            right: f32::from(self.right.width),
            bottom: f32::from(self.bottom.width),
            left: f32::from(self.left.width),
        }
    }
}

// --------------------------------------------------------------------------
// length parsing
// --------------------------------------------------------------------------

/// A parsed length before `em`/`%` resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RawLength {
    Abs(f32),
    Em(f32),
    /// `rem` — relative to the ROOT font size, not the parent's (unlike `em`).
    Rem(f32),
    Pct(f32),
}

fn unit_factor(unit: &str) -> Option<f32> {
    match unit {
        "px" | "" => Some(1.0),
        "pt" => Some(DPI / 72.0),
        "pc" => Some(DPI / 6.0),
        "in" => Some(DPI),
        "cm" => Some(DPI / 2.54),
        "mm" => Some(DPI / 25.4),
        _ => None,
    }
}

fn split_unit(s: &str) -> (&str, &str) {
    let end = s
        .char_indices()
        .find(|(_, c)| c.is_ascii_alphabetic() || *c == '%')
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    (&s[..end], &s[end..])
}

fn parse_raw(s: &str) -> Option<RawLength> {
    let t = s.trim();
    let (num, unit) = split_unit(t);
    let value: f32 = num.parse().ok()?;
    match unit {
        "%" => Some(RawLength::Pct(value)),
        "em" => Some(RawLength::Em(value)),
        "rem" => Some(RawLength::Rem(value)),
        // Viewport-percentage units resolve to px against the layout viewport now
        // (they carry no dependency on the containing block) — `100vh` on a hero /
        // full-screen section gives it a real height instead of failing to parse.
        "vw" | "vh" | "vmin" | "vmax" => Some(RawLength::Abs(viewport_len(value, unit))),
        u => unit_factor(u).map(|f| RawLength::Abs(value * f)),
    }
}

/// A viewport-percentage length (`vw`/`vh`/`vmin`/`vmax`) as px against the current
/// layout viewport (`1vh` = 1% of viewport height, `vmin`/`vmax` the smaller/larger
/// axis).
fn viewport_len(value: f32, unit: &str) -> f32 {
    let (vw, vh) = crate::style::viewport_px();
    let basis = match unit {
        "vw" => vw,
        "vh" => vh,
        "vmin" => vw.min(vh),
        _ => vw.max(vh),
    };
    value / 100.0 * basis
}

fn raw_to_px(raw: RawLength, font_size: f32, basis: f32) -> f32 {
    match raw {
        RawLength::Abs(v) => v,
        RawLength::Em(v) => v * font_size,
        // `rem` is root-relative; the root font size is the initial 16px (turbo does
        // not track a document-level `html { font-size }` override).
        RawLength::Rem(v) => v * DEFAULT_FONT_SIZE,
        RawLength::Pct(p) => p / 100.0 * basis,
    }
}

/// Evaluate an additive `calc(A ± B ± …)` of non-`%` length terms to px. CSS
/// mandates spaces around `+`/`-`, so the body tokenizes on whitespace. Real
/// stylesheets lean on this for component sizing — Codex's radio label offset
/// `padding-left: calc(1rem + 10px)`, breakpoints like `calc(1120px - 1px)`, etc.
/// (`var()` is already substituted by the cascade.) Multiplicative or `%` terms
/// aren't handled here and yield `None` (the caller falls back to its default).
fn eval_calc_px(s: &str, font_size: f32) -> Option<f32> {
    let toks: Vec<&str> = strip_calc(s)?.split_whitespace().collect();
    sum_calc_terms(&toks, font_size)
}

/// The inside of a `calc( … )` / `CALC( … )` wrapper, or `None` if `s` isn't one.
fn strip_calc(s: &str) -> Option<&str> {
    let t = s.trim();
    t.strip_prefix("calc(")
        .or_else(|| t.strip_prefix("CALC("))?
        .strip_suffix(')')
}

/// One `calc()` term as px — an absolute/font-relative length (never a `%`).
fn calc_term_px(t: &str, font_size: f32) -> Option<f32> {
    match parse_raw(t)? {
        RawLength::Pct(_) => None,
        raw => Some(raw_to_px(raw, font_size, 0.0)),
    }
}

/// Left-fold `term (± term)*` (only `+`/`-` supported), or `None` on any bad token.
fn sum_calc_terms(toks: &[&str], font_size: f32) -> Option<f32> {
    let (first, rest) = toks.split_first()?;
    let init = calc_term_px(first, font_size)?;
    rest.chunks_exact(2).try_fold(init, |total, pair| {
        let v = calc_term_px(pair[1], font_size)?;
        match pair[0] {
            "+" => Some(total + v),
            "-" => Some(total - v),
            _ => None,
        }
    })
}

/// Parse an absolute/`em` length (or `calc()` thereof) to px (no `%`); used for
/// font-relative values.
pub fn parse_px(s: &str, font_size: f32) -> Option<f32> {
    if let Some(px) = eval_calc_px(s, font_size) {
        return Some(px);
    }
    match parse_raw(s)? {
        RawLength::Pct(_) => None,
        raw => Some(raw_to_px(raw, font_size, 0.0)),
    }
}

/// Parse a `<length-percentage>`/`auto` into a [`LengthPct`].
pub fn parse_length_pct(s: &str, font_size: f32) -> Option<LengthPct> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("auto") {
        return Some(LengthPct::Auto);
    }
    if let Some(px) = eval_calc_px(t, font_size) {
        return Some(LengthPct::Px(px));
    }
    match parse_raw(t)? {
        RawLength::Pct(p) => Some(LengthPct::Pct(p)),
        RawLength::Abs(v) => Some(LengthPct::Px(v)),
        RawLength::Em(v) => Some(LengthPct::Px(v * font_size)),
        RawLength::Rem(v) => Some(LengthPct::Px(v * DEFAULT_FONT_SIZE)),
    }
}

// --------------------------------------------------------------------------
// color parsing
// --------------------------------------------------------------------------

fn named_color(name: &str) -> Option<Rgba> {
    let c = match name {
        "black" => (0, 0, 0, 255),
        "white" => (255, 255, 255, 255),
        "red" => (255, 0, 0, 255),
        "green" => (0, 128, 0, 255),
        "blue" => (0, 0, 255, 255),
        "gray" | "grey" => (128, 128, 128, 255),
        "transparent" => (0, 0, 0, 0),
        _ => return None,
    };
    Some(Rgba::new(c.0, c.1, c.2, c.3))
}

fn hex_pair(s: &str) -> Option<u8> {
    u8::from_str_radix(s, 16).ok()
}

fn parse_hex3(h: &str) -> Option<Rgba> {
    let dup = |c: &str| hex_pair(&format!("{c}{c}"));
    Some(Rgba::new(
        dup(&h[0..1])?,
        dup(&h[1..2])?,
        dup(&h[2..3])?,
        255,
    ))
}

fn parse_hex6(h: &str) -> Option<Rgba> {
    Some(Rgba::new(
        hex_pair(&h[0..2])?,
        hex_pair(&h[2..4])?,
        hex_pair(&h[4..6])?,
        255,
    ))
}

fn parse_hex8(h: &str) -> Option<Rgba> {
    let mut c = parse_hex6(&h[0..6])?;
    c.a = hex_pair(&h[6..8])?;
    Some(c)
}

fn parse_hex(h: &str) -> Option<Rgba> {
    match h.len() {
        3 => parse_hex3(h),
        6 => parse_hex6(h),
        8 => parse_hex8(h),
        _ => None,
    }
}

fn channel(part: &str) -> Option<u8> {
    let v: f32 = part.trim().parse().ok()?;
    Some(v.round().clamp(0.0, 255.0) as u8)
}

fn alpha_channel(part: &str) -> Option<u8> {
    let v: f32 = part.trim().parse().ok()?;
    Some((v * 255.0).round().clamp(0.0, 255.0) as u8)
}

fn nth_channel(parts: &[&str], i: usize) -> Option<u8> {
    channel(parts.get(i)?)
}

fn parse_rgb_parts(parts: &[&str]) -> Option<Rgba> {
    Some(Rgba::new(
        nth_channel(parts, 0)?,
        nth_channel(parts, 1)?,
        nth_channel(parts, 2)?,
        255,
    ))
}

fn parse_rgb_fn(inner: &str) -> Option<Rgba> {
    let parts: Vec<&str> = inner.split([',', '/']).collect();
    let rgb = parse_rgb_parts(&parts)?;
    let a = match parts.get(3) {
        Some(p) => alpha_channel(p)?,
        None => 255,
    };
    Some(Rgba { a, ..rgb })
}

fn paren_inner(s: &str) -> Option<&str> {
    s.trim().strip_prefix('(')?.strip_suffix(')')
}

fn rgb_inner(t: &str) -> Option<&str> {
    let body = t.strip_prefix("rgba").or_else(|| t.strip_prefix("rgb"))?;
    paren_inner(body)
}

/// Parse a CSS color (`#rgb`/`#rrggbb`/`#rrggbbaa`, `rgb()`/`rgba()`, or a small
/// named set). Returns `None` for an unrecognized value.
pub fn parse_color(s: &str) -> Option<Rgba> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Some(inner) = rgb_inner(t) {
        return parse_rgb_fn(inner);
    }
    #[cfg(feature = "print-color")]
    if let Some(cmyk) = parse_cmyk(t) {
        return Some(cmyk);
    }
    named_color(&t.to_ascii_lowercase())
}

/// Parse a `cmyk(c, m, y, k)` functional colour into the nearest [`Rgba`]
/// (`print-color`, AC-7.x). The four components are percentages or 0..=1
/// fractions. Layout is RGB-internal, so we store the device-equivalent RGB; the
/// emitter (`emit::color::set_fill`) converts it back to DeviceCMYK so the page
/// stream carries CMYK ink. Round-trips exactly for the achromatic and primary
/// inks print stylesheets use.
#[cfg(feature = "print-color")]
fn parse_cmyk(s: &str) -> Option<Rgba> {
    let inner = paren_inner(s.strip_prefix("cmyk")?)?;
    let [c, m, y, k] = cmyk_components(inner)?;
    let to_byte = |ink: f32| ((1.0 - ink) * (1.0 - k) * 255.0).round() as u8;
    Some(Rgba::new(to_byte(c), to_byte(m), to_byte(y), 255))
}

/// Parse the four CMYK components from the `(...)` inner text, or `None` unless
/// exactly four valid components are present.
#[cfg(feature = "print-color")]
fn cmyk_components(inner: &str) -> Option<[f32; 4]> {
    let mut out = [0.0_f32; 4];
    let mut seen = 0;
    for token in inner.split([',', '/']) {
        let slot = out.get_mut(seen)?;
        *slot = cmyk_component(token.trim())?;
        seen += 1;
    }
    (seen == 4).then_some(out)
}

/// One CMYK component: a `NN%` percentage or a bare `0..=1` fraction, clamped.
#[cfg(feature = "print-color")]
fn cmyk_component(token: &str) -> Option<f32> {
    let value = match token.strip_suffix('%') {
        Some(pct) => pct.trim().parse::<f32>().ok()? / 100.0,
        None => token.parse::<f32>().ok()?,
    };
    Some(value.clamp(0.0, 1.0))
}

// --------------------------------------------------------------------------
// resolved box style
// --------------------------------------------------------------------------

/// CSS `position` scheme. `Static` is normal flow; the rest establish the box's
/// relationship to a containing block (and, for non-`Static`, a stacking
/// context when paired with a `z-index`). `Sticky` is treated as `Relative` for
/// layout (no scroll container in a paged/snapshot render).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
    #[default]
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

impl Position {
    /// Whether the box is taken out of normal flow (`absolute`/`fixed`).
    pub fn is_out_of_flow(self) -> bool {
        matches!(self, Position::Absolute | Position::Fixed)
    }
}

/// CSS `float`. A floated box is pulled out of block flow and packed to the left
/// or right edge; following in-flow content clears below the float row (a
/// pragmatic model — no per-line text wrap around a float).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Float {
    #[default]
    None,
    Left,
    Right,
}

/// The fully typed style a box uses for layout, resolved from a [`ComputedStyle`].
#[derive(Debug, Clone, PartialEq)]
pub struct BoxStyle {
    pub display: Display,
    /// CSS `position` scheme (was `position_relative: bool`).
    pub position: Position,
    /// Inset offsets (`top`/`right`/`bottom`/`left`); `Auto` when unset.
    pub inset_top: LengthPct,
    pub inset_right: LengthPct,
    pub inset_bottom: LengthPct,
    pub inset_left: LengthPct,
    /// `z-index` paint order within the stacking context; `None` = `auto`.
    pub z_index: Option<i32>,
    /// CSS `float` (packs the box to the left/right edge, out of block flow).
    pub float: Float,
    pub margin: Edges,
    pub padding: Edges,
    pub border: BorderEdges,
    pub width: LengthPct,
    pub height: LengthPct,
    pub min_width: LengthPct,
    pub max_width: LengthPct,
    pub min_height: LengthPct,
    pub max_height: LengthPct,
    /// `border-radius` (first value; `%` against the box size at layout).
    pub border_radius: LengthPct,
    pub box_sizing: BoxSizing,
    pub font_families: Vec<String>,
    pub font_size: f32,
    pub font_weight: u16,
    pub italic: bool,
    pub color: Rgba,
    pub line_height: Option<f32>,
    pub text_align: crate::text::Align,
    pub white_space: crate::text::WhiteSpace,
    pub letter_spacing: f32,
    pub vertical_align: VAlign,
    pub break_before: BreakRule,
    pub break_after: BreakRule,
    pub break_inside_avoid: bool,
    pub orphans: u8,
    pub widows: u8,
    pub background: Option<Rgba>,
    /// `box-shadow` first/topmost layer (outer or inset), or `None`.
    pub box_shadow: Option<super::fragment::BoxShadow>,
    /// A `linear-gradient(...)` background image, or `None`.
    pub background_gradient: Option<super::fragment::LinearGradient>,
    /// A CSS 2D `transform` — the linear part `[a,b,c,d]` plus the translate
    /// components (kept as `<length-percentage>` so a `%` translate resolves against
    /// the box's own size at layout). `None` for `none`/unparsable/3D-only.
    pub transform: Option<RawTransform>,
}

/// A parsed CSS 2D transform before its `%` translate is resolved: the linear
/// matrix part `[a, b, c, d]` (`matrix(a,b,c,d,·,·)`) and the two translate offsets
/// (`e`/`f`), which may be `%` of the box size. Layout resolves the translate and
/// combines it into the full `[a,b,c,d,e,f]` matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawTransform {
    pub linear: [f32; 4],
    pub tx: LengthPct,
    pub ty: LengthPct,
}

/// Context for resolving font-relative and percentage values.
#[derive(Debug, Clone, Copy)]
pub struct ResolveCtx {
    pub parent_font_size: f32,
    pub cb_width: f32,
}

fn resolve_font_size(s: &ComputedStyle, parent: f32) -> f32 {
    let raw = match s.get("font-size") {
        Some(v) => v,
        None => return parent,
    };
    match parse_raw(raw) {
        Some(r) => raw_to_px(r, parent, parent),
        None => parent,
    }
}

/// The resolved `display` keyword (needed at box-generation time, before widths
/// are known).
pub fn display_of(s: &ComputedStyle) -> Display {
    match s.get("display").unwrap_or("block").trim() {
        "inline" => Display::Inline,
        "inline-block" => Display::InlineBlock,
        "flex" | "inline-flex" => Display::Flex,
        "grid" | "inline-grid" => Display::Grid,
        "none" => Display::None,
        "table" => Display::Table,
        "table-row" => Display::TableRow,
        "table-cell" => Display::TableCell,
        "table-header-group" => Display::TableHeaderGroup,
        "table-footer-group" => Display::TableFooterGroup,
        "list-item" => Display::ListItem,
        _ => Display::Block,
    }
}

/// The CSS `position` scheme (defaults to `static`).
pub fn position_of(s: &ComputedStyle) -> Position {
    match s.get("position").map(str::trim) {
        Some("relative") => Position::Relative,
        Some("absolute") => Position::Absolute,
        Some("fixed") => Position::Fixed,
        Some("sticky") => Position::Sticky,
        _ => Position::Static,
    }
}

/// The CSS `float` of a box (`left`/`right`, else `none`).
pub fn float_of(s: &ComputedStyle) -> Float {
    match s.get("float").map(str::trim) {
        Some("left") => Float::Left,
        Some("right") => Float::Right,
        _ => Float::None,
    }
}

fn edge_px(token: &str, fs: f32, basis: f32) -> f32 {
    parse_length_pct(token, fs)
        .and_then(|l| l.resolve(basis))
        .unwrap_or(0.0)
}

fn edges_from_slice(v: &[f32]) -> Edges {
    match v {
        [a] => Edges::all(*a),
        [a, b] => Edges {
            top: *a,
            right: *b,
            bottom: *a,
            left: *b,
        },
        [a, b, c] => Edges {
            top: *a,
            right: *b,
            bottom: *c,
            left: *b,
        },
        [a, b, c, d] => Edges {
            top: *a,
            right: *b,
            bottom: *c,
            left: *d,
        },
        _ => Edges::default(),
    }
}

fn parse_edge_shorthand(v: &str, fs: f32, basis: f32) -> Edges {
    let vals: Vec<f32> = v
        .split_whitespace()
        .map(|t| edge_px(t, fs, basis))
        .collect();
    edges_from_slice(&vals)
}

fn side_value(s: &ComputedStyle, prop: &str, fs: f32, basis: f32) -> Option<f32> {
    s.get(prop).map(|v| edge_px(v, fs, basis))
}

fn resolve_edges(s: &ComputedStyle, prefix: &str, fs: f32, basis: f32) -> Edges {
    let mut e = s
        .get(prefix)
        .map(|v| parse_edge_shorthand(v, fs, basis))
        .unwrap_or_default();
    let sides = [
        ("top", &mut e.top),
        ("right", &mut e.right),
        ("bottom", &mut e.bottom),
        ("left", &mut e.left),
    ];
    for (name, slot) in sides {
        if let Some(v) = side_value(s, &format!("{prefix}-{name}"), fs, basis) {
            *slot = v;
        }
    }
    e
}

fn length_prop(s: &ComputedStyle, prop: &str, fs: f32, default: LengthPct) -> LengthPct {
    s.get(prop)
        .and_then(|v| parse_length_pct(v, fs))
        .unwrap_or(default)
}

/// `border-radius`: the first radius (horizontal, first corner). Elliptical `/`
/// and per-corner values collapse to that single radius — enough for the common
/// uniform-radius / pill / circle (`50%`) cases.
fn border_radius_of(s: &ComputedStyle, fs: f32) -> LengthPct {
    s.get("border-radius")
        .and_then(|v| v.split(['/', ' ']).find(|t| !t.trim().is_empty()))
        .and_then(|t| parse_length_pct(t.trim(), fs))
        .unwrap_or(LengthPct::Px(0.0))
}

fn border_width_token(token: &str, fs: f32) -> Option<u16> {
    let w = match token {
        "thin" => 1.0,
        "medium" => 3.0,
        "thick" => 5.0,
        "none" => 0.0,
        other => parse_px(other, fs)?,
    };
    Some(w.round().clamp(0.0, f32::from(u16::MAX)) as u16)
}

fn parse_border_shorthand(v: Option<&str>, fs: f32) -> BorderSide {
    let mut side = BorderSide::default();
    let Some(text) = v else { return side };
    for token in text.split_whitespace() {
        if let Some(w) = border_width_token(token, fs) {
            side.width = w;
        } else if let Some(c) = parse_color(token) {
            side.color = Some(c);
        }
    }
    side
}

fn resolve_border_side(s: &ComputedStyle, name: &str, fs: f32) -> BorderSide {
    // Base: the `border` shorthand, overridden by the per-side `border-<name>`.
    let mut b = parse_border_shorthand(s.get("border"), fs);
    if let Some(v) = s.get(&format!("border-{name}")) {
        b = parse_border_shorthand(Some(v), fs);
    }
    apply_border_longhands(&mut b, s, name, fs);
    b
}

/// Apply the `border-{color,width}` and `border-<name>-{width,color}` longhands over
/// a side already parsed from the shorthands. Without this a
/// `border:1px solid transparent` + `border-color:#72777d` (Codex's radio icon) kept
/// the transparent colour and the circle outline was invisible.
fn apply_border_longhands(b: &mut BorderSide, s: &ComputedStyle, name: &str, fs: f32) {
    if let Some(c) = s.get("border-color").and_then(parse_color) {
        b.color = Some(c);
    }
    if let Some(w) = s
        .get("border-width")
        .and_then(|v| border_width_token(v, fs))
    {
        b.width = w;
    }
    if let Some(w) = s
        .get(&format!("border-{name}-width"))
        .and_then(|v| border_width_token(v, fs))
    {
        b.width = w;
    }
    if let Some(c) = s.get(&format!("border-{name}-color")).and_then(parse_color) {
        b.color = Some(c);
    }
}

fn resolve_borders(s: &ComputedStyle, fs: f32) -> BorderEdges {
    BorderEdges {
        top: resolve_border_side(s, "top", fs),
        right: resolve_border_side(s, "right", fs),
        bottom: resolve_border_side(s, "bottom", fs),
        left: resolve_border_side(s, "left", fs),
    }
}

fn font_families(s: &ComputedStyle) -> Vec<String> {
    s.get("font-family")
        .map(|v| {
            v.split(',')
                .map(|f| f.trim().trim_matches(['"', '\'']).to_string())
                .filter(|f| !f.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn font_weight(s: &ComputedStyle) -> u16 {
    match s.get("font-weight").unwrap_or("normal").trim() {
        "normal" => 400,
        "bold" => 700,
        n => n.parse().unwrap_or(400),
    }
}

fn line_height(s: &ComputedStyle, fs: f32) -> Option<f32> {
    let v = s.get("line-height")?.trim();
    if v.eq_ignore_ascii_case("normal") {
        return None;
    }
    match v.parse::<f32>() {
        Ok(mult) => Some(mult * fs),
        Err(_) => parse_px(v, fs),
    }
}

fn align_of(s: &ComputedStyle) -> crate::text::Align {
    use crate::text::Align;
    match s.get("text-align").unwrap_or("left").trim() {
        "right" => Align::Right,
        "center" => Align::Center,
        "justify" => Align::Justify,
        _ => Align::Left,
    }
}

fn white_space_of(s: &ComputedStyle) -> crate::text::WhiteSpace {
    use crate::text::WhiteSpace;
    match s.get("white-space").unwrap_or("normal").trim() {
        "pre" => WhiteSpace::Pre,
        "nowrap" => WhiteSpace::NoWrap,
        _ => WhiteSpace::Normal,
    }
}

fn vertical_align_of(s: &ComputedStyle) -> VAlign {
    match s.get("vertical-align").unwrap_or("baseline").trim() {
        "sub" => VAlign::Sub,
        "super" => VAlign::Super,
        "middle" => VAlign::Middle,
        "top" => VAlign::Top,
        "bottom" => VAlign::Bottom,
        _ => VAlign::Baseline,
    }
}

fn break_rule_of(s: &ComputedStyle, prop: &str) -> BreakRule {
    match s.get(prop).unwrap_or("auto").trim() {
        "avoid" => BreakRule::Avoid,
        "page" | "column" => BreakRule::Page,
        _ => BreakRule::Auto,
    }
}

fn int_prop(s: &ComputedStyle, prop: &str, default: u8) -> u8 {
    s.get(prop)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn resolve_box_metrics(s: &ComputedStyle, fs: f32, ctx: ResolveCtx) -> BoxStyle {
    BoxStyle {
        display: display_of(s),
        position: position_of(s),
        float: float_of(s),
        inset_top: length_prop(s, "top", fs, LengthPct::Auto),
        inset_right: length_prop(s, "right", fs, LengthPct::Auto),
        inset_bottom: length_prop(s, "bottom", fs, LengthPct::Auto),
        inset_left: length_prop(s, "left", fs, LengthPct::Auto),
        z_index: s.get("z-index").and_then(|v| v.trim().parse::<i32>().ok()),
        margin: resolve_edges(s, "margin", fs, ctx.cb_width),
        padding: resolve_edges(s, "padding", fs, ctx.cb_width),
        border: resolve_borders(s, fs),
        width: length_prop(s, "width", fs, LengthPct::Auto),
        height: length_prop(s, "height", fs, LengthPct::Auto),
        min_width: length_prop(s, "min-width", fs, LengthPct::Px(0.0)),
        max_width: length_prop(s, "max-width", fs, LengthPct::Auto),
        min_height: length_prop(s, "min-height", fs, LengthPct::Px(0.0)),
        max_height: length_prop(s, "max-height", fs, LengthPct::Auto),
        border_radius: border_radius_of(s, fs),
        box_sizing: box_sizing_of(s),
        font_families: font_families(s),
        font_size: fs,
        font_weight: font_weight(s),
        italic: matches!(
            s.get("font-style").map(str::trim),
            Some("italic") | Some("oblique")
        ),
        color: s.get("color").and_then(parse_color).unwrap_or(Rgba::BLACK),
        line_height: line_height(s, fs),
        text_align: align_of(s),
        white_space: white_space_of(s),
        letter_spacing: s
            .get("letter-spacing")
            .and_then(|v| parse_px(v, fs))
            .unwrap_or(0.0),
        vertical_align: vertical_align_of(s),
        break_before: break_rule_of(s, "break-before"),
        break_after: break_rule_of(s, "break-after"),
        break_inside_avoid: s.get("break-inside").map(str::trim) == Some("avoid"),
        orphans: int_prop(s, "orphans", 2),
        widows: int_prop(s, "widows", 2),
        background: background_of(s),
        box_shadow: box_shadow_of(s, fs),
        background_gradient: linear_gradient_of(s),
        transform: transform_of(s, fs),
    }
}

/// Parse a CSS 2D `transform` list into a [`RawTransform`]: the non-translate
/// functions (`scale`/`rotate`/`matrix`/`skew`) multiply into the linear part, and
/// a leading `translate*` keeps its `<length-percentage>` offsets for layout to
/// resolve against the box size. `None` for `none`/absent/no recognized function.
/// (Exact when the translate leads the list — the dominant real-world form,
/// `translate(...) scale(...)`; a translate BEHIND a rotate is approximated.)
fn transform_of(s: &ComputedStyle, fs: f32) -> Option<RawTransform> {
    let value = s.get("transform")?.trim();
    if is_none_keyword(value) {
        return None;
    }
    let mut acc = TransformAcc {
        linear: [1.0, 0.0, 0.0, 1.0], // identity 2×2 as (a,b,c,d)
        tx: LengthPct::Px(0.0),
        ty: LengthPct::Px(0.0),
    };
    let mut any = false;
    for (name, args) in transform_functions(value) {
        any |= apply_transform_fn(&mut acc, &name, &args, fs);
    }
    any.then_some(RawTransform {
        linear: acc.linear,
        tx: acc.tx,
        ty: acc.ty,
    })
}

/// Accumulated 2D-transform state built by folding over the function list: the
/// 2×2 linear part (as `(a, b, c, d)`) and the pending translate offsets.
struct TransformAcc {
    linear: [f32; 4],
    tx: LengthPct,
    ty: LengthPct,
}

/// Apply one parsed transform function to `acc`, returning whether it was a
/// recognized function (so the caller knows the list contributed anything).
/// Unrecognized functions (`translate3d` z, `perspective`, `matrix3d`, …) are
/// ignored and return `false`.
fn apply_transform_fn(acc: &mut TransformAcc, name: &str, args: &[&str], fs: f32) -> bool {
    let n = |i: usize| args.get(i).and_then(|a| a.trim().parse::<f32>().ok());
    match name {
        "translatex" => acc.tx = len_or_zero(args.first(), fs),
        "translatey" => acc.ty = len_or_zero(args.first(), fs),
        "translate" | "translate3d" => {
            acc.tx = len_or_zero(args.first(), fs);
            acc.ty = len_or_zero(args.get(1), fs);
        }
        "scale" | "scale3d" => {
            let sx = n(0).unwrap_or(1.0);
            let sy = n(1).unwrap_or(sx);
            acc.linear = mul_linear(acc.linear, [sx, 0.0, 0.0, sy]);
        }
        "scalex" => acc.linear = mul_linear(acc.linear, [n(0).unwrap_or(1.0), 0.0, 0.0, 1.0]),
        "scaley" => acc.linear = mul_linear(acc.linear, [1.0, 0.0, 0.0, n(0).unwrap_or(1.0)]),
        "rotate" | "rotatez" => {
            let (sn, c) = parse_angle(args.first())
                .unwrap_or(0.0)
                .to_radians()
                .sin_cos();
            acc.linear = mul_linear(acc.linear, [c, sn, -sn, c]);
        }
        "skewx" => {
            let t = parse_angle(args.first()).unwrap_or(0.0).to_radians().tan();
            acc.linear = mul_linear(acc.linear, [1.0, 0.0, t, 1.0]);
        }
        "skewy" => {
            let t = parse_angle(args.first()).unwrap_or(0.0).to_radians().tan();
            acc.linear = mul_linear(acc.linear, [1.0, t, 0.0, 1.0]);
        }
        "matrix" if args.len() == 6 => {
            let m: Vec<f32> = args.iter().filter_map(|a| a.trim().parse().ok()).collect();
            if m.len() == 6 {
                acc.linear = mul_linear(acc.linear, [m[0], m[1], m[2], m[3]]);
                acc.tx = LengthPct::Px(m[4]);
                acc.ty = LengthPct::Px(m[5]);
            }
        }
        _ => return false,
    }
    true
}

/// A CSS keyword value that produces no effect: empty or the `none` keyword.
fn is_none_keyword(value: &str) -> bool {
    value.is_empty() || value.eq_ignore_ascii_case("none")
}

/// Multiply two 2×2 linear parts stored as `(a, b, c, d)` = `[[a, c], [b, d]]`
/// (CSS `matrix` order): the result applies `l1` after `l2` (`l1 · l2`).
fn mul_linear(l1: [f32; 4], l2: [f32; 4]) -> [f32; 4] {
    let [a1, b1, c1, d1] = l1;
    let [a2, b2, c2, d2] = l2;
    [
        a1 * a2 + c1 * b2,
        b1 * a2 + d1 * b2,
        a1 * c2 + c1 * d2,
        b1 * c2 + d1 * d2,
    ]
}

/// A `<length-percentage>` transform arg → `LengthPct` (px resolved via `fs`),
/// defaulting to `0` for an absent/`0`/unparsable arg.
fn len_or_zero(tok: Option<&&str>, fs: f32) -> LengthPct {
    match tok.map(|t| t.trim()) {
        Some(t) if !t.is_empty() && t != "0" => {
            parse_length_pct(t, fs).unwrap_or(LengthPct::Px(0.0))
        }
        _ => LengthPct::Px(0.0),
    }
}

/// A CSS `<angle>` → degrees (`deg`/`rad`/`grad`/`turn`, bare number = deg).
fn parse_angle(tok: Option<&&str>) -> Option<f32> {
    let t = tok?.trim();
    for (unit, factor) in [("deg", 1.0), ("grad", 0.9), ("turn", 360.0)] {
        if let Some(num) = t.strip_suffix(unit) {
            return num.trim().parse::<f32>().ok().map(|v| v * factor);
        }
    }
    if let Some(num) = t.strip_suffix("rad") {
        return num.trim().parse::<f32>().ok().map(f32::to_degrees);
    }
    t.parse::<f32>().ok() // bare number = degrees
}

/// Split a `transform` value into `(lowercased-function-name, args)` pairs.
fn transform_functions(value: &str) -> Vec<(String, Vec<&str>)> {
    let mut out = Vec::new();
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        i = skip_separators(bytes, i);
        // function name = run up to '('
        let name_start = i;
        i = scan_until(bytes, i, b'(');
        if i >= bytes.len() {
            break;
        }
        let name = value[name_start..i].trim().to_ascii_lowercase();
        i += 1; // past '('
        let args_start = i;
        i = scan_until(bytes, i, b')');
        let args: Vec<&str> = value[args_start..i.min(bytes.len())]
            .split(',')
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .collect();
        if !name.is_empty() {
            out.push((name, args));
        }
        i += 1; // past ')'
    }
    out
}

/// Advance past any run of ASCII whitespace and commas, returning the new index.
fn skip_separators(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
        i += 1;
    }
    i
}

/// Advance until `bytes[i] == stop` (or the end), returning that index.
fn scan_until(bytes: &[u8], mut i: usize, stop: u8) -> usize {
    while i < bytes.len() && bytes[i] != stop {
        i += 1;
    }
    i
}

/// A `linear-gradient(...)` from `background-image` or the `background` shorthand,
/// or `None` (no gradient / `radial-`/`conic-` unsupported / unparsable). Syntax:
/// `linear-gradient( [<angle> | to <side/corner>]? , <color> [<pos>]? , ...)`. The
/// direction defaults to `to bottom` (180°); stops without a position spread evenly.
fn linear_gradient_of(s: &ComputedStyle) -> Option<super::fragment::LinearGradient> {
    let inner = linear_gradient_inner(s)?;
    let mut parts = top_level_comma_split(inner);
    let first = parts.next()?;
    // Leading angle / `to <side>`, else `first` is actually the first colour stop.
    let (angle, first_stop) = match parse_gradient_direction(first) {
        Some(a) => (a, None),
        None => (180.0, Some(first)),
    };
    let stop_tokens: Vec<&str> = first_stop.into_iter().chain(parts).collect();
    let stops = gradient_stops(&stop_tokens)?;
    (stops.len() >= 2).then_some(super::fragment::LinearGradient {
        angle_deg: angle,
        stops,
    })
}

/// The balanced text inside the first `linear-gradient(...)` of a box's
/// `background-image` (else `background` shorthand), or `None` if there is none.
fn linear_gradient_inner(s: &ComputedStyle) -> Option<&str> {
    let raw = s
        .get("background-image")
        .or_else(|| s.get("background"))
        .map(str::trim)?;
    let inner = raw
        .find("linear-gradient(")
        .map(|i| &raw[i + "linear-gradient(".len()..])?;
    // Take up to the matching close paren (the gradient's own parens are balanced).
    balanced_paren_slice(inner)
}

/// The substring up to the paren that closes the one just opened before `s`
/// (depth starts at 1). Handles `rgb(...)`/nested funcs inside a gradient.
fn balanced_paren_slice(s: &str) -> Option<&str> {
    let mut depth = 1i32;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// A gradient direction token → CSS angle in degrees (0 = to top, clockwise), or
/// `None` if the token isn't a direction (so the caller treats it as a colour stop).
fn parse_gradient_direction(tok: &str) -> Option<f32> {
    let t = tok.trim();
    if let Some(deg) = t.strip_suffix("deg") {
        return deg.trim().parse::<f32>().ok();
    }
    let sides = t
        .strip_prefix("to ")?
        .split_whitespace()
        .collect::<Vec<_>>();
    let angle = match sides.as_slice() {
        ["top"] => 0.0,
        ["right"] => 90.0,
        ["bottom"] => 180.0,
        ["left"] => 270.0,
        // Corners: 45° diagonals (exact CSS corner math is aspect-dependent; this is
        // the common visual approximation).
        ["top", "right"] | ["right", "top"] => 45.0,
        ["bottom", "right"] | ["right", "bottom"] => 135.0,
        ["bottom", "left"] | ["left", "bottom"] => 225.0,
        ["top", "left"] | ["left", "top"] => 315.0,
        _ => return None,
    };
    Some(angle)
}

/// Resolve colour-stop tokens (`<color> [<pos%>]?`) to positioned 0..1 stops.
/// Explicit `%` positions are honoured; stops without one spread evenly across the
/// remaining span (endpoints default 0 and 1). `None` if a token names no colour.
fn gradient_stops(tokens: &[&str]) -> Option<Vec<super::fragment::GradientStop>> {
    let n = tokens.len();
    let mut colors = Vec::with_capacity(n);
    let mut positions: Vec<Option<f32>> = Vec::with_capacity(n);
    for (i, tok) in tokens.iter().enumerate() {
        let (color, pos) = parse_gradient_stop(tok)?;
        colors.push(color);
        // First/last default to the endpoints when unpositioned.
        positions.push(pos.or_else(|| endpoint_default(i, n)));
    }
    fill_position_gaps(&mut positions);
    Some(
        colors
            .into_iter()
            .zip(positions)
            .map(|(color, pos)| super::fragment::GradientStop {
                color,
                pos: pos.unwrap_or(0.0),
            })
            .collect(),
    )
}

/// Parse one gradient stop token (`<color> [<pos%>]?`) into its colour and an
/// optional explicit 0..1 position. `None` if the token names no colour.
fn parse_gradient_stop(tok: &str) -> Option<(Rgba, Option<f32>)> {
    let mut parts = css_value_tokens(tok).into_iter();
    let color = parse_color(parts.next()?)?;
    let pos = parts
        .next()
        .and_then(|p| p.strip_suffix('%'))
        .and_then(|p| {
            p.trim()
                .parse::<f32>()
                .ok()
                .map(|v| (v / 100.0).clamp(0.0, 1.0))
        });
    Some((color, pos))
}

/// The default position for stop `i` of `n` when it carries no explicit one: the
/// endpoints pin to 0 and 1, interior stops stay unresolved (`None`).
fn endpoint_default(i: usize, n: usize) -> Option<f32> {
    match i {
        0 => Some(0.0),
        _ if i == n - 1 => Some(1.0),
        _ => None,
    }
}

/// Fill unpositioned interior stops by even interpolation between the nearest
/// resolved neighbours (endpoints already pinned to 0/1 by the caller).
fn fill_position_gaps(positions: &mut [Option<f32>]) {
    let n = positions.len();
    let mut i = 0;
    while i < n {
        if positions[i].is_some() {
            i += 1;
            continue;
        }
        let prev = positions[..i]
            .iter()
            .rposition(|p| p.is_some())
            .unwrap_or(0);
        let next = (i..n).find(|&k| positions[k].is_some()).unwrap_or(n - 1);
        let (p0, p1) = (
            positions[prev].unwrap_or(0.0),
            positions[next].unwrap_or(1.0),
        );
        let span = (next - prev).max(1) as f32;
        for (step, slot) in positions[prev + 1..next].iter_mut().enumerate() {
            // `step + 1` is the stop's distance (in stop count) from `prev`.
            *slot = Some(p0 + (p1 - p0) * ((step + 1) as f32 / span));
        }
        i = next;
    }
}

/// The first (topmost) `box-shadow` layer: `[inset]? <ox> <oy> <blur>? <spread>?
/// <color>?` in any color position, comma-separated layers (v1 keeps the first).
/// `None` for `none`/absent/malformed (needs at least the two offset lengths).
/// The color defaults to the box's `color` (CSS `currentColor`).
fn box_shadow_of(s: &ComputedStyle, fs: f32) -> Option<super::fragment::BoxShadow> {
    let value = s.get("box-shadow")?.trim();
    if is_none_keyword(value) {
        return None;
    }
    let first = top_level_comma_split(value).next()?;
    let parts = parse_shadow_parts(first, fs);
    if parts.lengths.len() < 2 {
        return None;
    }
    let lengths = parts.lengths;
    Some(super::fragment::BoxShadow {
        offset_x: lengths[0].round() as i32,
        offset_y: lengths[1].round() as i32,
        blur: lengths.get(2).copied().unwrap_or(0.0).max(0.0).round() as u32,
        spread: lengths.get(3).copied().unwrap_or(0.0).round() as i32,
        color: parts
            .color
            .unwrap_or_else(|| s.get("color").and_then(parse_color).unwrap_or(Rgba::BLACK)),
        inset: parts.inset,
    })
}

/// The raw pieces of one `box-shadow` layer: the `inset` flag, the offset/blur/
/// spread lengths in source order, and an explicit colour if the layer names one.
struct ShadowParts {
    inset: bool,
    lengths: Vec<f32>,
    color: Option<Rgba>,
}

/// Classify the whitespace tokens of a single `box-shadow` layer into a
/// [`ShadowParts`] (the `inset` keyword, `<length>`s, and a colour).
fn parse_shadow_parts(layer: &str, fs: f32) -> ShadowParts {
    let mut parts = ShadowParts {
        inset: false,
        lengths: Vec::new(),
        color: None,
    };
    for tok in css_value_tokens(layer) {
        classify_shadow_token(tok, fs, &mut parts);
    }
    parts
}

/// Fold one `box-shadow` token into `parts`: `inset`, a `<length>` (`0`, or any
/// `parse_px`-able value), or a colour. Anything else is ignored.
fn classify_shadow_token(tok: &str, fs: f32, parts: &mut ShadowParts) {
    if tok.eq_ignore_ascii_case("inset") {
        parts.inset = true;
    } else if tok == "0" {
        parts.lengths.push(0.0);
    } else if let Some(px) = parse_px(tok, fs) {
        parts.lengths.push(px);
    } else if let Some(c) = parse_color(tok) {
        parts.color = Some(c);
    }
}

/// Split a CSS value on top-level commas, keeping parenthesized groups
/// (`rgba(0, 0, 0, .2)`) intact. Used to separate `box-shadow` layers.
fn top_level_comma_split(value: &str) -> impl Iterator<Item = &str> {
    let bytes = value.as_bytes();
    let mut parts = Vec::new();
    let (mut start, mut depth) = (0usize, 0i32);
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => {
                parts.push(value[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(value[start..].trim());
    parts.into_iter().filter(|p| !p.is_empty())
}

/// The used background colour: the `background-color` longhand, else a colour
/// found in the `background` shorthand. Real stylesheets set backgrounds via the
/// shorthand (`background: #fff url(...) no-repeat`) far more than the longhand,
/// so ignoring it leaves most real pages painting no backgrounds at all.
fn background_of(s: &ComputedStyle) -> Option<Rgba> {
    if let Some(c) = s.get("background-color").and_then(parse_color) {
        return Some(c);
    }
    s.get("background").and_then(background_shorthand_color)
}

/// The colour token of a `background` shorthand (`url(...)` / gradients skipped),
/// or `None` if it names no colour.
fn background_shorthand_color(value: &str) -> Option<Rgba> {
    css_value_tokens(value)
        .into_iter()
        .filter(|t| !t.starts_with("url(") && !t.contains("gradient("))
        .find_map(parse_color)
}

/// Split a CSS value into top-level tokens, keeping parenthesized groups
/// (`rgb(1, 2, 3)`, `url(a b)`) whole so a functional value isn't torn apart at
/// its inner spaces/commas. Shared by the `background`/`background-image`
/// shorthand readers.
pub(crate) fn css_value_tokens(value: &str) -> Vec<&str> {
    let bytes = value.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        i = scan_value_token(bytes, i);
        tokens.push(&value[start..i]);
    }
    tokens
}

/// The index just past the value token starting at `i`: run until unnested
/// whitespace, keeping parenthesized groups (`rgb(1, 2, 3)`) whole.
fn scan_value_token(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b if b.is_ascii_whitespace() && depth == 0 => break,
            _ => {}
        }
        i += 1;
    }
    i
}

fn box_sizing_of(s: &ComputedStyle) -> BoxSizing {
    match s.get("box-sizing").map(str::trim) {
        Some("border-box") => BoxSizing::BorderBox,
        _ => BoxSizing::ContentBox,
    }
}

/// Resolve a [`ComputedStyle`] into a typed [`BoxStyle`] for layout.
pub fn resolve_box_style(s: &ComputedStyle, ctx: ResolveCtx) -> BoxStyle {
    crate::hot!("layout.resolve_box_style");
    let fs = resolve_font_size(s, ctx.parent_font_size);
    resolve_box_metrics(s, fs, ctx)
}

/// Whether `resolve_box_style(s, ctx)` is the same for every `ctx` — i.e. the box
/// uses no relative units and has an absolute, present `font-size` (so its font
/// size, and every `em` derived from it, is fixed rather than inherited from the
/// context). When true a box may resolve its style once and reuse it across the
/// measure and placement passes; otherwise it must re-resolve per call.
pub(crate) fn is_ctx_independent(s: &ComputedStyle) -> bool {
    s.has_no_relative_units() && font_size_absolute(s)
}

fn font_size_absolute(s: &ComputedStyle) -> bool {
    matches!(
        s.get("font-size").and_then(parse_raw),
        Some(RawLength::Abs(_))
    )
}

#[cfg(test)]
mod box_shadow_tests {
    use super::*;

    fn shadow(value: &str) -> Option<super::super::fragment::BoxShadow> {
        let s = ComputedStyle::from_pairs([("box-shadow", value), ("color", "#123456")]);
        box_shadow_of(&s, 16.0)
    }

    #[test]
    fn parses_offsets_blur_spread_and_color() {
        let sh = shadow("2px 4px 8px 1px rgba(0,0,0,0.5)").expect("shadow");
        assert_eq!((sh.offset_x, sh.offset_y), (2, 4));
        assert_eq!((sh.blur, sh.spread), (8, 1));
        assert_eq!((sh.color.r, sh.color.a), (0, 128)); // 0.5·255 = 127.5 → 128
        assert!(!sh.inset);
    }

    #[test]
    fn color_defaults_to_currentcolor_and_zero_is_a_valid_length() {
        // No color token → the box's `color` (#123456). Bare `0` counts as a length.
        let sh = shadow("0 0 4px").expect("shadow");
        assert_eq!((sh.offset_x, sh.offset_y, sh.blur), (0, 0, 4));
        assert_eq!((sh.color.r, sh.color.g, sh.color.b), (0x12, 0x34, 0x56));
    }

    #[test]
    fn inset_keyword_and_leading_color_both_parse() {
        let sh = shadow("inset #f00 3px 3px").expect("shadow");
        assert!(sh.inset);
        assert_eq!((sh.offset_x, sh.offset_y, sh.color.r), (3, 3, 255));
    }

    #[test]
    fn first_layer_wins_and_none_or_partial_yields_none() {
        // Comma-split: the first (topmost) layer is kept.
        assert_eq!(shadow("1px 1px red, 9px 9px blue").unwrap().offset_x, 1);
        assert!(shadow("none").is_none());
        assert!(shadow("2px").is_none()); // needs both offsets
    }
}

#[cfg(test)]
mod gradient_tests {
    use super::*;

    fn grad(value: &str) -> Option<super::super::fragment::LinearGradient> {
        linear_gradient_of(&ComputedStyle::from_pairs([("background-image", value)]))
    }

    #[test]
    fn angle_direction_and_two_stops() {
        let g = grad("linear-gradient(90deg, #ff0000, #0000ff)").expect("gradient");
        assert_eq!(g.angle_deg, 90.0);
        assert_eq!(g.stops.len(), 2);
        assert_eq!((g.stops[0].color.r, g.stops[0].pos), (255, 0.0));
        assert_eq!((g.stops[1].color.b, g.stops[1].pos), (255, 1.0));
    }

    #[test]
    fn to_side_keywords_map_to_angles_and_default_is_to_bottom() {
        assert_eq!(
            grad("linear-gradient(to right, red, blue)")
                .unwrap()
                .angle_deg,
            90.0
        );
        assert_eq!(
            grad("linear-gradient(to top, red, blue)")
                .unwrap()
                .angle_deg,
            0.0
        );
        // No direction token → defaults to 180° (to bottom); first token is a stop.
        let g = grad("linear-gradient(red, blue)").unwrap();
        assert_eq!(g.angle_deg, 180.0);
        assert_eq!(g.stops.len(), 2);
    }

    #[test]
    fn explicit_percent_stops_and_even_spread_of_the_middle() {
        // Middle stop has no position → evenly interpolated between 0 and 1 → 0.5.
        let g = grad("linear-gradient(180deg, #000, #888, #fff)").unwrap();
        assert_eq!(
            g.stops.iter().map(|s| s.pos).collect::<Vec<_>>(),
            vec![0.0, 0.5, 1.0]
        );
        // Honour an explicit % on the middle stop.
        let g = grad("linear-gradient(#000, #888 25%, #fff)").unwrap();
        assert_eq!(g.stops[1].pos, 0.25);
    }

    #[test]
    fn nested_rgb_commas_dont_split_stops_and_non_linear_is_none() {
        let g = grad("linear-gradient(to right, rgb(1, 2, 3), rgba(4, 5, 6, 0.5))").unwrap();
        assert_eq!(g.stops.len(), 2);
        assert_eq!(g.stops[1].color.a, 128);
        assert!(grad("radial-gradient(red, blue)").is_none());
        assert!(grad("#ffffff").is_none());
    }

    #[test]
    fn every_side_and_corner_direction_maps_to_its_angle() {
        let angle = |dir: &str| {
            grad(&format!("linear-gradient({dir}, red, blue)"))
                .unwrap()
                .angle_deg
        };
        assert_eq!(angle("to bottom"), 180.0);
        assert_eq!(angle("to left"), 270.0);
        assert_eq!(angle("to top right"), 45.0);
        assert_eq!(angle("to bottom right"), 135.0);
        assert_eq!(angle("to bottom left"), 225.0);
        assert_eq!(angle("to top left"), 315.0);
    }

    #[test]
    fn unknown_direction_keyword_is_not_a_direction() {
        // `to nowhere` is not a recognized direction, so it's treated as the first
        // colour stop — which names no colour, so the gradient fails to parse.
        assert!(grad("linear-gradient(to nowhere, red, blue)").is_none());
    }

    #[test]
    fn unbalanced_parens_yield_no_gradient() {
        // No closing paren -> the balanced-slice scan finds no end -> None.
        assert!(grad("linear-gradient(red, blue").is_none());
    }
}

#[cfg(test)]
mod border_longhand_tests {
    use super::*;

    #[test]
    fn border_width_longhand_overrides_shorthand_width() {
        // `border: 1px solid` gives width 1; the `border-width: 4px` longhand wins.
        let s = ComputedStyle::from_pairs([
            ("border".to_string(), "1px solid #000".to_string()),
            ("border-width".to_string(), "4px".to_string()),
        ]);
        let side = resolve_border_side(&s, "top", 16.0);
        assert_eq!(side.width, 4);
    }
}

#[cfg(test)]
mod transform_tests {
    use super::*;

    fn xf(value: &str) -> Option<RawTransform> {
        transform_of(&ComputedStyle::from_pairs([("transform", value)]), 16.0)
    }

    #[test]
    fn translate_keeps_length_percentage_offsets() {
        let t = xf("translate(20px, 30px)").expect("xf");
        assert_eq!(t.linear, [1.0, 0.0, 0.0, 1.0]); // identity linear
        assert_eq!(t.tx, LengthPct::Px(20.0));
        assert_eq!(t.ty, LengthPct::Px(30.0));
        // Single-axis + percentage (the `translate(-50%,-50%)` centring idiom).
        let t = xf("translateY(-50%)").unwrap();
        assert_eq!(t.ty, LengthPct::Pct(-50.0));
        assert_eq!(t.tx, LengthPct::Px(0.0));
    }

    #[test]
    fn rotate_builds_a_rotation_matrix() {
        let t = xf("rotate(90deg)").unwrap();
        // cos90≈0, sin90≈1 → (a,b,c,d) = (0,1,-1,0).
        assert!(t.linear[0].abs() < 1e-3 && (t.linear[1] - 1.0).abs() < 1e-3);
        assert!((t.linear[2] + 1.0).abs() < 1e-3 && t.linear[3].abs() < 1e-3);
    }

    #[test]
    fn scale_and_combined_translate_scale() {
        assert_eq!(xf("scale(2)").unwrap().linear, [2.0, 0.0, 0.0, 2.0]);
        assert_eq!(xf("scale(2, 3)").unwrap().linear, [2.0, 0.0, 0.0, 3.0]);
        // Real Nike form: leading translate then scale — split stays exact.
        let t = xf("translate3d(-50%,-50%,0) scale(10)").unwrap();
        assert_eq!(t.linear, [10.0, 0.0, 0.0, 10.0]);
        assert_eq!((t.tx, t.ty), (LengthPct::Pct(-50.0), LengthPct::Pct(-50.0)));
    }

    #[test]
    fn matrix_and_none() {
        let t = xf("matrix(1, 0, 0, 1, 12, 34)").unwrap();
        assert_eq!(t.linear, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!((t.tx, t.ty), (LengthPct::Px(12.0), LengthPct::Px(34.0)));
        assert!(xf("none").is_none());
        assert!(xf("").is_none());
    }

    #[test]
    fn skew_functions_shear_the_linear_part() {
        // skewX puts tan(angle) in the `c` slot; skewY in the `b` slot.
        let t = xf("skewX(45deg)").unwrap();
        assert!((t.linear[2] - 1.0).abs() < 1e-3, "tan45 in c: {t:?}");
        let t = xf("skewY(45deg)").unwrap();
        assert!((t.linear[1] - 1.0).abs() < 1e-3, "tan45 in b: {t:?}");
    }

    #[test]
    fn angle_units_radians_and_bare_number() {
        // `rad` is converted to degrees before rotation; cos(pi) = -1.
        let t = xf("rotate(3.14159265rad)").unwrap();
        assert!((t.linear[0] + 1.0).abs() < 1e-2, "cos(pi): {t:?}");
        // A bare number is treated as degrees.
        let t = xf("rotate(180)").unwrap();
        assert!((t.linear[0] + 1.0).abs() < 1e-2, "180 == 180deg: {t:?}");
    }

    #[test]
    fn unknown_function_contributes_nothing() {
        // An unrecognized function alone yields no transform...
        assert!(xf("perspective(500px)").is_none());
        // ...but a recognized function alongside it still parses.
        assert!(xf("rotate(10deg) perspective(1px)").is_some());
    }

    #[test]
    fn zero_translate_arg_and_trailing_token() {
        // A `0` translate arg resolves to zero length (the non-length arm).
        let t = xf("translate(0, 12px)").unwrap();
        assert_eq!(t.tx, LengthPct::Px(0.0));
        assert_eq!(t.ty, LengthPct::Px(12.0));
        // A trailing name with no `(` ends function scanning cleanly.
        let t = xf("rotate(90deg) trailing").unwrap();
        assert!(
            (t.linear[1] - 1.0).abs() < 1e-3,
            "rotate still applied: {t:?}"
        );
    }
}
